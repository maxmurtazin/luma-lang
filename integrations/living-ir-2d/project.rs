mod branch;
mod core_reads;
mod recipe;

use core_reads::CoreReads;

use super::{part_head, restore_part, PartHead, ProjectStore, Receipt, Registry};
use crate::{error, json_error, Error};
use hir_evaluation_api::Cancellation;
use hir_transaction::{PreparationContext, PreparationStage};
use hir_types::{RequestId, Revision};
use lir2d_runtime::{CommandEnvelope, Runtime};
use lir2d_types::{DocumentSnapshot, ErrorCode, ErrorEnvelope, ObjectId};
use luma::cad_engine::ProfileExtrusion;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Core's registered representation for one Living IR 2D document. The
/// canonical provider payload is decoded through DocumentSnapshot on use.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Embedded2dV1 {
    schema: EmbeddedSchema,
    pub(super) revision: u64,
    snapshot_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recipe: Option<recipe::AcceptedRecipe>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum EmbeddedSchema {
    #[serde(rename = "living-ir.2d.embedded/1")]
    V1,
}

impl Embedded2dV1 {
    fn new(document: &str, snapshot: &DocumentSnapshot) -> Result<Self, Error> {
        check_scope(document, snapshot)?;
        let snapshot_json = serde_json::to_string(snapshot).map_err(json_error)?;
        if snapshot_json.len() > lir2d_types::MAX_SNAPSHOT_BYTES {
            return Err(error(
                "ResourceLimit",
                "Embedded 2D snapshot exceeds its byte limit",
            ));
        }
        Ok(Self {
            schema: EmbeddedSchema::V1,
            revision: snapshot.revision.0,
            snapshot_json,
            recipe: None,
        })
    }

    fn successor(&self, document: &str, snapshot: &DocumentSnapshot) -> Result<Self, Error> {
        if self.revision.checked_add(1) != Some(snapshot.revision.0) {
            return Err(error(
                "InternalError",
                "Unexpected provider candidate revision",
            ));
        }
        let mut next = Self::new(document, snapshot)?;
        next.recipe = self.recipe.clone();
        Ok(next)
    }

    pub(super) fn managed_recipe(&self) -> bool {
        self.recipe.as_ref().is_some_and(|recipe| !recipe.detached)
    }

    fn snapshot(&self, document: &str) -> Result<DocumentSnapshot, Error> {
        let snapshot = DocumentSnapshot::from_json(self.snapshot_json.as_bytes())
            .map_err(|problem| provider_error(&problem))?;
        check_scope(document, &snapshot)?;
        if snapshot.revision.0 != self.revision {
            return Err(error(
                "CheckpointCorrupt",
                "Embedded 2D revision does not match its payload",
            ));
        }
        Ok(snapshot)
    }
}

impl ProjectStore {
    pub fn register_embedded_2d(
        &mut self,
        part: &str,
        operation: &str,
        expected_project: u64,
        bytes: &[u8],
        context: &PreparationContext,
    ) -> Result<Receipt, Error> {
        let snapshot =
            DocumentSnapshot::from_json(bytes).map_err(|problem| provider_error(&problem))?;
        let record = Embedded2dV1::new(part, &snapshot)?;
        let command = ("register-embedded-2d/1", expected_project, &record);
        self.commit(
            (part, operation, expected_project),
            &command,
            context,
            |registry| {
                let (head, _, _) = part_head(registry, part)?;
                if registry.embedded_2d.contains_key(part) {
                    return Err(error(
                        "Conflict",
                        "Part already has registered embedded 2D state",
                    ));
                }
                let mut reads = CoreReads::new(registry, context);
                reads.snapshot(&snapshot, true)?;
                reads.dependencies(&snapshot)?;
                registry.embedded_2d.insert(part.to_owned(), record.clone());
                Ok(head.native_revision)
            },
        )
    }

    pub fn apply_embedded_2d(
        &mut self,
        part: &str,
        operation: &str,
        expected_project: u64,
        bytes: &[u8],
        extrusion: Option<&ProfileExtrusion>,
        context: &PreparationContext,
    ) -> Result<Receipt, Error> {
        let batch =
            CommandEnvelope::from_json(bytes).map_err(|problem| provider_error(&problem))?;
        let project = ObjectId::try_from(self.project_id.clone())
            .map_err(|_| error("InvalidPayload", "Invalid provider project identity"))?;
        if batch.project_id != project || batch.document_id.as_str() != part {
            return Err(error(
                "InvalidPayload",
                "2D batch does not target this project and Part",
            ));
        }
        let command = ("apply-embedded-2d/1", expected_project, &batch, extrusion);
        self.commit(
            (part, operation, expected_project),
            &command,
            context,
            |registry| {
                let (head, _, _) = part_head(registry, part)?;
                let record = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
                reject_recipe_edit(registry, part)?;
                if record.revision != batch.expected_revision.0 {
                    return Err(error("Conflict", "Embedded 2D revision changed"));
                }
                let native_revision = Revision(head.native_revision);
                context
                    .check(native_revision, PreparationStage::Validation)
                    .map_err(living_hir_facade::overlay_error)?;
                let base = record.snapshot(part)?;
                let mut reads = CoreReads::new(registry, context);
                reads.snapshot(&base, false)?;
                reads.commands(&batch.commands)?;
                let sources = reads.dependencies(&base)?;
                let candidate = Runtime::prepare_embedded(
                    project,
                    base,
                    &batch,
                    sources,
                    &context.cancellation_flag(),
                )
                .map_err(|problem| provider_error(&problem))?;
                reads.snapshot(&candidate, true)?;
                let staged = record.successor(part, &candidate)?;
                context
                    .check(native_revision, PreparationStage::Sealing)
                    .map_err(living_hir_facade::overlay_error)?;
                let revision = match extrusion {
                    Some(intent) => {
                        stage_extrusion(registry, &head, &candidate, intent, false, context)?
                    }
                    None => head.native_revision,
                };
                registry.embedded_2d.insert(part.to_owned(), staged);
                Ok(revision)
            },
        )
    }

