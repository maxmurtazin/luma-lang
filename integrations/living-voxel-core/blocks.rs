//! Project-bound resident Bool edits through the pinned Core publication pilot.
use super::{core_error, encode, io_error, publish, read, usage, Result};
use living_cad_viewer::checkpoint::project::{PartHead, ProjectStore};
use living_voxel_pilot::{
    BlockEdit, BlockPatch, Cancellation, Error, PublicationStore, ReadScope, Receipt, SlotUpdate,
    State,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_LVX: u64 = 16 * 1024 * 1024;

type Coord = [i32; 3];

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Edit {
    Replace {
        channel: String,
        edits: Vec<BlockEdit>,
    },
    CopyFace {
        channel: String,
        from: Coord,
        to: Coord,
    },
}

pub(super) fn execute(
    project: &mut ProjectStore,
    database: &Path,
    provider: &Path,
    args: &[String],
) -> Result<Value> {
    let mut store = PublicationStore::open(database, provider).map_err(error)?;
    match args {
        [action, job, part, revision, hash, slot, generation, input, digest]
            if action == "seed" =>
        {
            let source = PartHead {
                project_id: project.project_id().into(),
                document_id: part.clone(),
                native_revision: revision.parse().map_err(|_| usage())?,
                definition_hash: hash.clone(),
            };
            let mut expected = [0; 32];
            hex::decode_to_slice(digest, &mut expected).map_err(|_| usage())?;
            let bytes = read(Path::new(input), MAX_LVX)?;
            let field = super::provider::decode(provider, &bytes, &expected)?;
            if !field["channels"]
                .as_array()
                .is_some_and(|channels| channels.iter().any(is_mask))
            {
                return Err(invalid("Seed requires a resident Bool mask channel"));
            }
            let update = SlotUpdate {
                name: slot.clone(),
                expected_generation: generation.parse().map_err(|_| usage())?,
            };
            store
                .admit_update(project, job, &source, expected, update)
                .map_err(error)?;
            publication(store.publish(project, job, &bytes).map_err(error)?)
        }
        [action, job, base, input] if action == "patch" => {
            let base_receipt = store.status(project, base).map_err(error)?;
            let input = read(Path::new(input), 1024 * 1024)?;
            let edit = serde_json::from_slice(&input).map_err(|_| invalid("Invalid mask patch"))?;
            let (mut patch, copied) = match edit {
                Edit::Replace { channel, edits } => (
                    BlockPatch {
                        channel,
                        scope: ReadScope::Pointwise,
                        edits,
                    },
                    None,
                ),
                Edit::CopyFace { channel, from, to } => {
                    let distance: u64 = (0..3)
                        .map(|axis| u64::from(from[axis].abs_diff(to[axis])))
                        .sum();
                    if distance != 1 {
                        return Err(invalid("Copy requires adjacent block coordinates"));
                    }
                    let (_, bytes) = recover(&store, project, base)?;
                    let field = super::provider::decode(
                        provider,
                        &bytes,
                        &base_receipt.binding.artifact_sha256,
                    )?;
                    let blocks = field["channels"]
                        .as_array()
                        .and_then(|channels| {
                            channels
                                .iter()
                                .find(|entry| entry["id"] == channel && is_mask(entry))
                        })
                        .and_then(|entry| entry["blocks"].as_array())
                        .ok_or_else(|| invalid("Copy requires a resident Bool mask channel"))?;
                    let coord = json!(from);
                    let data = blocks
                        .iter()
                        .find(|block| block["coord"] == coord)
                        .map(|block| block["data"].clone());
                    (
                        BlockPatch {
                            channel,
                            scope: ReadScope::FaceHalo,
                            edits: vec![BlockEdit { coord: to, data }],
                        },
                        Some(from),
                    )
                }
            };
            patch.edits.sort_by_key(|edit| edit.coord);
            let command: [u8; 32] = Sha256::digest(
                serde_json::to_vec(&(&patch, Option::<&str>::None)).map_err(io_error)?,
            )
            .into();
            let reads_copy_source = |certificate: &living_voxel_pilot::PatchCertificate| {
                copied.is_none_or(|coord| certificate.reads.iter().any(|read| read.coord == coord))
            };
            if let Some(receipt) = replay(&store, project, job, base, |certificate| {
                certificate.command_sha256 == command
                    && certificate.undo_of.is_none()
                    && reads_copy_source(certificate)
            })? {
                return publication(receipt);
            }
            let source = head(&base_receipt);
            let prepared = store
                .prepare_block_patch(project, &source, base, &patch)
                .map_err(error)?;
            if !reads_copy_source(prepared.certificate()) {
                return Err(invalid("Copy source lies outside the mask grid"));
            }
            store
                .admit_block_patch(project, job, &prepared)
                .map_err(error)?;
            publication(
                store
                    .publish(project, job, prepared.bytes())
                    .map_err(error)?,
            )
        }
        [action, job, target] if action == "undo" => {
            if let Some(receipt) = replay(&store, project, job, target, |receipt| {
                receipt.undo_of.as_deref() == Some(target)
            })? {
                return publication(receipt);
            }
            let source = head(&store.status(project, target).map_err(error)?);
            let prepared = store
                .prepare_block_undo(project, &source, target)
                .map_err(error)?;
            store
                .admit_block_patch(project, job, &prepared)
                .map_err(error)?;
            publication(
                store
                    .publish(project, job, prepared.bytes())
                    .map_err(error)?,
            )
        }
        [action, job] if action == "status" => encode(store.status(project, job).map_err(error)?),
        [action, part, slot] if action == "head" => {
            encode(store.slot_head(project, part, slot).map_err(error)?)
        }
        [action, job, output] if action == "export" => {
            let (receipt, bytes) = recover(&store, project, job)?;
            publish(Path::new(output), &bytes)?;
            encode(receipt)
        }
        [action, operation, left, right, channel] if action == "boolean" => {
            let operation = match operation.as_str() {
                "and" => "AND",
                "or" => "OR",
                "diff" => "DIFF",
                _ => return Err(usage()),
            };
            let directory = tempfile::tempdir().map_err(io_error)?;
            let operand = |name: &str, job: &str| -> Result<(Value, Receipt)> {
                let (receipt, bytes) = recover(&store, project, job)?;
                let field =
                    super::provider::decode(provider, &bytes, &receipt.binding.artifact_sha256)?;
                let path = directory.path().join(name);
                std::fs::write(&path, field.to_string()).map_err(io_error)?;
                Ok((
                    json!({"path":path,"channel":channel,"selection":{"kind":"Boolean"}}),
                    receipt,
                ))
            };
            let (left, left_source) = operand("left.json", left)?;
            let (right, right_source) = operand("right.json", right)?;
            let field = super::provider::operate(
                provider,
                "living.voxel.mask.boolean",
                &json!({
                    "left":left,"right":right,"operation":operation,
                    "output_channel":"mask","max_visited_blocks":512
                }),
            )?;
            Ok(json!({"result":field,"sources":[left_source,right_source]}))
        }
        [action, job] if action == "cancel" => {
            let state = match store.cancel(project, job).map_err(error)? {
                Cancellation::AlreadyPublished => "already_published",
                Cancellation::Cancelled => "cancelled",
                Cancellation::Superseded => "superseded",
            };
            Ok(json!({"state":state}))
        }
        _ => Err(usage()),
    }
}

fn replay(
    store: &PublicationStore,
    project: &ProjectStore,
    job: &str,
    source: &str,
    matches: impl FnOnce(&living_voxel_pilot::PatchCertificate) -> bool,
) -> Result<Option<Receipt>> {
    match store.status(project, job) {
        Err(Error::UnknownJob) => Ok(None),
        Err(fault) => Err(error(fault)),
        Ok(receipt)
            if receipt.binding.patch.as_ref().is_some_and(|certificate| {
                certificate.source_job == source && matches(certificate)
            }) =>
        {
            Ok((receipt.state != State::Admitted).then_some(receipt))
        }
        Ok(_) => Err(json!({"code":"RequestIdReuse"})),
    }
}

fn head(receipt: &Receipt) -> PartHead {
    PartHead {
        project_id: receipt.project_id.clone(),
        document_id: receipt.binding.document_id.clone(),
        native_revision: receipt.binding.native_revision,
        definition_hash: receipt.binding.definition_hash.clone(),
    }
}

fn is_mask(channel: &Value) -> bool {
    channel["dtype"] == "Bool"
        && matches!(
            channel["semantic_kind"].as_str(),
            Some("SampledIndicator" | "CellUnionMask")
        )
}

pub(super) fn recover(
    store: &PublicationStore,
    project: &ProjectStore,
    job: &str,
) -> Result<(Receipt, Vec<u8>)> {
    store
        .recover(project, job)
        .map_err(error)?
        .ok_or_else(|| json!({"code":"JobPending"}))
}

fn publication(receipt: Receipt) -> Result<Value> {
    if receipt.state == State::Superseded {
        Err(json!({"code":"Superseded","receipt":receipt}))
    } else {
        encode(receipt)
    }
}

fn invalid(message: &str) -> Value {
    json!({"code":"InvalidPatch","message":message})
}

pub(super) fn error(fault: Error) -> Value {
    let code = match fault {
        Error::Project(fault) => return core_error(fault),
        Error::DependencyChanged | Error::SourceChanged | Error::InputChanged => "InputChanged",
        Error::JobConflict => "RequestIdReuse",
        Error::UnknownJob => "UnknownJob",
        Error::InvalidPatch => "InvalidPatch",
        Error::InvalidSlot => "InvalidSlot",
        Error::InvalidValidator => "InvalidValidator",
        Error::InvalidLvx(_) | Error::CorruptRecord => "CorruptArtifact",
        Error::WrongDatabase => "WrongDatabase",
        Error::ResourceLimit => "LimitExceeded",
        Error::Storage(_) | Error::Encoding(_) | Error::Io(_) => "IoFailure",
    };
    json!({"code":code,"message":format!("{fault:?}")})
}
