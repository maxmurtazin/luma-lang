//! Luma recipe adapter for the pinned Core ProjectStore.
use super::*;
use hir_transaction::PreparationStage;
use hir_types::{RequestId, Revision};
use luma::{
    cad::{prepare_command, prepare_geometry},
    cad_engine::{Geometry, Parameter, ParameterSnapshot, RecipeGeometryPlan},
    host::WireSourceBundle,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

const TOKEN_DOMAIN: &[u8] = b"luma-prepared-recipe/1";
const PROFILE: &str = "__LUMA_CAD_INTEGRATION_PIN__";
const TTL_SECONDS: u64 = 300;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    kind: String,
    id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecipeRecord {
    version: u32,
    instance: String,
    source_digest: String,
    source: String,
    parameter_json: String,
    features: BTreeMap<String, Identity>,
    parameter_ids: BTreeMap<String, String>,
    retired_parameters: BTreeSet<String>,
    public_output: String,
    view: Option<SourcePin>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    version: u32,
    profile: String,
    project: String,
    document: String,
    expected_project: u64,
    expires: u64,
    recipe: RecipeRecord,
    snapshot: Option<(u64, String, String)>,
    command_source: Option<String>,
    changed: bool,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub token: Vec<u8>,
    pub project_revision: String,
    pub accepted_source_digest: String,
    pub candidate_pin: SourcePin,
    pub candidate_changed: bool,
}

impl ProjectStore {
    /// Prepare a recipe in the explicitly selected Part. The project is only read here.
    pub fn prepare_luma_recipe(
        &self,
        document: &str,
        expected_project: u64,
        source: &str,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        self.prepare_luma(document, expected_project, source, false, ctx)
    }

    /// Prepare a command and re-run the exact accepted recipe against its override overlay.
    pub fn prepare_luma_command(
        &self,
        document: &str,
        expected_project: u64,
        source: &str,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        self.prepare_luma(document, expected_project, source, true, ctx)
    }

    fn prepare_luma(
        &self,
        document: &str,
        expected_project: u64,
        source: &str,
        command: bool,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        let registry = self.registry()?;
        if registry.revision != expected_project {
            return Err(error(
                "Conflict",
                "Project revision changed before Luma preparation",
            ));
        }
        let (head, part, native_bytes) = part_head(&registry, document)?;
        let revision = Revision(head.native_revision);
        ctx.check(revision, PreparationStage::Validation)
            .map_err(overlay_error)?;
        let previous = registry.luma_recipes.get(document);
        let old: Value = serde_json::from_str(native_bytes).map_err(json_error)?;
        if previous.is_none()
            && [
                "/geometry/nodes",
                "/hir/entities",
                "/hir/components",
                "/hir/ports",
                "/dependencies",
            ]
            .into_iter()
            .any(|path| {
                old.pointer(path)
                    .and_then(Value::as_array)
                    .is_none_or(|items| !items.is_empty())
            })
        {
            return Err(error(
                "ManagedEntityConflict",
                "A new Luma recipe requires an empty Part",
            ));
        }
        let parameters: BTreeMap<String, Parameter> = previous
            .map(|record| serde_json::from_str(&record.parameter_json))
            .transpose()
            .map_err(json_error)?
            .unwrap_or_default();
        let inherited = ParameterSnapshot::new(parameters).map_err(luma_error)?;
        let admitted = admit(source)?;
        let (exact_source, bundle, values) = if command {
            let previous = previous.ok_or_else(|| {
                error("MissingAcceptedSource", "Part has no accepted Luma recipe")
            })?;
            let values = prepare_command(&admitted, document.into(), &inherited, 100_000)
                .map_err(execution_error)?;
            (previous.source.as_str(), admit(&previous.source)?, values)
        } else {
            (source, admitted, inherited)
        };
        let plan = prepare_geometry(&bundle, &values, 100_000).map_err(execution_error)?;
        let instance = living_cad_features::derived_id(uuid(document)?, "luma/recipe/1");
        let (next, features, view_requested) = canonical(old, instance, &plan, previous)?;
        let parameter_ids: BTreeMap<_, _> = plan
            .parameters
            .parameters
            .entries()
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    living_cad_features::derived_id(instance, &format!("parameter/{key}"))
                        .to_string(),
                )
            })
            .collect();
        let mut retired_parameters = BTreeSet::new();
        if let Some(previous) = previous {
            retired_parameters.extend(previous.retired_parameters.iter().cloned());
            retired_parameters.extend(
                previous
                    .parameter_ids
                    .keys()
                    .filter(|key| !parameter_ids.contains_key(*key))
                    .cloned(),
            );
            if features
                .keys()
                .chain(parameter_ids.keys())
                .any(|key| retired_parameters.contains(key))
                || parameter_ids
                    .keys()
                    .any(|key| previous.features.contains_key(key))
            {
                return Err(error(
                    "StableKeyMigrationRequired",
                    "Retired or reassigned recipe key",
                ));
            }
        }
        let (facade, store) = restore_part(part, document, revision.0, ctx)?;
        let bytes = serde_json::to_vec(&next).map_err(json_error)?;
        let cancel = Cancellation::from_shared_flag(ctx.cancellation_flag());
        let checked = facade.validate(&bytes, &cancel)?;
        let changed_model = checked.definition_hash().to_string() != head.definition_hash;
        let (native, snapshot) = if changed_model {
            let candidate =
                facade.prepare_import(&store, &bytes, RequestId::from_u128(1), revision, ctx)?;
            let snapshot = std::sync::Arc::new(candidate.snapshot().clone());
            let canonical = facade.export_snapshot(&snapshot, &cancel)?;
            let native = Some((
                snapshot.revision.0,
                canonical.definition_hash().to_string(),
                String::from_utf8(canonical.canonical_bytes().to_vec())
                    .map_err(|_| Error::internal())?,
            ));
            (native, snapshot)
        } else {
            (
                None,
                store.snapshot().map_err(living_hir_facade::commit_error)?,
            )
        };
        let (output, model) = Host::project_source(&facade, snapshot, &cancel)?;
        let expected = &features[&plan.outputs["solid"]].id;
        if &output.entity_id != expected {
            return Err(error(
                "UnsupportedCapability",
                "The selected output must be the terminal native solid",
            ));
        }
        let mut pin = source_pin(&registry, &model.pin);
        pin.output_ref = output;
        let record = RecipeRecord {
            version: 1,
            instance: instance.to_string(),
            source_digest: plan.source_digest,
            source: exact_source.into(),
            parameter_json: serde_json::to_string(plan.parameters.parameters.entries())
                .map_err(json_error)?,
            public_output: expected.clone(),
            features,
            parameter_ids,
            retired_parameters,
            view: view_requested.then(|| pin.clone()),
        };
        let changed = match previous {
            Some(previous) => {
                changed_model
                    || previous.source_digest != record.source_digest
                    || previous.parameter_json != record.parameter_json
                    || previous.view != record.view
            }
            None => true,
        };
        let candidate = Candidate {
            version: 1,
            profile: PROFILE.into(),
            project: registry.project_id,
            document: document.into(),
            expected_project,
            expires: now()?.saturating_add(TTL_SECONDS),
            recipe: record,
            snapshot: native,
            command_source: command.then(|| source.to_owned()),
            changed,
        };
        let token = seal(&self.key, TOKEN_DOMAIN, &candidate)?;
        Ok(Preview {
            token,
            project_revision: expected_project.to_string(),
            accepted_source_digest: candidate.recipe.source_digest,
            candidate_pin: pin,
            candidate_changed: changed,
        })
    }

    /// Accept exactly the authenticated preview at Core's SQLite CAS point.
    pub fn apply_luma(
        &mut self,
        token: &[u8],
        operation_id: &str,
        ctx: &PreparationContext,
    ) -> Result<Receipt, Error> {
        if token.len() > MAX_BYTES {
            return Err(error(
                "ResourceLimit",
                "Luma prepared candidate exceeds size limit",
            ));
        }
        let candidate: Candidate = unseal(&self.key, TOKEN_DOMAIN, token)?;
        if candidate.version != 1
            || candidate.profile != PROFILE
            || candidate.project != self.project_id
        {
            return Err(error(
                "PreparedContextChanged",
                "Luma candidate belongs to another provider or project",
            ));
        }
        let apply = |store: &mut Self| {
            store.commit(
                (
                    &candidate.document,
                    operation_id,
                    candidate.expected_project,
                ),
                &("luma-recipe/1", &candidate),
                ctx,
                |registry| {
                    if now()? > candidate.expires {
                        return Err(error("PreparedExpired", "Prepare the Luma candidate again"));
                    }
                    let entry = registry
                        .documents
                        .get_mut(&candidate.document)
                        .ok_or_else(|| error("MissingDocument", "Part document is absent"))?;
                    if let Some(ref native) = candidate.snapshot {
                        let part = registry
                            .parts
                            .get_mut(&candidate.document)
                            .ok_or_else(Error::internal)?;
                        part.snapshots.push(native.clone());
                        // This profile protects the managed graph from generic history edits.
                        part.timeline = None;
                        entry.revision = native.0;
                    } else if candidate.changed || candidate.command_source.is_some() {
                        registry.revision = registry
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| error("ResourceLimit", "Project revision exhausted"))?;
                    }
                    if candidate.changed {
                        registry
                            .luma_recipes
                            .insert(candidate.document.clone(), candidate.recipe.clone());
                    }
                    if let Some(source) = &candidate.command_source {
                        registry
                            .luma_commands
                            .entry(candidate.document.clone())
                            .or_default()
                            .insert(operation_id.to_owned(), source.clone());
                    }
                    Ok(entry.revision)
                },
            )
        };
        match apply(self) {
            Err(first)
                if self
                    .luma_receipt(&candidate.document, operation_id)?
                    .is_some() =>
            {
                // A racing writer may have accepted this request since the initial lookup.
                apply(self).map_err(|error| {
                    if error.code.as_ref() == "RequestIdReuse" {
                        error
                    } else {
                        first
                    }
                })
            }
            result => result,
        }
    }

    /// Look up the original Core receipt after a lost response or process restart.
    pub fn luma_receipt(
        &self,
        document: &str,
        operation_id: &str,
    ) -> Result<Option<Receipt>, Error> {
        uuid(document)?;
        if operation_id.len() != 32
            || !operation_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(error("InvalidPayload", "Invalid request identity"));
        }
        let Some(stored) =
            self.db
                .receipt(super::super::CommandActor::Owner, document, operation_id)?
        else {
            return Ok(None);
        };
        let domain = format!("receipt/{document}/{operation_id}");
        let receipt: Receipt = unseal(&self.key, domain.as_bytes(), &stored.bytes)?;
        if receipt.checkpoint.as_slice() != stored.checkpoint_hash
            || receipt.document_id != document
        {
            return Err(error("CheckpointCorrupt", "Receipt binding does not match"));
        }
        self.registry_at(&receipt)?;
        Ok(Some(receipt))
    }

    /// Resolve a View V2 scene from the exact accepted checkpoint.
    #[cfg(feature = "living-view-v2")]
    pub fn luma_scene(&self, receipt: &Receipt, scratch: &Path) -> Result<Value, Error> {
        let registry = self.registry_at(receipt)?;
        let pin = registry
            .luma_recipes
            .get(&receipt.document_id)
            .and_then(|recipe| recipe.view.as_ref())
            .ok_or_else(|| {
                error(
                    "MissingViewIntent",
                    "The accepted recipe has no View intent",
                )
            })?;
        let selection = render::Selection {
            project_id: registry.project_id.clone(),
            document_id: receipt.document_id.clone(),
            expected_project_revision: registry.revision,
            historical_revision: Some(pin.native_revision.to_string()),
            quality: Default::default(),
            transport: Default::default(),
            known_resources: Vec::new(),
            direct_fields: false,
        };
        serde_json::to_value(render::scene(
            &registry,
            &selection,
            scratch,
            &Cancellation::default(),
        )?)
        .map_err(json_error)
    }

    /// Resolve the accepted View intent at the original receipt's checkpoint.
    pub fn luma_view(&self, receipt: &Receipt) -> Result<Option<SourcePin>, Error> {
        let registry = self.registry_at(receipt)?;
        Ok(registry
            .luma_recipes
            .get(&receipt.document_id)
            .and_then(|record| record.view.clone()))
    }
}