    pub fn embedded_2d_revision_at(&self, receipt: &Receipt) -> Result<u64, Error> {
        let registry = self.registry_at(receipt)?;
        Ok(registry
            .embedded_2d
            .get(&receipt.document_id)
            .ok_or_else(unregistered)?
            .revision)
    }

    pub fn embedded_2d(&self, part: &str, sketch: Option<&str>) -> Result<Value, Error> {
        let registry = self.registry()?;
        let (head, _, _) = part_head(&registry, part)?;
        let snapshot = registry
            .embedded_2d
            .get(part)
            .ok_or_else(unregistered)?
            .snapshot(part)?;
        let payload = if let Some(id) = sketch {
            let id = ObjectId::try_from(id.to_owned())
                .map_err(|_| error("InvalidPayload", "Invalid Sketch ID"))?;
            let mut report = serde_json::to_value(
                snapshot
                    .sketch_report(&id)
                    .map_err(|problem| provider_error(&problem))?,
            )
            .map_err(json_error)?;
            let authored = snapshot
                .sketches()
                .get(&id)
                .ok_or_else(|| error("CheckpointCorrupt", "Missing Sketch for solver report"))?;
            report["solver_policy"] = json!(authored.solver_policy);
            report["branch_token"] = json!(authored.branch_token);
            report
        } else {
            serde_json::to_value(&snapshot).map_err(json_error)?
        };
        Ok(json!({
            "project_id": registry.project_id,
            "project_revision": registry.revision.to_string(),
            "part_revision": head.native_revision.to_string(),
            "embedded_revision": snapshot.revision.0.to_string(),
            "value": payload
        }))
    }
}

pub(super) fn reject_recipe_edit(registry: &Registry, part: &str) -> Result<(), Error> {
    if registry
        .embedded_2d
        .get(part)
        .is_some_and(Embedded2dV1::managed_recipe)
    {
        return Err(error(
            "ManagedEntityConflict",
            "Update the accepted Sketch recipe to edit this Part",
        ));
    }
    Ok(())
}

fn check_scope(document: &str, snapshot: &DocumentSnapshot) -> Result<(), Error> {
    if snapshot.document_id.as_str() != document {
        return Err(error(
            "InvalidPayload",
            "2D document identity must match the Part",
        ));
    }
    if snapshot.is_standalone() {
        return Err(error(
            "UnsupportedEmbedded2d",
            "Registration requires an embedded snapshot",
        ));
    }
    if snapshot
        .sheets()
        .values()
        .flat_map(|sheet| &sheet.views)
        .any(|view| view.core_projection.is_some())
    {
        return Err(error(
            "UnsupportedEmbedded2d",
            "Embedded sheet Core projections require the drawing publication profile",
        ));
    }
    Ok(())
}

fn unregistered() -> Error {
    error(
        "UnsupportedEmbedded2d",
        "Part has no registered embedded 2D state",
    )
}

fn provider_error(problem: &ErrorEnvelope) -> Error {
    let code = match problem.code {
        ErrorCode::Cancelled => "Cancelled",
        ErrorCode::ResourceLimit => "ResourceLimit",
        ErrorCode::SchemaUnsupported => "UnsupportedEmbedded2d",
        _ => "Embedded2dRejected",
    };
    error(code, &problem.details.reason).detail(
        "provider_diagnostic",
        serde_json::to_string(problem).unwrap_or_else(|_| problem.message_key.to_owned()),
    )
}

