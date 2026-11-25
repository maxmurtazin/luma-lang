//! Read-only CAD recipe preparation.
use crate::{host, runtime};
use luma_cad_engine::{ParamValue, ParameterHost, ParameterPlan, ParameterSnapshot};

#[derive(Debug)]
pub enum PreparationError {
    Execution(runtime::Failure),
    Parameters(luma_host_api::Error),
}

/// Run recipe-local declarations and produce a parameter plan. A failed run
/// drops its entire transient host, so partial declarations cannot escape.
pub fn prepare_parameters(
    bundle: &host::SourceBundle,
    inherited: &ParameterSnapshot,
    expression_budget: usize,
) -> Result<ParameterPlan, PreparationError> {
    let session = ParameterHost::new(inherited.clone()).map_err(PreparationError::Parameters)?;
    let execution = host::run_with_host(bundle, host::Mode::Recipe, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    session
        .finish(&execution.outputs)
        .map_err(PreparationError::Parameters)
}

/// Prepare ordered override edits and rerun the admitted recipe once against the
/// resulting snapshot. Both the edits and the rerun must succeed to return a plan.
pub fn prepare_parameter_edits<'a>(
    accepted_recipe: &host::SourceBundle,
    inherited: &ParameterSnapshot,
    edits: impl IntoIterator<Item = (&'a str, Option<ParamValue>)>,
    expression_budget: usize,
) -> Result<ParameterPlan, PreparationError> {
    let edited = inherited
        .with_overrides(edits)
        .map_err(PreparationError::Parameters)?;
    prepare_parameters(accepted_recipe, &edited, expression_budget)
}
