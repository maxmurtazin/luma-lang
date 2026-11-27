//! Compute admitted engineering tasks against an immutable native publication.
use crate::{
    artifact, check_topology_digest, core_error, encoding_error, kernel_context, load_brep, usage,
    Failure, Result,
};
use living_cad_viewer::checkpoint::project::{
    luma_cad::brep_jobs::{BrepJobInput, BrepSource},
    ProjectStore,
};
use living_ir_brep_preview::publication::Artifacts;
use serde_json::Value;
use std::path::Path;

const PROVIDER: &str = include_str!("brep-job-pin.txt");

pub(crate) fn run(
    project: &ProjectStore,
    derived: &Path,
    action: &str,
    args: &[String],
) -> Result<Value> {
    match (action, args) {
        ("jobs-submit", [request, operation, digest, bundle]) => {
            let mut context = kernel_context();
            let (published, model) = load_brep(project, derived, operation, &mut context)?;
            check_topology_digest(&model, digest)?;
            let source = source(&published)?;
            let (source_digest, tasks) = artifact::prepare_tasks(bundle)?;
            let input = BrepJobInput {
                source,
                source_digest,
                tasks,
            };
            let delivery = project
                .admit_brep_job(request, PROVIDER, input)
                .map_err(core_error)?;
            serde_json::to_value(delivery).map_err(encoding_error)
        }
        ("jobs-work", []) => {
            let receipt = project
                .run_brep_job(PROVIDER, |input| {
                    let mut context = kernel_context();
                    let (published, model) =
                        load_brep(project, derived, &input.source.publication, &mut context)
                            .map_err(|problem| problem.0)?;
                    if source(&published).map_err(|problem| problem.0)? != input.source {
                        return Err("PinnedPublicationChanged");
                    }
                    check_topology_digest(&model, &input.source.geometry_sha256)
                        .map_err(|problem| problem.0)?;
                    artifact::evaluate_tasks(
                        &published,
                        &model,
                        &input.source_digest,
                        &input.tasks,
                        &mut context,
                    )
                    .map_err(|problem| problem.0)
                })
                .map_err(core_error)?;
            serde_json::to_value(receipt).map_err(encoding_error)
        }
        ("jobs-receipt", [request]) => {
            let delivery = project.brep_job_receipt(request).map_err(core_error)?;
            serde_json::to_value(delivery).map_err(encoding_error)
        }
        ("jobs-status", [job]) => {
            let receipt = project.brep_job_status(job_id(job)?).map_err(core_error)?;
            serde_json::to_value(receipt).map_err(encoding_error)
        }
        ("jobs-result", [job]) => project.brep_job_result(job_id(job)?).map_err(core_error),
        ("jobs-cancel", [job]) => {
            let job = job_id(job)?;
            project.brep_job_status(job).map_err(core_error)?;
            let receipt = project.cancel_luma_job(job).map_err(core_error)?;
            serde_json::to_value(receipt).map_err(encoding_error)
        }
        ("jobs-retry", [job, attempt]) => {
            let job = job_id(job)?;
            let attempt = attempt
                .parse::<i64>()
                .ok()
                .filter(|number| *number >= 0)
                .ok_or_else(usage)?;
            project.brep_job_status(job).map_err(core_error)?;
            let receipt = project.retry_luma_job(job, attempt).map_err(core_error)?;
            serde_json::to_value(receipt).map_err(encoding_error)
        }
        _ => Err(usage()),
    }
}

fn job_id(value: &str) -> Result<i64> {
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(usage)
}

fn source(artifact: &Artifacts) -> Result<BrepSource> {
    let receipt = &artifact.receipt;
    let provenance: Value = serde_json::from_slice(&artifact.provenance).map_err(encoding_error)?;
    let job = &provenance["job"];
    let revision = job["revision"]
        .as_u64()
        .ok_or(Failure("InvalidProvenance", Value::Null))?;
    let definition_digest = job["project_definition_hash"]
        .as_str()
        .ok_or(Failure("InvalidProvenance", Value::Null))?
        .into();
    Ok(BrepSource {
        publication: receipt.commit_id.to_string(),
        geometry_sha256: artifact::hex(&receipt.geometry),
        provenance_sha256: artifact::hex(&receipt.provenance),
        document: receipt.output.document.to_string(),
        revision,
        definition_digest,
    })
}