fn stage_extrusion(
    registry: &mut Registry,
    head: &PartHead,
    source: &DocumentSnapshot,
    intent: &ProfileExtrusion,
    rebuild: bool,
    context: &PreparationContext,
) -> Result<u64, Error> {
    use crate::sketch::{Decimal, Element, Sketch};
    use living_cad_features::extrude::Direction;
    use std::collections::BTreeMap;

    let document = &head.document_id;
    if registry
        .luma_recipes
        .get(document)
        .is_some_and(|record| record.is_managed())
    {
        return Err(error(
            "ManagedEntityConflict",
            "The Part is owned by an accepted recipe",
        ));
    }
    let profile_id = ObjectId::try_from(intent.profile_id.clone())
        .map_err(|_| error("InvalidPayload", "Invalid embedded Profile ID"))?;
    let profile = source
        .profile_artifact(&profile_id)
        .map_err(|problem| provider_error(&problem))?;
    let profile = profile.solid().ok_or_else(|| {
        error(
            "UnsupportedGeometry",
            "Normal extrusion requires a solid profile",
        )
    })?;
    let part = registry
        .parts
        .get_mut(document)
        .ok_or_else(|| error("CheckpointCorrupt", "Missing Part"))?;
    let revision = Revision(head.native_revision);
    let (facade, store) = restore_part(part, document, revision.0, context)?;
    let pin = store.snapshot().map_err(living_hir_facade::commit_error)?;
    let cancel = Cancellation::from_shared_flag(context.cancellation_flag());
    let canonical = facade.export_snapshot(&pin, &cancel)?;
    let mut model: Value =
        serde_json::from_slice(canonical.canonical_bytes()).map_err(json_error)?;
    let old_sketch = crate::sketch::read(&model)?;
    if !model["geometry"]["nodes"]
        .as_array()
        .is_some_and(Vec::is_empty)
        || (!rebuild && old_sketch.is_some())
    {
        return Err(error(
            "UnsupportedGeometry",
            "Embedded Profile extrusion requires an empty native Part",
        ));
    }
    if rebuild {
        let prior = crate::extrude::read(&model)?
            .ok_or_else(|| error("CheckpointCorrupt", "Missing managed extrusion"))?;
        if prior.features.len() != 1
            || prior.features[0].id != intent.feature_id
            || prior.pocket_sketch.is_some()
            || old_sketch
                .as_ref()
                .is_none_or(|sketch| sketch.id != profile.sketch_id.as_str())
        {
            return Err(error(
                "CheckpointCorrupt",
                "Managed extrusion identity differs",
            ));
        }
    }

    let mut elements = BTreeMap::new();
    for edge in &profile.outer {
        for (id, xy) in [(&edge.from_id, edge.start_m), (&edge.to_id, edge.end_m)] {
            elements.insert(
                id.as_str().into(),
                Element::Point {
                    xy: xy.map(Decimal),
                },
            );
        }
        elements.insert(
            edge.entity_id.as_str().into(),
            Element::Line {
                start: edge.from_id.as_str().into(),
                end: edge.to_id.as_str().into(),
            },
        );
    }
    for hole in &profile.holes {
        elements.insert(
            hole.center_id.as_str().into(),
            Element::Point {
                xy: hole.center_m.map(Decimal),
            },
        );
        elements.insert(
            hole.entity_id.as_str().into(),
            Element::Circle {
                center: hole.center_id.as_str().into(),
                radius: Decimal(hole.radius_m),
            },
        );
    }
    let sketch = Sketch {
        format: "living-ir.sketch-sidecar/v1".into(),
        id: profile.sketch_id.as_str().into(),
        plane: "XY".into(),
        sketch_revision: source.revision.0,
        parent_digest: old_sketch.as_ref().map(Sketch::digest).transpose()?,
        cad_document_id: document.clone(),
        created_at_revision: old_sketch
            .as_ref()
            .map_or(revision.0, |sketch| sketch.created_at_revision),
        source_definition_hash: old_sketch.map_or_else(
            || head.definition_hash.clone(),
            |sketch| sketch.source_definition_hash,
        ),
        source_entity: None,
        numeric_profile: "sk1-research-example/v1".into(),
        elements,
    };
    sketch.model()?;
    let command = if rebuild {
        crate::extrude::refresh_source(
            &mut model,
            serde_json::to_string(&sketch).map_err(json_error)?,
            &cancel,
        )?;
        crate::extrude::Command::UpdateExtrudeLength {
            feature_id: intent.feature_id.clone(),
            length_m: intent.signed_height_m.to_string(),
        }
    } else {
        crate::sketch::write(&mut model, &sketch)?;
        crate::extrude::Command::CreateExtrude {
            feature_id: intent.feature_id.clone(),
            source_digest: sketch.digest()?,
            length_m: intent.signed_height_m.to_string(),
            direction: Direction::OneSided,
        }
    };
    let (model, _) = crate::extrude::apply(&model, command, &cancel)?;
    let bytes = serde_json::to_vec(&model).map_err(json_error)?;
    let candidate =
        facade.prepare_import(&store, &bytes, RequestId::from_u128(1), revision, context)?;
    let snapshot = candidate.snapshot();
    let native = facade.export_snapshot(snapshot, &cancel)?;
    let bytes =
        String::from_utf8(native.canonical_bytes().to_vec()).map_err(|_| Error::internal())?;
    let timeline = store
        .checkpoint_timeline(Some(&candidate))
        .map_err(living_hir_facade::commit_error)?;
    part.timeline = Some(timeline);
    part.snapshots.push((
        snapshot.revision.0,
        native.definition_hash().to_string(),
        bytes,
    ));
    registry
        .documents
        .get_mut(document)
        .ok_or_else(|| error("CheckpointCorrupt", "Missing Part entry"))?
        .revision = snapshot.revision.0;
    Ok(snapshot.revision.0)
}
