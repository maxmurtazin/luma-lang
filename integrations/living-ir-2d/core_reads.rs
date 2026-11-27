//! Resolve Core-backed literal Lengths from the registry used by ProjectStore CAS.
use super::super::{part_head, restore_part, ProjectStore, Registry};
use crate::{error, uuid, Error};
use hir_transaction::{DefinitionStore, PreparationContext};
use hir_types::Revision;
use lir2d_runtime::Command;
use lir2d_types::{
    CoreParameterPin, Digest, DocumentSnapshot, Literal, ObjectId, ParameterDef, ValueSource,
};
use living_symbolic::{Dimension, Node};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const MAX_PINS: usize = 64;

pub(super) struct CoreReads<'a> {
    registry: &'a Registry,
    context: &'a PreparationContext,
    stores: BTreeMap<String, DefinitionStore>,
}

impl<'a> CoreReads<'a> {
    pub(super) fn new(registry: &'a Registry, context: &'a PreparationContext) -> Self {
        Self {
            registry,
            context,
            stores: BTreeMap::new(),
        }
    }

    pub(super) fn snapshot(
        &mut self,
        source: &DocumentSnapshot,
        require_current: bool,
    ) -> Result<Vec<Value>, Error> {
        let linked = source
            .parameters()
            .values()
            .filter_map(|parameter| parameter.core_source().map(|pin| (parameter, pin)));
        let mut reads = Vec::new();
        for (parameter, pin) in linked {
            if reads.len() == MAX_PINS {
                return Err(error(
                    "ResourceLimit",
                    "Embedded 2D has too many Core Length bindings",
                ));
            }
            let value = parameter_length(parameter)?;
            self.verify(pin, value, require_current)?;
            reads.push(json!({"parameter_id":parameter.id,"source":pin,"value_m":value}));
        }
        Ok(reads)
    }

    pub(super) fn commands(&mut self, commands: &[Command]) -> Result<(), Error> {
        let mut count = 0;
        for item in commands {
            let (pin, value) = match item {
                Command::CreateParameter { parameter } => {
                    let Some(pin) = parameter.core_source() else {
                        continue;
                    };
                    (pin, parameter_length(parameter)?)
                }
                Command::SetLiteral {
                    core_source: Some(pin),
                    value_si: Some(value),
                    ..
                } => (pin, value.get()),
                Command::SetLiteral {
                    core_source: Some(_),
                    ..
                } => return Err(wrong_type()),
                _ => continue,
            };
            count += 1;
            if count > MAX_PINS {
                return Err(error(
                    "ResourceLimit",
                    "2D batch changes too many Core Length bindings",
                ));
            }
            self.verify(pin, value, true)?;
        }
        Ok(())
    }

    fn verify(
        &mut self,
        pin: &CoreParameterPin,
        value: f64,
        require_current: bool,
    ) -> Result<(), Error> {
        if require_current {
            let (head, _, _) = part_head(self.registry, pin.document_id.as_str())?;
            if head.native_revision != pin.native_revision.0
                || Digest::try_from(head.definition_hash).map_err(|_| Error::internal())?
                    != pin.definition_hash
            {
                return Err(stale(pin));
            }
        }
        let (source, actual) = self.resolve(
            pin.document_id.as_str(),
            pin.entity_id.as_str(),
            Some(Revision(pin.native_revision.0)),
        )?;
        if &source != pin {
            return Err(stale(pin));
        }
        if actual != value {
            return Err(error(
                "CoreLengthValueMismatch",
                "2D literal must equal the pinned Core Length in metres",
            ));
        }
        Ok(())
    }

    fn resolve(
        &mut self,
        document: &str,
        entity: &str,
        revision: Option<Revision>,
    ) -> Result<(CoreParameterPin, f64), Error> {
        let (head, part, _) = part_head(self.registry, document)?;
        let revision = revision.unwrap_or(Revision(head.native_revision));
        let definition_hash = part
            .snapshots
            .iter()
            .find(|record| record.0 == revision.0)
            .map(|record| record.1.clone())
            .ok_or_else(|| error("Conflict", "Core Length revision is no longer retained"))?;
        let store = match self.stores.entry(document.to_owned()) {
            std::collections::btree_map::Entry::Occupied(found) => found.into_mut(),
            std::collections::btree_map::Entry::Vacant(missing) => {
                let (_, store) = restore_part(part, document, head.native_revision, self.context)?;
                missing.insert(store)
            }
        };
        let snapshot = store
            .revision(revision)
            .map_err(living_hir_facade::commit_error)?
            .ok_or_else(|| error("Conflict", "Core Length revision is no longer retained"))?;
        let entity_id = uuid(entity)?;
        let node = snapshot
            .definition
            .entities
            .get(&entity_id)
            .ok_or_else(|| {
                error(
                    "MissingCoreLength",
                    "Core parameter is absent from the pinned Part",
                )
            })?;
        let Some(Node::Parameter(length)) = Node::decode(&node.payload.bytes).ok() else {
            return Err(wrong_type());
        };
        if length.dimension != Dimension::LENGTH || !length.si.is_finite() {
            return Err(wrong_type());
        }
        Ok((
            CoreParameterPin {
                document_id: ObjectId::try_from(document.to_owned())
                    .map_err(|_| Error::internal())?,
                entity_id: ObjectId::try_from(entity.to_owned()).map_err(|_| Error::internal())?,
                native_revision: lir2d_types::Revision(revision.0),
                definition_hash: Digest::try_from(definition_hash)
                    .map_err(|_| Error::internal())?,
            },
            length.si,
        ))
    }
}

impl ProjectStore {
    pub fn embedded_2d_length_source(
        &self,
        document: &str,
        entity: &str,
        context: &PreparationContext,
    ) -> Result<Value, Error> {
        let registry = self.registry()?;
        let (source, value) = CoreReads::new(&registry, context).resolve(document, entity, None)?;
        Ok(
            json!({"project_revision":registry.revision.to_string(),"source":source,"value_m":value}),
        )
    }
}

fn parameter_length(parameter: &ParameterDef) -> Result<f64, Error> {
    match &parameter.value_source {
        ValueSource::Literal(Literal::Si { value_si }) => Ok(value_si.get()),
        _ => Err(wrong_type()),
    }
}

fn wrong_type() -> Error {
    error(
        "CoreLengthTypeMismatch",
        "Expected a literal Length parameter",
    )
}

fn stale(pin: &CoreParameterPin) -> Error {
    error(
        "Conflict",
        "Core Length revision or definition hash changed",
    )
    .detail("source_document", json!(pin.document_id))
    .detail("source_entity", json!(pin.entity_id))
}
