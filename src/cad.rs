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

/// Prepare the native plate/through-hole profile from a recipe snapshot.
pub fn prepare_geometry(
    bundle: &host::SourceBundle,
    inherited: &ParameterSnapshot,
    expression_budget: usize,
) -> Result<luma_cad_engine::RecipeGeometryPlan, PreparationError> {
    let session = luma_cad_engine::GeometryHost::new(inherited.clone())
        .map_err(PreparationError::Parameters)?;
    let execution = host::run_with_host(bundle, host::Mode::Recipe, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    let digest = bundle.source_digest();
    session
        .finish(&execution.outputs, digest)
        .map_err(PreparationError::Parameters)
}

/// Re-evaluate the accepted source once against the complete override batch.
pub fn prepare_geometry_edits<'a>(
    accepted_recipe: &host::SourceBundle,
    inherited: &ParameterSnapshot,
    edits: impl IntoIterator<Item = (&'a str, Option<ParamValue>)>,
    expression_budget: usize,
) -> Result<luma_cad_engine::RecipeGeometryPlan, PreparationError> {
    let edited = inherited
        .with_overrides(edits)
        .map_err(PreparationError::Parameters)?;
    prepare_geometry(accepted_recipe, &edited, expression_budget)
}

/// Execute a command against one admitted Part and return its isolated overlay.
pub fn prepare_command(
    bundle: &host::SourceBundle,
    part: String,
    inherited: &ParameterSnapshot,
    expression_budget: usize,
) -> Result<ParameterSnapshot, PreparationError> {
    let session = luma_cad_engine::CommandHost::new(part, inherited.clone())
        .map_err(PreparationError::Parameters)?;
    host::run_with_host(bundle, host::Mode::Command, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    session.finish().map_err(PreparationError::Parameters)
}

/// Prepare typed sketch commands for the registered Living IR 2D host.
pub fn prepare_sketch(
    bundle: &host::SourceBundle,
    expression_budget: usize,
) -> Result<luma_cad_engine::SketchPlan, PreparationError> {
    let session = luma_cad_engine::SketchHost::new().map_err(PreparationError::Parameters)?;
    host::run_with_host(bundle, host::Mode::Command, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    session.finish().map_err(PreparationError::Parameters)
}

/// Declare engineering work for the publication supplied by the native caller.
pub fn prepare_brep_tasks(
    bundle: &host::SourceBundle,
    expression_budget: usize,
) -> Result<Vec<luma_cad_engine::BrepTask>, PreparationError> {
    let session = luma_cad_engine::BrepTaskHost::new().map_err(PreparationError::Parameters)?;
    host::run_with_host(bundle, host::Mode::Command, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    session.finish().map_err(PreparationError::Parameters)
}

/// Plan one M7 operation against two captured native publications.
pub fn prepare_planar_boolean(
    bundle: &host::SourceBundle,
    expression_budget: usize,
) -> Result<luma_cad_engine::PlanarPlan, PreparationError> {
    let session = luma_cad_engine::PlanarHost::new().map_err(PreparationError::Parameters)?;
    host::run_with_host(bundle, host::Mode::Command, &session, expression_budget)
        .map_err(PreparationError::Execution)?;
    session.finish().map_err(PreparationError::Parameters)
}
