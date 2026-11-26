use luma_host_api::{Error, Type, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_AFFECTED_RECIPES: usize = 64;
const MAX_READ_REFERENCES: usize = 4096;

pub type RecipeOutputs = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingRevision {
    FollowCurrent,
    ExactPinned(u64),
}

#[derive(Debug)]
pub struct RecipeBinding {
    pub producer: String,
    pub output: String,
    pub revision: BindingRevision,
    pub expected_type: Type,
}

#[derive(Debug)]
pub struct AdmittedRecipe {
    pub revision: u64,
    pub bindings: BTreeMap<String, RecipeBinding>,
    pub outputs: RecipeOutputs,
}

/// One authoritative project snapshot and its retained immutable revisions.
#[derive(Debug, Default)]
pub struct AdmittedRecipes {
    pub project_revision: u64,
    pub current: BTreeMap<String, AdmittedRecipe>,
    pub history: BTreeMap<(String, u64), RecipeOutputs>,
}

#[derive(Debug)]
pub struct PreparedRecipe<Change> {
    pub change: Change,
    pub outputs: RecipeOutputs,
}

/// Apply the ordered changes together after comparing the project revision,
/// recipe heads and retained pins with the captured admission state.
#[derive(Debug)]
pub struct PreparedRecipes<Change> {
    pub project_revision: u64,
    pub heads: BTreeMap<String, u64>,
    pub pins: BTreeSet<(String, u64)>,
    pub changes: Vec<(String, PreparedRecipe<Change>)>,
}

#[derive(Debug)]
pub enum ClosureError<E> {
    Graph(Error),
    Recipe { id: String, error: E },
}

impl<E> From<Error> for ClosureError<E> {
    fn from(error: Error) -> Self {
        Self::Graph(error)
    }
}

impl AdmittedRecipes {
    /// The evaluator must stage changes without external writes. Evaluation
    /// sees candidate outputs for affected follow-current producers and
    /// admitted outputs for every other read.
    pub fn prepare<Change, E>(
        &self,
        edited: &BTreeSet<String>,
        mut evaluate: impl FnMut(&str, &BTreeMap<String, &Value>) -> Result<PreparedRecipe<Change>, E>,
    ) -> Result<PreparedRecipes<Change>, ClosureError<E>> {
        let affected = self.affected(edited)?;
        let mut batch = PreparedRecipes {
            project_revision: self.project_revision,
            heads: BTreeMap::new(),
            pins: BTreeSet::new(),
            changes: Vec::with_capacity(affected.len()),
        };
        let mut prerequisites = BTreeMap::new();
        let mut consumers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut read_count = 0;
        for &id in &affected {
            let recipe = self.head(id)?;
            batch.heads.insert(id.into(), recipe.revision);
            let mut dependencies = BTreeSet::new();
            for binding in recipe.bindings.values() {
                read_count += 1;
                if read_count > MAX_READ_REFERENCES {
                    return Err(
                        Error::new("RecipeReadLimit", "too many recipe binding reads").into(),
                    );
                }
                let admitted_outputs = match binding.revision {
                    BindingRevision::FollowCurrent => {
                        let producer = self.head(&binding.producer)?;
                        batch
                            .heads
                            .insert(binding.producer.clone(), producer.revision);
                        if affected.contains(binding.producer.as_str()) {
                            dependencies.insert(binding.producer.as_str());
                        }
                        &producer.outputs
                    }
                    BindingRevision::ExactPinned(revision) => {
                        batch.pins.insert((binding.producer.clone(), revision));
                        self.pinned(&binding.producer, revision)?
                    }
                };
                output(binding, admitted_outputs)?;
            }
            for producer in &dependencies {
                consumers.entry(producer).or_default().push(id);
            }
            prerequisites.insert(id, dependencies.len());
        }

        let order = evaluation_order(prerequisites, consumers)?;
        let mut prepared = BTreeMap::<&str, PreparedRecipe<Change>>::new();
        for &id in &order {
            let mut inputs = BTreeMap::new();
            for (name, binding) in &self.current[id].bindings {
                let outputs = match binding.revision {
                    BindingRevision::FollowCurrent => match prepared.get(binding.producer.as_str())
                    {
                        Some(candidate) => &candidate.outputs,
                        None => &self.current[&binding.producer].outputs,
                    },
                    BindingRevision::ExactPinned(revision) => {
                        self.pinned(&binding.producer, revision)?
                    }
                };
                inputs.insert(name.clone(), output(binding, outputs)?);
            }
            let result = evaluate(id, &inputs).map_err(|error| ClosureError::Recipe {
                id: id.into(),
                error,
            })?;
            prepared.insert(id, result);
        }
        for id in order {
            batch
                .changes
                .push((id.into(), prepared.remove(id).expect("evaluated recipe")));
        }
        Ok(batch)
    }

    fn affected<'a>(&'a self, edited: &'a BTreeSet<String>) -> Result<BTreeSet<&'a str>, Error> {
        let mut consumers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (id, recipe) in &self.current {
            for binding in recipe.bindings.values() {
                if binding.revision == BindingRevision::FollowCurrent {
                    consumers.entry(&binding.producer).or_default().insert(id);
                }
            }
        }

        let mut pending: BTreeSet<&str> = edited.iter().map(String::as_str).collect();
        let mut affected = BTreeSet::new();
        while let Some(id) = pending.pop_first() {
            if affected.insert(id) {
                if affected.len() > MAX_AFFECTED_RECIPES {
                    return Err(Error::new(
                        "AffectedRecipeLimit",
                        "too many affected recipes",
                    ));
                }
                if let Some(next) = consumers.get(id) {
                    pending.extend(next.difference(&affected));
                }
            }
        }
        Ok(affected)
    }

    fn head(&self, id: &str) -> Result<&AdmittedRecipe, Error> {
        self.current
            .get(id)
            .ok_or_else(|| Error::new("MissingRecipe", format!("missing recipe {id}")))
    }

    fn pinned(&self, id: &str, revision: u64) -> Result<&RecipeOutputs, Error> {
        if let Some(head) = self
            .current
            .get(id)
            .filter(|head| head.revision == revision)
        {
            return Ok(&head.outputs);
        }
        self.history.get(&(id.into(), revision)).ok_or_else(|| {
            Error::new(
                "MissingRecipeRevision",
                format!("missing pinned revision {revision} of {id}"),
            )
        })
    }
}

fn evaluation_order<'a>(
    mut prerequisites: BTreeMap<&'a str, usize>,
    consumers: BTreeMap<&'a str, Vec<&'a str>>,
) -> Result<Vec<&'a str>, Error> {
    let mut ready: BTreeSet<_> = prerequisites
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    let mut order = Vec::with_capacity(prerequisites.len());
    while let Some(id) = ready.pop_first() {
        order.push(id);
        for &consumer in consumers.get(id).into_iter().flatten() {
            let count = prerequisites
                .get_mut(consumer)
                .expect("registered consumer");
            *count -= 1;
            if *count == 0 {
                ready.insert(consumer);
            }
        }
    }
    if order.len() != prerequisites.len() {
        return Err(Error::new(
            "RecipeDependencyCycle",
            "affected follow-current graph contains a cycle",
        ));
    }
    Ok(order)
}

fn output<'a>(binding: &RecipeBinding, outputs: &'a RecipeOutputs) -> Result<&'a Value, Error> {
    let value = outputs.get(&binding.output).ok_or_else(|| {
        Error::new(
            "MissingRecipeOutput",
            format!("{} has no output {}", binding.producer, binding.output),
        )
    })?;
    if value.value_type().as_ref() != Some(&binding.expected_type) {
        return Err(Error::new(
            "RecipeOutputTypeMismatch",
            format!(
                "{}.{} has an incompatible type",
                binding.producer, binding.output
            ),
        ));
    }
    Ok(value)
}
