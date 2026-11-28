//! Local owner-only CLI for the composed Core and Luma adapter.
use hir_transaction::PreparationContext;
use living_cad_viewer::{
    checkpoint::project::{
        luma_cad::LengthBinding, CreateDocument, DocumentKind, ProjectStore, Receipt,
    },
    error, Error,
};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::ExitCode,
};

fn main() -> ExitCode {
    let mut envelope = json!({"protocol":"luma.cad.core-project/1","schema_version":1});
    let args: Vec<_> = std::env::args().skip(1).collect();
    let status = match run(&args) {
        Ok(result) => {
            envelope["result"] = result;
            0
        }
        Err(problem) => {
            let status = match problem.code.as_ref() {
                "Usage" | "ProjectKey" | "InputFile" | "OutputFile" => 2,
                "Conflict" | "RequestIdReuse" | "ManagedEntityConflict" => 4,
                "Cancelled" | "ResourceLimit" => 6,
                "CheckpointAuthentication" | "CheckpointCorrupt" | "InternalError" => 5,
                _ => 3,
            };
            envelope["error"] = json!(problem);
            status
        }
    };
    println!("{envelope}");
    ExitCode::from(status)
}

fn run(args: &[String]) -> Result<Value, Error> {
    let [db, project, action, args @ ..] = args else {
        return Err(usage());
    };
    let context = PreparationContext::default();
    let key = key()?;
    let path = Path::new(db);
    if let ("init", [part, operation]) = (action.as_str(), args) {
        living_cad_viewer::uuid(part)?;
        validate_operation(operation)?;
        return ProjectStore::create(path, project, key)?
            .create_document(
                &CreateDocument {
                    request_id: operation.clone(),
                    document_id: part.clone(),
                    expected_project_revision: 0,
                    kind: DocumentKind::Part,
                    name: "Luma Part".into(),
                },
                &context,
            )
            .map(|receipt| receipt_json(&receipt));
    }
    let mut store = ProjectStore::open(path, project, key)?;
    match (action.as_str(), args) {
        ("capabilities", []) => Ok(json!({
            "profile":"core-part/1", "binding_type":"Length", "task_keys":["part_definition"],
            "limits":{"affected_recipes":64,"binding_reads":4096,"prepared_token_bytes":8*1024*1024}
        })),
        ("inspect", []) => {
            let registry = store.registry()?;
            Ok(json!({
                "project_id": registry.project_id,
                "project_revision": registry.revision.to_string(),
                "documents": registry.documents,
            }))
        }
        ("assembly-bom", [document]) => store.assembly_bom(document, None, &context),
        ("assembly-bom", [document, configuration]) => {
            store.assembly_bom(document, Some(configuration), &context)
        }
        ("preview", [mode, part, revision, bundle, destination, jobs @ ..]) => {
            let revision = revision.parse().map_err(|_| usage())?;
            let source = String::from_utf8(read(bundle, 256 * 1024)?)
                .map_err(|_| error("InvalidSourceBundle", "Expected UTF-8 Luma bundle"))?;
            let command = match mode.as_str() {
                "recipe" => false,
                "command" => true,
                _ => return Err(usage()),
            };
            let preview =
                store.prepare_luma_with_jobs(part, revision, &source, command, jobs, &context)?;
            write_preview(&preview, destination)
        }
        ("create-part", [part, revision, operation]) => {
            let revision = revision.parse().map_err(|_| usage())?;
            validate_operation(operation)?;
            store
                .create_document(
                    &CreateDocument {
                        request_id: operation.clone(),
                        document_id: part.clone(),
                        expected_project_revision: revision,
                        kind: DocumentKind::Part,
                        name: "Luma Part".into(),
                    },
                    &context,
                )
                .map(|receipt| receipt_json(&receipt))
        }
        ("bind", [part, revision, file, destination, jobs @ ..]) => {
            let revision = revision.parse().map_err(|_| usage())?;
            let bindings: Vec<LengthBinding> = serde_json::from_slice(&read(file, 64 * 1024)?)
                .map_err(|_| {
                    error(
                        "InvalidRecipeBinding",
                        "Expected a JSON array of Length bindings",
                    )
                })?;
            let preview = store.prepare_luma_bindings(part, revision, &bindings, jobs, &context)?;
            write_preview(&preview, destination)
        }
        ("recipe-info", [part]) => store.luma_recipe_info(part),
        ("detach", [part, revision, destination]) => {
            let revision = revision.parse().map_err(|_| usage())?;
            let preview = store.prepare_luma_detach(part, revision, &context)?;
            write_preview(&preview, destination)
        }
        ("apply", [token, operation]) => Ok(receipt_json(&store.apply_luma(
            &read(token, 8 * 1024 * 1024)?,
            operation,
            &context,
        )?)),
        ("receipt", [part, operation]) => Ok(receipt_json(&receipt(&store, part, operation)?)),
        ("job", [part, key]) => Ok(json!(store.luma_job(part, key)?)),
        ("work", []) => Ok(json!(store.run_luma_job()?)),
        ("job-cancel", [job]) => Ok(json!(
            store.cancel_luma_job(job.parse().map_err(|_| usage())?)?
        )),
        ("job-retry", [job, attempt]) => Ok(json!(store.retry_luma_job(
            job.parse().map_err(|_| usage())?,
            attempt.parse().map_err(|_| usage())?,
        )?)),
        ("job-definition", [job]) => store.luma_job_definition(job.parse().map_err(|_| usage())?),
        #[cfg(feature = "living-view-v2")]
        ("scene", [part, operation, scratch]) => {
            store.luma_scene(&receipt(&store, part, operation)?, Path::new(scratch))
        }
        _ => Err(usage()),
    }
}

