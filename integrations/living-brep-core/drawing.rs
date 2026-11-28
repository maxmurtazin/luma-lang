//! Render a standalone 2D sheet from one accepted Core Part and fence publication.
use crate::{core_error, encoding_error, id, usage, Failure, Result};
use hir_transaction::PreparationContext;
use living_cad_viewer::checkpoint::project::{PartHead, ProjectStore};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::Path,
    process::Command,
};

const MAX_SHEET_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn run(project: &mut ProjectStore, action: &str, args: &[String]) -> Result<Value> {
    let [part, renderer, input, sheet, policy, tail @ ..] = args else {
        return Err(usage());
    };
    let (destination, options) = match (action, tail) {
        ("sheet-validate", _) => (None, tail),
        ("sheet-publish", [output, options @ ..]) => {
            (Some(destination_path(Path::new(output))?), options)
        }
        _ => return Err(usage()),
    };
    let (expectation, options) = match options {
        [flag, revision, definition, drawing, options @ ..] if flag == "--expect-source" => {
            (Some((revision, definition, drawing)), options)
        }
        _ => (None, options),
    };
    let font = match options {
        [] => None,
        [flag, font] if flag == "--font" => Some(font),
        _ => return Err(usage()),
    };
    if !matches!(policy.as_str(), "review" | "manufacturing") {
        return Err(usage());
    }
    id(part)?;
    id(sheet)?;
    let renderer = Path::new(renderer).canonicalize().map_err(sheet_io)?;
    let builder = &mut tempfile::Builder::new();
    builder.prefix(".luma-sheet-");
    let (format, staging) = if let Some(path) = &destination {
        let format = match path.extension().and_then(|extension| extension.to_str()) {
            Some("svg") => "svg",
            Some("pdf") => "pdf",
            _ => return Err(usage()),
        };
        let staging = builder.tempdir_in(path.parent().ok_or_else(usage)?);
        (format, staging)
    } else {
        ("svg", builder.tempdir())
    };
    let staging = staging.map_err(sheet_io)?;
    let drawing = staging.path().join("drawing.json");
    let mut normalize = renderer_command(&renderer, "normalize", Path::new(input));
    invoke(normalize.arg("--output").arg(&drawing))?;
    let drawing_digest = file_sha256(&drawing)?;
    let mut inspect = renderer_command(&renderer, "sheet.core-sources", &drawing);
    let references = invoke(inspect.args(["--sheet", sheet]))?;
    let references = references
        .as_array()
        .filter(|values| (1..=8).contains(&values.len()))
        .ok_or(Failure(
            "UnsupportedDrawingSource",
            json!("Expected one to eight Core plate sources"),
        ))?;
    let features = references
        .iter()
        .map(|reference| {
            if reference["document_id"].as_str() != Some(part) {
                return Err(Failure(
                    "UnsupportedDrawingSource",
                    json!("All Core views must reference the selected Part"),
                ));
            }
            id(reference["feature_id"].as_str().ok_or_else(usage)?)
        })
        .collect::<Result<Vec<_>>>()?;
    let (head, plates) = project
        .drawing_plate_sources(part, &features, &PreparationContext::default())
        .map_err(core_error)?;
    let revision = head.native_revision.to_string();
    if let Some((expected_revision, expected_definition, expected_drawing)) = expectation {
        if expected_revision != &revision
            || expected_definition != &head.definition_hash
            || expected_drawing != &drawing_digest
        {
            return Err(Failure(
                "Conflict",
                json!("The drawing or Core Part differs from the validated source"),
            ));
        }
    }
    let head_file = staging.path().join("core-head.json");
    let geometry = staging.path().join("core-geometry.json");
    write_json(
        &head_file,
        &json!({
            "document_id":part, "native_revision":revision, "definition_hash":head.definition_hash,
            "parameters":[],
        }),
    )?;
    write_json(
        &geometry,
        &serde_json::to_value(&plates).map_err(encoding_error)?,
    )?;
    let staged_file = staging.path().join(format!("sheet.{format}"));
    let mut export = renderer_command(&renderer, &format!("sheet.export-{format}"), &drawing);
    export
        .args(["--sheet", sheet, "--policy", policy])
        .arg("--core-head")
        .arg(head_file)
        .arg("--core-geometry")
        .arg(geometry);
    if let Some(font) = font {
        export.arg("--font").arg(font);
    }
    let mut report = invoke(export.arg("--output").arg(&staged_file))?;
    let expected: BTreeSet<_> = plates
        .iter()
        .map(|plate| plate.feature_id.as_str())
        .collect();
    check_report(&report, &head, &revision, &expected, format, policy, sheet)?;
    if let Some(issue) = report["release_issues"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|issue| {
            matches!(
                issue["message_key"].as_str(),
                Some("projection.anchor_missing" | "projection.core_source_ambiguous")
            )
        })
    {
        return Err(Failure("UnresolvedDrawingReference", issue.clone()));
    }
    verify_staged_file(&staged_file, &report)?;
    project
        .with_part_head(part, |current| {
            if current != head {
                return Err(Failure(
                    "Conflict",
                    json!("Part changed during sheet rendering"),
                ));
            }
            if let Some(destination) = &destination {
                fs::hard_link(&staged_file, destination).map_err(sheet_io)?;
            }
            Ok(())
        })
        .map_err(core_error)??;
    let published = destination.is_some();
    report["published"] = json!(published);
    report["core_source_status"] = json!("verified_current");
    report["revision_policy"] = json!(if published {
        "require_current_at_publish"
    } else {
        "require_current_at_validation"
    });
    report["core_project_id"] = json!(head.project_id);
    report["validated_source"] = json!({
        "native_revision": revision,
        "definition_hash": head.definition_hash,
        "drawing_sha256": drawing_digest
    });
    Ok(report)
}

