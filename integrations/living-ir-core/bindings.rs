//! Project-local Length bindings between admitted recipe instances.
use super::*;
use luma::{
    cad_engine::{
        AdmittedRecipe, AdmittedRecipes, BindingRevision, ClosureError, ParamValue, RecipeBinding,
        RecipeOutputs,
    },
    host_api::{Type, Value as HostValue},
};

const LENGTH: &str = "Quantity<Length>";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LengthBinding {
    pub target: String,
    pub producer: String,
    pub output: String,
    /// Omit for follow_current. Otherwise use the producer's accepted recipe revision.
    pub revision: Option<u64>,
}

fn output_map(entries: impl IntoIterator<Item = (String, ParamValue)>) -> RecipeOutputs {
    entries
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                HostValue::Opaque {
                    type_name: LENGTH.into(),
                    payload: json!(value),
                },
            )
        })
        .collect()
}

pub(super) fn published(parameters: &BTreeMap<String, Parameter>) -> BTreeMap<String, ParamValue> {
    parameters
        .iter()
        .filter_map(|(key, parameter)| {
            let value = parameter.effective();
            (parameter.declaration.exposed && matches!(value, ParamValue::Length(_)))
                .then(|| (key.clone(), value))
        })
        .collect()
}

impl RecipeRecord {
    pub(super) fn parameters(&self) -> Result<BTreeMap<String, Parameter>, Error> {
        serde_json::from_str(&self.parameter_json).map_err(json_error)
    }
}

pub(super) fn graph(
    registry: &Registry,
    primary: &str,
    edited: Option<&[LengthBinding]>,
) -> Result<AdmittedRecipes, Error> {
    let mut graph = AdmittedRecipes {
        project_revision: registry.revision,
        ..Default::default()
    };
    for (document, recipe) in &registry.luma_recipes {
        let bindings = if recipe.detached {
            &[]
        } else if document == primary {
            edited.unwrap_or(&recipe.bindings)
        } else {
            &recipe.bindings
        };
        if bindings.len() > 128 {
            return Err(error("ResourceLimit", "Too many bindings on one recipe"));
        }
        let mut by_target = BTreeMap::new();
        for binding in bindings {
            uuid(&binding.producer)?;
            let revision = match binding.revision {
                Some(revision) => BindingRevision::ExactPinned(revision),
                None => {
                    if registry
                        .luma_recipes
                        .get(&binding.producer)
                        .is_some_and(|recipe| recipe.detached)
                    {
                        return Err(error("DetachedRecipe", "Follow-current source is detached"));
                    }
                    BindingRevision::FollowCurrent
                }
            };
            if by_target
                .insert(
                    binding.target.clone(),
                    RecipeBinding {
                        producer: binding.producer.clone(),
                        output: binding.output.clone(),
                        revision,
                        expected_type: Type::Opaque(LENGTH.into()),
                    },
                )
                .is_some()
            {
                return Err(error(
                    "DuplicateRecipeBinding",
                    "Parameter may have one input binding",
                ));
            }
        }
        graph.current.insert(
            document.clone(),
            AdmittedRecipe {
                revision: recipe.accepted_revision,
                bindings: by_target,
                outputs: output_map(published(&recipe.parameters()?)),
            },
        );
        for (&rev, values) in &recipe.historical_outputs {
            graph.history.insert(
                (document.clone(), rev),
                output_map(
                    serde_json::from_str::<BTreeMap<String, ParamValue>>(values)
                        .map_err(json_error)?,
                ),
            );
        }
    }
    graph
        .current
        .entry(primary.into())
        .or_insert_with(|| AdmittedRecipe {
            revision: 0,
            bindings: BTreeMap::new(),
            outputs: BTreeMap::new(),
        });
    Ok(graph)
}

pub(super) fn prepared_outputs(record: &RecipeRecord) -> Result<RecipeOutputs, Error> {
    Ok(output_map(published(&record.parameters()?)))
}

pub(super) fn input_values(
    inputs: &BTreeMap<String, &HostValue>,
) -> Result<BTreeMap<String, ParamValue>, Error> {
    inputs
        .iter()
        .map(|(key, value)| {
            let HostValue::Opaque { payload, .. } = value else {
                return Err(Error::internal());
            };
            let value: ParamValue = serde_json::from_value(payload.clone()).map_err(json_error)?;
            if !matches!(value, ParamValue::Length(_)) {
                return Err(error(
                    "RecipeOutputTypeMismatch",
                    "Expected a Length binding",
                ));
            }
            Ok((key.clone(), value))
        })
        .collect()
}

pub(super) fn closure_error(problem: ClosureError<Error>) -> Error {
    match problem {
        ClosureError::Graph(problem) => luma_error(problem),
        ClosureError::Recipe { id, error } => error.detail("recipe_document", json!(id)),
    }
}

pub(super) fn reject_followers(registry: &Registry, document: &str) -> Result<(), Error> {
    if registry.luma_recipes.values().any(|record| {
        !record.detached
            && record
                .bindings
                .iter()
                .any(|binding| binding.producer == document && binding.revision.is_none())
    }) {
        return Err(error(
            "ReferencedEntityRemoval",
            "A managed recipe follows this recipe",
        ));
    }
    Ok(())
}