fn now() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| Error::internal())
}

fn admit(source: &str) -> Result<luma::host::SourceBundle, Error> {
    if source.len() > 256 * 1024 {
        return Err(error("ResourceLimit", "Luma input frame exceeds 256 KiB"));
    }
    serde_json::from_str::<WireSourceBundle>(source)
        .map_err(json_error)?
        .admit()
        .map_err(source_error)
}

fn source_error(failure: luma::runtime::Failure) -> Error {
    let code = failure
        .diagnostics
        .first()
        .map(|(_, diagnostic)| diagnostic.code)
        .unwrap_or("InvalidSourceBundle");
    error(code, "Luma source failed validation").detail(
        "source_diagnostics",
        json!(failure
            .diagnostics
            .iter()
            .map(|(file, diagnostic)| luma::host::diagnostic_json(file, diagnostic))
            .collect::<Vec<_>>()),
    )
}
fn execution_error(err: luma::cad::PreparationError) -> Error {
    match err {
        luma::cad::PreparationError::Execution(failure) => source_error(failure),
        luma::cad::PreparationError::Parameters(problem) => luma_error(problem),
    }
}
fn luma_error(err: luma::host_api::Error) -> Error {
    error(err.code, &err.message)
}
fn overlay_error(err: hir_transaction::OverlayError) -> Error {
    living_hir_facade::commit_error(hir_transaction::CommitError::Overlay(err))
}