fn write_preview(
    preview: &living_cad_viewer::checkpoint::project::luma_cad::Preview,
    destination: &str,
) -> Result<Value, Error> {
    write_token(destination, &preview.token)?;
    let mut result = json!({"project_revision":preview.project_revision,
        "candidate_changed":preview.candidate_changed,"candidate_pin":preview.candidate_pin,
        "accepted_source_digest":preview.accepted_source_digest});
    if let Some(report) = &preview.detach {
        result["detach"] = json!(report);
    }
    Ok(result)
}

fn write_token(destination: &str, bytes: &[u8]) -> Result<(), Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(destination)
        .and_then(|mut file| file.write_all(bytes).and_then(|_| file.sync_all()))
        .map_err(|_| error("OutputFile", "Could not create a new prepared-token file"))
}

fn receipt(store: &ProjectStore, part: &str, operation: &str) -> Result<Receipt, Error> {
    store
        .luma_receipt(part, operation)?
        .ok_or_else(|| error("UnknownOperation", "No accepted receipt exists"))
}

fn receipt_json(receipt: &Receipt) -> Value {
    let mut result = json!({"document_id":receipt.document_id,
        "document_revision":receipt.document_revision.to_string(),
        "project_revision":receipt.project_revision.to_string()});
    if !receipt.luma_jobs.is_empty() {
        result["jobs"] = json!(receipt.luma_jobs);
    }
    if !receipt.luma_dependent_jobs.is_empty() {
        result["dependent_jobs"] = json!(receipt.luma_dependent_jobs);
    }
    result
}

fn validate_operation(value: &str) -> Result<(), Error> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(error(
            "Usage",
            "Operation ID must contain 32 lowercase hexadecimal digits",
        ));
    }
    Ok(())
}

fn read(path: &str, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(limit as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| error("InputFile", "Could not read input file"))?;
    if bytes.len() > limit {
        return Err(Error::resource("input_bytes", limit, bytes.len()));
    }
    Ok(bytes)
}

fn key() -> Result<[u8; 32], Error> {
    let bad = || {
        error(
            "ProjectKey",
            "Set LUMA_CAD_PROJECT_KEY to the local project's 64-hex key",
        )
    };
    let value = std::env::var("LUMA_CAD_PROJECT_KEY").map_err(|_| bad())?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let mut key = [0; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| bad())?;
    }
    Ok(key)
}

fn usage() -> Error {
    error("Usage", "luma-cad-project DB PROJECT init PART OP | create-part PART REVISION OP | assembly-bom ASSEMBLY [CONFIGURATION] | preview MODE PART REVISION BUNDLE TOKEN [JOB_KEY ...] | bind PART REVISION BINDINGS TOKEN [JOB_KEY ...] | recipe-info PART | detach PART REVISION TOKEN | apply TOKEN OP | receipt PART OP | job PART KEY | work | job-cancel JOB | job-retry JOB ATTEMPT | job-definition JOB | scene PART OP SCRATCH")
}
