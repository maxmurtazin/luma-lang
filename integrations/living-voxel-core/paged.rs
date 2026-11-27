//! Paged storage for historical Core mask publications. Living Voxel owns
//! the object format, directory lease, snapshot CAS and quarantine operation.
use super::{blocks, io_error, provider, read, usage, Result};
use living_cad_viewer::checkpoint::project::ProjectStore;
use living_voxel_pilot::{PublicationStore, Receipt, State};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

pub(super) fn execute(
    project: &ProjectStore,
    database: &Path,
    provider: &Path,
    directory: &Path,
    args: &[String],
) -> Result<Value> {
    let publications = PublicationStore::open(database, provider).map_err(blocks::error)?;
    let temp = tempfile::tempdir().map_err(io_error)?;
    let (operation, request) = match args {
        [action, job, limits] if action == "pack" => {
            let (receipt, bytes) = blocks::recover(&publications, project, job)?;
            let field = provider::decode(provider, &bytes, &receipt.binding.artifact_sha256)?;
            let input = temp.path().join("field.json");
            let bytes = serde_json::to_vec(&field).map_err(io_error)?;
            fs::write(&input, &bytes).map_err(io_error)?;
            (
                "living.voxel.storage.pack",
                json!({
                    "input":input,
                    "expected_source_sha256":format!("{:x}", Sha256::digest(&bytes)),
                    "store":directory,
                    "fanout":16,
                    "limits":load(limits, 4096)?,
                    "publication":{"name":snapshot_name(&receipt),"expected":null}
                }),
            )
        }
        [action, job, channel, indices, limits] if action == "query" => {
            let receipt = publications.status(project, job).map_err(blocks::error)?;
            if receipt.state != State::Published {
                return Err(json!({"code":"JobPending"}));
            }
            (
                "living.voxel.storage.stream",
                json!({
                    "store":directory,"snapshot":snapshot_name(&receipt),"channel":channel,
                    "selection":{"kind":"Points","indices":load(indices, 128 * 1024)?},
                    "limits":load(limits, 4096)?,"max_points":4096
                }),
            )
        }
        [action, limits, expected @ ..] if action == "gc" && expected.len() <= 1 => {
            let action = match expected {
                [digest] => json!({"kind":"Quarantine","expected_plan_sha256":digest}),
                _ => json!({"kind":"Plan"}),
            };
            (
                "living.voxel.storage.gc",
                json!({
                    "store":directory,"action":action,"max_entries":65_536,"limits":load(limits, 4096)?
                }),
            )
        }
        _ => return Err(usage()),
    };
    provider::operate(provider, operation, &request)
}

fn snapshot_name(receipt: &Receipt) -> String {
    let mut hash = Sha256::new();
    hash.update(b"luma.cad.voxel.paged/1\0");
    hash.update(receipt.project_id.as_bytes());
    hash.update([0]);
    hash.update(receipt.job_id.as_bytes());
    hash.update(receipt.binding.artifact_sha256);
    format!("{:x}", hash.finalize())
}

fn load(path: &str, limit: u64) -> Result<Value> {
    serde_json::from_slice(&read(Path::new(path), limit)?)
        .map_err(|_| json!({"code":"InvalidRequest","message":"Invalid JSON input"}))
}