fn canonical(
    mut document: Value,
    instance: hir_types::EntityId,
    plan: &RecipeGeometryPlan,
    previous: Option<&RecipeRecord>,
) -> Result<(Value, BTreeMap<String, Identity>, bool), Error> {
    let output = plan
        .outputs
        .get("solid")
        .filter(|_| plan.outputs.len() == 1)
        .ok_or_else(|| {
            error(
                "UnsupportedCapability",
                "Project profile requires one public solid output",
            )
        })?;
    if plan.view_intents.iter().any(|view| &view.body != output) {
        return Err(error(
            "UnsupportedCapability",
            "View may show only the public solid output",
        ));
    }
    if let Some(previous) = previous {
        if previous.version != 1 || previous.instance != instance.to_string() {
            return Err(error(
                "IncompatibleRecipe",
                "Stored recipe identity or version differs",
            ));
        }
        if previous
            .features
            .keys()
            .any(|key| !plan.features.iter().any(|feature| &feature.key == key))
        {
            return Err(error(
                "UnsupportedCapability",
                "Removing accepted features requires reference reconciliation",
            ));
        }
    }
    // The Part is empty at adoption and reserved for this recipe thereafter.
    // Recreate only its managed graph, preserving the native document and module.
    document["modules"][0]["entities"] = json!([]);
    for field in ["entities", "components", "ports"] {
        document["hir"][field] = json!([]);
    }
    document["geometry"]["nodes"] = json!([]);
    document["dependencies"] = json!([]);
    document["capabilities"] = json!(["s1.hir", "s1.geometry", "s1.graph.metadata"]);
    let mut features = BTreeMap::<String, Identity>::new();
    for feature in &plan.features {
        let id = living_cad_features::derived_id(instance, &format!("feature/{}", feature.key));
        let kind = match &feature.geometry {
            Geometry::Plate { size_m } => {
                node(&mut document, id, "cad.create_plate.v1", json!({}), "Plate");
                for (slot, value) in ["width", "height", "thickness"].into_iter().zip(*size_m) {
                    let parameter =
                        living_cad_features::derived_id(id, &format!("parameter/{slot}"));
                    node(
                        &mut document,
                        parameter,
                        "core.parameter.v1",
                        json!({
                            "name":slot,"dimension":[1,0,0],"binding":{"literal":{"decimal":value.to_string(),"unit":"m"}}
                        }),
                        slot,
                    );
                    document["dependencies"].as_array_mut().unwrap().push(json!({
                        "id":living_cad_features::derived_id(id, &format!("edge/{slot}")).to_string(),
                        "kind":"core.depends_on","schema_version":1,"consumer":id.to_string(),
                        "prerequisite":parameter.to_string(),"slot":slot
                    }));
                }
                "plate"
            }
            Geometry::ThroughHole {
                base,
                diameter_m,
                centre_m: [x, y],
                entry_z_m,
            } => {
                let base = features
                    .get(base)
                    .ok_or_else(|| error("InvalidPlan", "Hole base must be earlier"))?;
                let request = json!({
                    "op":"create_hole", "feature_id":id.to_string(),"base":base.id,
                    "payload":{"end":"through_all","head":"none","direction":"opposite_normal",
                        "placement":{"kind":"datum","pose":{
                            "translation_m":[x.to_string(),y.to_string(),entry_z_m.to_string()],
                            "rotation_xyzw":["0","0","0","1"]}}},
                    "parameters":{"diameter":diameter_m.to_string()}
                });
                (document, _) = crate::feature_authoring::apply(
                    &document,
                    serde_json::from_value(request).map_err(json_error)?,
                )?;
                "through_hole"
            }
        };
        if previous
            .and_then(|record| record.features.get(&feature.key))
            .is_some_and(|prior| prior.kind != kind || prior.id != id.to_string())
        {
            return Err(error(
                "FeatureTypeMigrationRequired",
                "Stable key changed feature kind or identity",
            ));
        }
        features.insert(
            feature.key.clone(),
            Identity {
                kind: kind.into(),
                id: id.to_string(),
            },
        );
    }
    let root = &features
        .get(output)
        .ok_or_else(|| error("InvalidPlan", "Solid output is missing"))?
        .id;
    if previous.is_some_and(|record| &record.public_output != root) {
        return Err(error(
            "UnsupportedCapability",
            "Replacing the public solid requires reference reconciliation",
        ));
    }
    crate::editor::set_root(&mut document, Some(root))?;
    crate::editor::manifest(&mut document)?;
    Ok((document, features, !plan.view_intents.is_empty()))
}