fn destination_path(path: &Path) -> Result<std::path::PathBuf> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(parent
        .canonicalize()
        .map_err(sheet_io)?
        .join(path.file_name().ok_or_else(usage)?))
}

fn renderer_command(executable: &Path, action: &str, path: &Path) -> Command {
    let mut command = Command::new(executable);
    // The standalone renderer needs the extracted manifest, never the owner key.
    command
        .env_remove("LUMA_CAD_PROJECT_KEY")
        .arg(action)
        .arg(path);
    command
}

fn invoke(command: &mut Command) -> Result<Value> {
    let result = command.output().map_err(sheet_io)?;
    if !result.status.success() {
        let diagnostic = serde_json::from_slice(&result.stderr)
            .unwrap_or_else(|_| json!(String::from_utf8_lossy(&result.stderr)));
        return Err(Failure("DrawingRenderer", diagnostic));
    }
    serde_json::from_slice(&result.stdout).map_err(encoding_error)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec(value).map_err(encoding_error)?).map_err(sheet_io)
}

fn check_report(
    report: &Value,
    head: &PartHead,
    revision: &str,
    expected: &BTreeSet<&str>,
    format: &str,
    policy: &str,
    sheet: &str,
) -> Result<()> {
    let invalid = || {
        Failure(
            "InvalidDrawingReport",
            json!("Renderer did not verify the pinned Core views"),
        )
    };
    if report["core_source_status"] != "verified_manifest"
        || report["format"] != format
        || report["release_policy"] != policy
        || report["sheet_id"] != sheet
    {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    for view in report["views"].as_array().ok_or_else(invalid)? {
        let projection = &view["projection"];
        if projection["source_definition_hash"].is_null() {
            continue;
        }
        if projection["source_definition_hash"] != head.definition_hash
            || projection["source_revision"] != revision
            || projection["source_document_id"] != head.document_id
            || projection["provider"] != "core_analytic_plate_v1"
        {
            return Err(invalid());
        }
        seen.insert(projection["source_solid_id"].as_str().ok_or_else(invalid)?);
    }
    if &seen != expected {
        return Err(invalid());
    }
    Ok(())
}

fn verify_staged_file(path: &Path, report: &Value) -> Result<()> {
    if report["artifact_body_sha256"] != file_sha256(path)? {
        return Err(Failure("InvalidDrawingArtifact", Value::Null));
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path).map_err(sheet_io)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SHEET_BYTES {
        return Err(Failure("InvalidDrawingArtifact", Value::Null));
    }
    let file = File::open(path).map_err(sheet_io)?;
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut file.take(MAX_SHEET_BYTES + 1), &mut hasher).map_err(sheet_io)?;
    if size != metadata.len() {
        return Err(Failure("InvalidDrawingArtifact", Value::Null));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sheet_io(problem: std::io::Error) -> Failure {
    let code = if problem.kind() == std::io::ErrorKind::AlreadyExists {
        "DrawingOutputExists"
    } else {
        "DrawingIo"
    };
    Failure(code, json!(problem.to_string()))
}
