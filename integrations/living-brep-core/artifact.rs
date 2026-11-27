//! Engineering queries and View projection of authenticated native artifacts.
use crate::{Failure, Result, OUTPUT_BYTES};
use brep_core::{
    AccuracyMode, AccuracyRequest, BrepKernel, CheckStatus, CoordinateSpace, DistanceMode, Frame3,
    Interval, KernelContext, KernelOutcome, MeshRequest, MeshTopologyPolicy, QueryData,
    QueryEntity, QueryRequest, QueryResult, StandaloneKernel, ValidatedBrep, SCHEMA_VERSION,
};
use living_ir_brep_preview::publication::Artifacts;
use serde_json::{json, Value};

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn view(
    artifact: &Artifacts,
    model: &ValidatedBrep,
    context: &mut KernelContext,
) -> Result<Value> {
    let receipt = &artifact.receipt;
    let provenance: Value =
        serde_json::from_slice(&artifact.provenance).map_err(crate::encoding_error)?;
    let revision = provenance
        .pointer("/job/revision")
        .and_then(Value::as_u64)
        .ok_or(Failure("InvalidProvenance", Value::Null))?;
    let definition = provenance
        .pointer("/job/project_definition_hash")
        .and_then(Value::as_str)
        .ok_or(Failure("InvalidProvenance", Value::Null))?;
    let request = MeshRequest {
        schema_version: SCHEMA_VERSION,
        geometry_blob_digest: receipt.geometry,
        linear_deflection_m: 1e-5,
        angular_limit_rad: 0.2,
        topology_policy: MeshTopologyPolicy::PreserveBoundaryTopology,
        max_triangles: 100_000,
        max_bytes: OUTPUT_BYTES as u64,
    };
    let mesh = outcome(StandaloneKernel.tessellate(model, request, context))?;
    if mesh.source_blob_digest != receipt.geometry
        || mesh.report.mesh_check_status != CheckStatus::Pass
        || mesh.source_face_ordinals.len() != mesh.triangle_indices.len()
    {
        return Err(Failure("InvalidTessellation", Value::Null));
    }
    Ok(json!({
        "schema":"living-view.brep-publication/1.0",
        "source":{
            "provider_revision":include_str!("brep-pin.txt").trim(),
            "project_id":receipt.output.branch.to_string(),
            "document_id":receipt.output.document.to_string(),
            "feature_id":receipt.output.output.feature_id.to_string(),
            "job_id":receipt.commit_id.to_string(),
            "source_revision":revision.to_string(),
            "generation":receipt.generation.to_string(),
            "definition_hash":definition,
            "geometry_sha256":hex(&receipt.geometry),
            "provenance_sha256":hex(&receipt.provenance),
            "source_volume_m3":interval(model.volume_m3()),
            "tessellation":{
                "policy_version":mesh.report.policy_version,
                "requested_deflection_m":mesh.report.requested_deflection_m,
                "deviation_upper_bound_m":mesh.report.deviation_upper_bound_m,
                "angular_upper_bound_rad":mesh.report.angular_upper_bound_rad,
            },
        },
        "positions_m":mesh.positions_m,"triangles":mesh.triangle_indices,
        "face_ordinals":mesh.source_face_ordinals,
    }))
}

pub(crate) fn distance(
    model: &ValidatedBrep,
    scope: &str,
    coordinates: &[String],
    context: &mut KernelContext,
) -> Result<Value> {
    let entity = if scope == "solid" {
        QueryEntity::Solid(model.solid_id())
    } else {
        let ordinal: usize = scope
            .strip_prefix("face:")
            .and_then(|value| value.parse().ok())
            .ok_or_else(crate::usage)?;
        let source = model
            .source_faces()
            .iter()
            .find(|source| source.face.ordinal() == ordinal)
            .ok_or(Failure("UnknownTopologyRef", Value::Null))?;
        QueryEntity::Face(source.face)
    };
    let [x, y, z] = coordinates else {
        return Err(crate::usage());
    };
    let point_m = [number(x)?, number(y)?, number(z)?];
    let result = query(
        model,
        entity,
        QueryData::Distance {
            point_m,
            mode: DistanceMode::UnsignedBoundary,
        },
        context,
    )?;
    let QueryResult::Distance(value) = result else {
        return Err(Failure("InvalidQueryResult", Value::Null));
    };
    Ok(json!({
        "kind":"unsigned_boundary_distance","coordinate_space":"body_local",
        "point_m":point_m,"target":scope,"distance_m":interval(value.unsigned_m),
        "geometry_sha256":hex(&model.blob_digest()),
        "multiple_minima_possible":value.may_have_multiple_minima,
    }))
}