fn node(document: &mut Value, id: hir_types::EntityId, kind: &str, payload: Value, name: &str) {
    crate::editor::append_node(
        document,
        &id.to_string(),
        &living_cad_features::derived_id(id, "component").to_string(),
        kind,
        payload,
        name,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use living_frep::Point3;

    const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
    const PART: &str = "00000000-0000-4000-8000-000000000002";
    const RECIPE: &str = include_str!("plate-geometry.json");
    const COMMAND: &str = include_str!("plate-width-command.json");

    fn operation(n: u32) -> String {
        format!("{n:032x}")
    }

    #[test]
    fn luma_cad1_recipe_command_cas_retry_restart() {
        let root = super::super::super::tests::Directory::new();
        let path = root.0.join("luma.db");
        let key = [9; 32];
        let mut store = ProjectStore::create(&path, PROJECT, key).unwrap();
        let ctx = PreparationContext::default;
        store
            .create_document(
                &CreateDocument {
                    request_id: operation(1),
                    document_id: PART.into(),
                    expected_project_revision: 0,
                    kind: DocumentKind::Part,
                    name: "Luma plate".into(),
                },
                &ctx(),
            )
            .unwrap();
        let preview = store.prepare_luma_recipe(PART, 1, RECIPE, &ctx()).unwrap();
        assert_eq!(store.registry().unwrap().revision, 1);
        let mut modified = preview.token.clone();
        let last = modified.len() - 3;
        modified[last] ^= 1;
        assert!(store.apply_luma(&modified, &operation(2), &ctx()).is_err());
        assert_eq!(store.registry().unwrap().revision, 1);
        let accepted = store
            .apply_luma(&preview.token, &operation(2), &ctx())
            .unwrap();
        assert_eq!(accepted.project_revision, 2);
        let first_pin = store.luma_view(&accepted).unwrap().unwrap();
        check_holes(&store, &first_pin, 0.04, 0.05);

        let stale = store.prepare_luma_recipe(PART, 2, RECIPE, &ctx()).unwrap();
        assert!(!stale.candidate_changed);
        let command = store
            .prepare_luma_command(PART, 2, COMMAND, &ctx())
            .unwrap();

        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let cancelled = PreparationContext::new(Default::default(), flag);
        assert!(store
            .apply_luma(&command.token, &operation(3), &cancelled)
            .is_err());
        assert!(store.luma_receipt(PART, &operation(3)).unwrap().is_none());
        assert_eq!(store.registry().unwrap().revision, 2);

        let updated = store
            .apply_luma(&command.token, &operation(3), &ctx())
            .unwrap();
        assert_eq!(updated.project_revision, 3);
        let updated_pin = store.luma_view(&updated).unwrap().unwrap();
        assert_eq!(updated_pin.output_ref, first_pin.output_ref);
        assert_ne!(updated_pin.native_revision, first_pin.native_revision);
        check_holes(&store, &updated_pin, 0.05, 0.06);
        check_holes(&store, &first_pin, 0.04, 0.05);
        assert_eq!(
            store
                .apply_luma(&stale.token, &operation(4), &ctx())
                .unwrap_err()
                .code
                .as_ref(),
            "Conflict"
        );
        assert_eq!(
            store
                .apply_luma(&stale.token, &operation(3), &ctx())
                .unwrap_err()
                .code
                .as_ref(),
            "RequestIdReuse"
        );
        assert_eq!(
            store
                .apply_luma(&command.token, &operation(3), &ctx())
                .unwrap(),
            updated
        );
        let registry = store.registry().unwrap();
        assert_eq!(registry.luma_recipes[PART].source, RECIPE);
        assert_eq!(registry.luma_commands[PART][&operation(3)], COMMAND);

        drop(store);
        let mut reopened = ProjectStore::open(&path, PROJECT, key).unwrap();
        assert_eq!(
            reopened.luma_receipt(PART, &operation(3)).unwrap().unwrap(),
            updated
        );
        assert_eq!(reopened.luma_view(&accepted).unwrap().unwrap(), first_pin);
        let no_change = reopened
            .prepare_luma_recipe(PART, 3, RECIPE, &ctx())
            .unwrap();
        assert!(!no_change.candidate_changed);
        assert_eq!(
            reopened
                .apply_luma(&no_change.token, &operation(5), &ctx())
                .unwrap()
                .project_revision,
            3
        );
        let protected = DocumentCommand {
            request_id: operation(6),
            document_id: PART.into(),
            expected_project_revision: 3,
            expected_document_revision: updated.document_revision,
            action: DocumentAction::Undo,
        };
        assert_eq!(
            reopened
                .apply_document(&protected, &root.0, &ctx())
                .unwrap_err()
                .code
                .as_ref(),
            "ManagedEntityConflict"
        );

        #[cfg(feature = "living-view-v2")]
        {
            let selection: render::Selection = serde_json::from_value(json!({
                "project_id": PROJECT, "expected_project_revision": 3, "document_id": PART,
                "historical_revision": updated.document_revision.to_string()
            }))
            .unwrap();
            let scene = render::scene(
                &reopened.registry().unwrap(),
                &selection,
                &root.0,
                &Cancellation::default(),
            )
            .unwrap();
            let value = serde_json::to_value(scene).unwrap();
            let text = value.to_string();
            assert!(text.contains(&updated_pin.definition_hash));
            assert!(text.contains("resources"));
        }
    }

    fn check_holes(store: &ProjectStore, pin: &SourcePin, centre: f64, half_width: f64) {
        let part = store.resolve_source(pin).unwrap();
        let sample = |x, y, z| {
            part.solid
                .sample_at(Point3::new([x, y, z]).unwrap())
                .unwrap()
                .value_m
                .unwrap()
        };
        for x in [-centre, centre] {
            for z in [-0.0038, 0.0, 0.0038] {
                assert!(sample(x, 0.0, z) > 0.0);
                assert!(sample(x, 0.0032, z) < 0.0);
            }
        }
        assert!(sample(half_width - 0.001, 0.012, 0.0) < 0.0);
        assert!(sample(half_width + 0.001, 0.012, 0.0) > 0.0);
    }
}
