use crate::parameters::validate_key;
use luma_host_api::Error;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum OwnedKey {
    Entity(String),
    Output(String),
}

impl OwnedKey {
    fn name(&self) -> &str {
        match self {
            Self::Entity(name) | Self::Output(name) => name,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Definition<T> {
    pub kind: String,
    pub value: T,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Provenance {
    pub recipe: String,
    pub accepted_revision: u64,
    pub source_digest: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Ownership<T> {
    Managed(Definition<T>),
    Tombstone,
    Detached(Definition<T>),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OwnedRecord<T> {
    pub id: String,
    pub kind: String,
    pub origin: Provenance,
    pub state: Ownership<T>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reconciliation {
    Create,
    Update,
    Reuse,
    Remove,
}

#[derive(Debug)]
pub struct ReconciledRegion<T> {
    pub region: OwnedRegion<T>,
    pub actions: BTreeMap<OwnedKey, Reconciliation>,
}

#[derive(Debug)]
pub struct DetachedRegion<T> {
    pub region: OwnedRegion<T>,
    pub former_owner: Provenance,
    pub released_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OwnedRegion<T> {
    provenance: Provenance,
    detached: bool,
    records: BTreeMap<OwnedKey, OwnedRecord<T>>,
}

impl<T: Clone + PartialEq> OwnedRegion<T> {
    /// Rehydrate one instance from the authoritative registry, including
    /// historical keys and final definitions of detached entities.
    pub fn new(
        provenance: Provenance,
        detached: bool,
        records: BTreeMap<OwnedKey, OwnedRecord<T>>,
    ) -> Result<Self, Error> {
        let mut identities = BTreeSet::new();
        for (key, record) in &records {
            validate_key(key.name())?;
            let state_matches = match &record.state {
                Ownership::Managed(definition) => !detached && definition.kind == record.kind,
                Ownership::Detached(definition) => detached && definition.kind == record.kind,
                Ownership::Tombstone => true,
            };
            if !state_matches
                || record.origin.recipe != provenance.recipe
                || record.origin.accepted_revision > provenance.accepted_revision
                || record.kind.is_empty()
                || record.id.is_empty()
                || !identities.insert(&record.id)
            {
                return Err(Error::new("InvalidOwnedRegistry", key.name()));
            }
        }
        Ok(Self {
            provenance,
            detached,
            records,
        })
    }

    pub fn records(&self) -> &BTreeMap<OwnedKey, OwnedRecord<T>> {
        &self.records
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    pub fn is_detached(&self) -> bool {
        self.detached
    }

    /// Plan from a complete successful recipe evaluation. The authority
    /// supplies every inbound reference from outside the replaced region,
    /// plus a side-effect-free reservation of a fresh identity for each create.
    pub fn reconcile(
        &self,
        provenance: Provenance,
        desired: impl IntoIterator<Item = (OwnedKey, Definition<T>)>,
        inbound: &BTreeMap<String, BTreeSet<String>>,
        mut reserve_id: impl FnMut(&OwnedKey) -> Result<String, Error>,
    ) -> Result<ReconciledRegion<T>, Error> {
        if self.detached {
            return Err(Error::new("DetachedRecipe", &self.provenance.recipe));
        }
        if provenance.recipe != self.provenance.recipe
            || provenance.accepted_revision < self.provenance.accepted_revision
            || (provenance.accepted_revision == self.provenance.accepted_revision
                && provenance != self.provenance)
        {
            return Err(Error::new("RecipeRevisionConflict", provenance.recipe));
        }

        let mut desired_by_key = BTreeMap::new();
        for (key, definition) in desired {
            validate_key(key.name())?;
            if definition.kind.is_empty() {
                return Err(Error::new("InvalidOwnedDefinition", key.name()));
            }
            if desired_by_key.contains_key(&key) {
                return Err(Error::new("DuplicateStableKey", key.name()));
            }
            desired_by_key.insert(key, definition);
        }

        let mut actions = BTreeMap::new();
        for (key, record) in &self.records {
            let action = match (&record.state, desired_by_key.get(key)) {
                (Ownership::Managed(previous), Some(next)) => {
                    if previous.kind != next.kind {
                        return Err(Error::new("OwnedKindMigrationRequired", key.name()));
                    }
                    if previous == next {
                        Reconciliation::Reuse
                    } else {
                        Reconciliation::Update
                    }
                }
                (Ownership::Managed(_), None) => {
                    if inbound
                        .get(&record.id)
                        .is_some_and(|links| !links.is_empty())
                    {
                        return Err(Error::new("ReferencedEntityRemoval", &record.id));
                    }
                    Reconciliation::Remove
                }
                (Ownership::Tombstone, Some(_)) => {
                    return Err(Error::new("TombstonedStableKey", key.name()))
                }
                _ => continue,
            };
            actions.insert(key.clone(), action);
        }

        if provenance == self.provenance
            && (actions
                .values()
                .any(|action| *action != Reconciliation::Reuse)
                || desired_by_key
                    .keys()
                    .any(|key| !self.records.contains_key(key)))
        {
            return Err(Error::new("RecipeRevisionConflict", provenance.recipe));
        }

        let mut records = self.records.clone();
        for (key, action) in &actions {
            if *action == Reconciliation::Remove {
                records.get_mut(key).expect("admitted owned key").state = Ownership::Tombstone;
            }
        }

        let mut identities: BTreeSet<_> =
            records.values().map(|record| record.id.clone()).collect();
        for (key, definition) in desired_by_key {
            if let Some(record) = records.get_mut(&key) {
                if actions[&key] == Reconciliation::Update {
                    record.state = Ownership::Managed(definition);
                }
            } else {
                let id = reserve_id(&key)?;
                if id.is_empty() || !identities.insert(id.clone()) {
                    return Err(Error::new("OwnedIdentityConflict", id));
                }
                actions.insert(key.clone(), Reconciliation::Create);
                records.insert(
                    key,
                    OwnedRecord {
                        id,
                        kind: definition.kind.clone(),
                        origin: provenance.clone(),
                        state: Ownership::Managed(definition),
                    },
                );
            }
        }

        Ok(ReconciledRegion {
            region: Self {
                provenance,
                detached: false,
                records,
            },
            actions,
        })
    }

    /// Freeze the complete region. The caller retains the captured definitions
    /// and identities when releasing them from managed editing restrictions.
    pub fn detach(
        &self,
        mut freeze: impl FnMut(&OwnedKey, &Definition<T>) -> Result<T, Error>,
    ) -> Result<DetachedRegion<T>, Error> {
        if self.detached {
            return Err(Error::new("DetachedRecipe", &self.provenance.recipe));
        }
        let mut region = self.clone();
        let mut released_ids = Vec::new();
        for (key, record) in &mut region.records {
            if let Ownership::Managed(definition) = &record.state {
                released_ids.push(record.id.clone());
                record.state = Ownership::Detached(Definition {
                    kind: definition.kind.clone(),
                    value: freeze(key, definition)?,
                });
            }
        }
        region.detached = true;
        Ok(DetachedRegion {
            region,
            former_owner: self.provenance.clone(),
            released_ids,
        })
    }
}