pub(crate) fn section(
    model: &ValidatedBrep,
    plane: &str,
    context: &mut KernelContext,
) -> Result<Value> {
    let frame: Frame3 = serde_json::from_str(plane).map_err(crate::encoding_error)?;
    let result = query(
        model,
        QueryEntity::Solid(model.solid_id()),
        QueryData::ParallelSection { plane_frame: frame },
        context,
    )?;
    let QueryResult::ParallelSection(value) = result else {
        return Err(Failure("InvalidQueryResult", Value::Null));
    };
    Ok(json!({
        "kind":format!("{:?}",value.kind),"coordinate_space":"body_local",
        "geometry_sha256":hex(&model.blob_digest()),"profile":value.profile,
        "origin_m":value.origin_m.map(interval),
        "source_face":value.source_face.map(|id|id.ordinal()),
    }))
}

fn query(
    model: &ValidatedBrep,
    entity_scope: QueryEntity,
    query_data: QueryData,
    context: &mut KernelContext,
) -> Result<QueryResult> {
    outcome(StandaloneKernel.query(
        model,
        QueryRequest {
            schema_version: SCHEMA_VERSION,
            entity_scope,
            coordinate_space: CoordinateSpace::BodyLocal,
            query_data,
            accuracy_request: AccuracyRequest {
                mode: AccuracyMode::BoundsRequired,
                absolute_error_m: 1e-8,
                relative_error: None,
                max_refinements: 32,
            },
        },
        context,
    ))
}

pub(crate) fn interval(value: Interval) -> [f64; 2] {
    [value.lower(), value.upper()]
}

pub(crate) fn outcome<T>(result: KernelOutcome<T>) -> Result<T> {
    match result {
        KernelOutcome::Success { value, .. } => Ok(value),
        KernelOutcome::Cancelled(_) => Err(Failure("Cancelled", Value::Null)),
        KernelOutcome::LimitExceeded(_) => Err(Failure("ResourceLimit", Value::Null)),
        KernelOutcome::Rejected(diagnostic) => Err(Failure(
            "UnsupportedBrepOperation",
            json!({
                "code":format!("{:?}",diagnostic.code),"rule":diagnostic.rule_id,"stage":diagnostic.stage
            }),
        )),
        KernelOutcome::Unresolved(diagnostic) => Err(Failure(
            "BrepUnresolved",
            json!({
                "code":format!("{:?}",diagnostic.code),"rule":diagnostic.rule_id,"stage":diagnostic.stage
            }),
        )),
    }
}

fn number(text: &str) -> Result<f64> {
    text.parse()
        .ok()
        .filter(|value: &f64| value.is_finite())
        .ok_or_else(crate::usage)
}

#[cfg(feature = "luma-tasks")]
pub(crate) fn run_tasks(
    published: &Artifacts,
    model: &ValidatedBrep,
    source_bundle: &str,
    context: &mut KernelContext,
) -> Result<Value> {
    use luma::cad_engine::{BrepTask, EngineeringQuery};
    use std::{fs::File, io::Read};

    let mut bytes = Vec::new();
    File::open(source_bundle)
        .and_then(|file| file.take(256 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|problem| Failure("SourceInputError", json!(problem.to_string())))?;
    if bytes.len() > 256 * 1024 {
        return Err(Failure(
            "ResourceLimit",
            json!("Luma bundle exceeds 256 KiB"),
        ));
    }
    let bundle: luma::host::WireSourceBundle =
        serde_json::from_slice(&bytes).map_err(crate::encoding_error)?;
    let bundle = bundle.admit().map_err(|problem| {
        Failure(
            "InvalidSourceBundle",
            json!(problem
                .diagnostics
                .iter()
                .map(|(file, diagnostic)| luma::host::diagnostic_json(file, diagnostic))
                .collect::<Vec<_>>()),
        )
    })?;
    let tasks = luma::cad::prepare_brep_tasks(&bundle, 100_000)
        .map_err(|problem| Failure("BrepTaskPlanRejected", json!(format!("{problem:?}"))))?;
    let mut reports = Vec::with_capacity(tasks.len());
    for task in tasks {
        let (operation, result) = match task {
            BrepTask::Mesh => ("cad.mesh", view(published, model, context)?),
            BrepTask::Section { z_m } => {
                let frame = json!({"origin_m":[0.0,0.0,z_m], "x_axis":[1,0,0],
                                   "y_axis":[0,1,0], "z_axis":[0,0,1]});
                (
                    "cad.section_task",
                    section(model, &frame.to_string(), context)?,
                )
            }
            BrepTask::Query(EngineeringQuery::Volume) => (
                "cad.query_task",
                json!({"kind":"volume", "volume_m3":interval(model.volume_m3())}),
            ),
            BrepTask::Query(EngineeringQuery::BoundaryDistance { point_m }) => (
                "cad.query_task",
                distance(
                    model,
                    "solid",
                    &point_m.map(|value| value.to_string()),
                    context,
                )?,
            ),
        };
        reports.push(json!({"operation":operation,"result":result}));
    }
    Ok(json!({
        "source_digest":bundle.source_digest(),
        "geometry_sha256":hex(&model.blob_digest()),
        "results":reports
    }))
}
