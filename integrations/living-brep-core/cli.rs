//! Owner-only bridge for derived artifacts from an accepted Core Part.
mod artifact;
mod drawing;
#[cfg(feature = "luma-tasks")]
mod m7;
mod profile;
#[cfg(feature = "luma-tasks")]
mod queue;

use brep_core::{
    decode_native, BrepKernel, KernelContext, StandaloneKernel, ValidatedBrep, K1_SOLID,
};
use hir_transaction::PreparationContext;
use hir_types::EntityId;
use living_cad_viewer::checkpoint::project::ProjectStore;
use living_ir_brep_preview::publication::{
    project::ProjectPublicationStore, Artifacts, PublicationError,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    process::ExitCode,
    time::{Duration, Instant},
};

const OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MEMORY_BYTES: usize = 64 * 1024 * 1024;

struct Failure(&'static str, Value);
type Result<T> = std::result::Result<T, Failure>;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mut envelope = json!({"protocol": "luma.cad.brep-project/1", "schema_version": 1});
    let code = match execute(&args) {
        Ok(result) => {
            envelope["result"] = result;
            0
        }
        Err(Failure(code, details)) => {
            envelope["error"] = json!({"code": code, "details": details});
            match code {
                "Usage" | "ProjectKey" => 2,
                "Conflict" | "RequestIdReuse" => 4,
                "ResourceLimit" | "Cancelled" => 6,
                _ => 3,
            }
        }
    };
    println!("{envelope}");
    ExitCode::from(code)
}

fn execute(args: &[String]) -> Result<Value> {
    let [database, project, derived, action, args @ ..] = args else {
        return Err(usage());
    };
    let mut project =
        ProjectStore::open(Path::new(database), project, project_key()?).map_err(core_error)?;
    let derived = Path::new(derived);
    match (action.as_str(), args) {
        #[cfg(feature = "luma-tasks")]
        (
            action @ ("jobs-submit" | "jobs-work" | "jobs-receipt" | "jobs-status" | "jobs-result"
            | "jobs-cancel" | "jobs-retry"),
            args,
        ) => queue::run(&project, derived, action, args),
        #[cfg(feature = "luma-tasks")]
        ("luma-tasks", [operation, digest, bundle]) => {
            let mut context = kernel_context();
            let (published, model) = load_brep(&project, derived, operation, &mut context)?;
            check_topology_digest(&model, digest)?;
            artifact::run_tasks(&published, &model, bundle, &mut context)
        }
        #[cfg(feature = "luma-tasks")]
        ("luma-m7", [provider, left, left_digest, right, right_digest, bundle]) => {
            let mut context = kernel_context();
            let (_, a) = load_brep(&project, derived, left, &mut context)?;
            check_topology_digest(&a, left_digest)?;
            let (_, b) = load_brep(&project, derived, right, &mut context)?;
            check_topology_digest(&b, right_digest)?;
            m7::run(provider, bundle, [(left, &a), (right, &b)], &mut context)
        }
        (action @ ("sheet-validate" | "sheet-publish"), args) => {
            drawing::run(&mut project, action, args)
        }
        ("source", [document, feature]) => {
            let source = project
                .extrusion_source(document, id(feature)?, &PreparationContext::default())
                .map_err(core_error)?;
            let (_, sketch, height) = source.input();
            let head = source.head();
            Ok(json!({"project":head.project_id, "part":head.document_id,
                "revision":head.native_revision.to_string(), "definition_hash":head.definition_hash,
                "feature":source.feature(), "sketch":sketch, "signed_height_m":height}))
        }
        ("sketch-profile", [document, feature]) => {
            let source = project
                .extrusion_source(document, id(feature)?, &PreparationContext::default())
                .map_err(core_error)?;
            profile::sketch_profile(&source)
        }
        ("publish", [document, feature, revision, digest, operation]) => {
            let revision: u64 = revision.parse().map_err(|_| usage())?;
            if !lower_hex(digest, 64) {
                return Err(usage());
            }
            let mut context = kernel_context();
            let preparation =
                PreparationContext::new(Default::default(), context.cancellation.shared_flag());
            let source = project
                .extrusion_source(document, id(feature)?, &preparation)
                .map_err(core_error)?;
            if source.head().native_revision != revision || source.head().definition_hash != *digest
            {
                return Err(Failure("Conflict", json!({"reason":"Part pin changed"})));
            }
            let operation = id(operation)?;
            let mut store =
                ProjectPublicationStore::open(derived, &project).map_err(publication_error)?;
            let outcome = store
                .run(&mut project, &source, operation, OUTPUT_BYTES, &mut context)
                .map_err(publication_error)?;
            serde_json::to_value(outcome).map_err(encoding_error)
        }
        ("latest", [document, feature]) => {
            let document = document.parse().map_err(|_| usage())?;
            let feature = id(feature)?;
            if !derived.exists() {
                return Ok(Value::Null);
            }
            let store =
                ProjectPublicationStore::open(derived, &project).map_err(publication_error)?;
            let latest = store
                .latest(&mut project, document, feature)
                .map_err(publication_error)?;
            Ok(latest.map_or(
                Value::Null,
                |(receipt, current)| json!({"receipt":receipt,"current":current}),
            ))
        }
        ("measure", [operation]) => {
            let mut context = kernel_context();
            let (artifacts, value) = load_brep(&project, derived, operation, &mut context)?;
            let profile = value.profile();
            let report: Value =
                serde_json::from_slice(&artifacts.report).map_err(encoding_error)?;
            let provenance: Value =
                serde_json::from_slice(&artifacts.provenance).map_err(encoding_error)?;
            Ok(json!({
                "receipt":artifacts.receipt, "volume_m3":artifact::interval(value.volume_m3()),
                "outer_loop":profile.outer_loop, "hole_loops":profile.hole_loops,
                "report":report, "provenance":provenance,
            }))
        }
        ("view", [operation]) => {
            let mut context = kernel_context();
            let (published, value) = load_brep(&project, derived, operation, &mut context)?;
            artifact::view(&published, &value, &mut context)
        }
        ("distance", [operation, digest, scope, coordinates @ ..]) => {
            let mut context = kernel_context();
            let (_, value) = load_brep(&project, derived, operation, &mut context)?;
            check_topology_digest(&value, digest)?;
            artifact::distance(&value, scope, coordinates, &mut context)
        }
        ("section", [operation, digest, plane]) => {
            let mut context = kernel_context();
            let (_, value) = load_brep(&project, derived, operation, &mut context)?;
            check_topology_digest(&value, digest)?;
            artifact::section(&value, plane, &mut context)
        }
        _ => Err(usage()),
    }
}

fn load_brep(
    project: &ProjectStore,
    derived: &Path,
    operation: &str,
    context: &mut KernelContext,
) -> Result<(Artifacts, ValidatedBrep)> {
    let operation = id(operation)?;
    if !derived.exists() {
        return Err(Failure("MissingPublication", Value::Null));
    }
    let store = ProjectPublicationStore::open(derived, project).map_err(publication_error)?;
    let artifacts = store
        .recover(operation, OUTPUT_BYTES)
        .map_err(publication_error)?
        .ok_or(Failure("MissingPublication", Value::Null))?;
    let shape = decode_native(&artifacts.geometry, artifacts.receipt.geometry, context)
        .map_err(|code| Failure("InvalidBrep", json!(format!("{code:?}"))))?;
    let value = artifact::outcome(StandaloneKernel.accept(shape, K1_SOLID, context))?;
    Ok((artifacts, value))
}

fn check_topology_digest(value: &ValidatedBrep, digest: &str) -> Result<()> {
    if artifact::hex(&value.blob_digest()) != digest {
        return Err(Failure("StaleTopologyRef", Value::Null));
    }
    Ok(())
}

fn kernel_context() -> KernelContext {
    let mut context = KernelContext::default();
    context.budget.max_memory_bytes = MEMORY_BYTES;
    context.budget.deadline = Some(Instant::now() + Duration::from_secs(30));
    context
}

fn publication_error(problem: PublicationError) -> Failure {
    match problem {
        PublicationError::Project(error) => core_error(error),
        PublicationError::Superseded => Failure("Conflict", Value::Null),
        PublicationError::RequestIdReuse => Failure("RequestIdReuse", Value::Null),
        PublicationError::LimitExceeded => Failure("ResourceLimit", Value::Null),
        PublicationError::Cancelled => Failure("Cancelled", Value::Null),
        PublicationError::InvalidStore => Failure("InvalidDerivedStore", Value::Null),
        other => Failure("BrepPublication", json!(format!("{other:?}"))),
    }
}

fn core_error(problem: living_cad_viewer::Error) -> Failure {
    let code = match problem.code.as_ref() {
        "ResourceLimit" => "ResourceLimit",
        "Cancelled" => "Cancelled",
        "Conflict" => "Conflict",
        "RequestIdReuse" => "RequestIdReuse",
        "UnknownJob" => "UnknownJob",
        "JobPending" => "JobPending",
        "UnsupportedGeometry" => "UnsupportedGeometry",
        _ => "CoreProject",
    };
    Failure(code, json!(problem))
}

fn encoding_error(problem: serde_json::Error) -> Failure {
    Failure("Encoding", json!(problem.to_string()))
}

fn lower_hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn project_key() -> Result<[u8; 32]> {
    let failure = || {
        Failure(
            "ProjectKey",
            json!("Set LUMA_CAD_PROJECT_KEY to the owner key"),
        )
    };
    let value = std::env::var("LUMA_CAD_PROJECT_KEY").map_err(|_| failure())?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(failure());
    }
    let mut key = [0; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| failure())?;
    }
    Ok(key)
}

fn id(value: &str) -> Result<EntityId> {
    living_cad_viewer::uuid(value).map_err(|_| usage())
}

fn usage() -> Failure {
    Failure("Usage", json!("luma-cad-brep DB PROJECT DERIVED source PART FEATURE | sketch-profile PART FEATURE | publish PART FEATURE REVISION DIGEST OPERATION_UUID | latest PART FEATURE | measure OPERATION_UUID | view OPERATION_UUID | distance OPERATION_UUID GEOMETRY_SHA solid|face:N X Y Z | section OPERATION_UUID GEOMETRY_SHA FRAME_JSON | luma-tasks OPERATION_UUID GEOMETRY_SHA BUNDLE | jobs-submit REQUEST_UUID OPERATION_UUID GEOMETRY_SHA BUNDLE | jobs-work | jobs-receipt REQUEST_UUID | jobs-status JOB | jobs-result JOB | jobs-cancel JOB | jobs-retry JOB ATTEMPT | luma-m7 M7_PROVIDER LEFT_OP LEFT_SHA RIGHT_OP RIGHT_SHA BUNDLE | sheet-validate PART LIR2D INPUT SHEET review|manufacturing [--font FONT] | sheet-publish PART LIR2D INPUT SHEET review|manufacturing OUTPUT [--expect-source REV HASH DRAWING_SHA] [--font FONT]"))
}
