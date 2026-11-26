//! Luma recipe adapter for the pinned Core ProjectStore.
mod bindings;
mod jobs;
use super::*;
pub use bindings::LengthBinding;
use hir_transaction::PreparationStage;
use hir_types::{RequestId, Revision};
use luma::{
    cad_engine::{
        CommandHost, Geometry, GeometryHost, ParamValue, Parameter, ParameterSnapshot,
        PreparedRecipe, RecipeGeometryPlan,
    },
    host::{run_with_host, Dispatch, Mode, SourceBundle, WireSourceBundle},
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
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    retired_features: BTreeSet<String>,
    #[serde(default)]
    detached: bool,
    #[serde(default)]
    accepted_revision: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bindings: Vec<LengthBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    binding_base: BTreeMap<String, Option<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    historical_outputs: BTreeMap<u64, String>,
    public_output: String,
    view: Option<SourcePin>,
}

impl RecipeRecord {
    pub(super) fn is_managed(&self) -> bool {
        !self.detached
    }
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
    parts: Vec<PartChange>,
    command_source: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PartChange {
    document: String,
    recipe: RecipeRecord,
    snapshot: Option<(u64, String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued: Option<jobs::QueueRequest>,
    changed: bool,
}

struct PreviewRequest<'a> {
    document: &'a str,
    expected: u64,
    source: &'a str,
    command: bool,
    job_keys: &'a [String],
    bindings: Option<&'a [LengthBinding]>,
}

struct PartRequest<'a> {
    document: &'a str,
    source: &'a str,
    command: bool,
    job_keys: &'a [String],
    bindings: &'a [LengthBinding],
    inputs: BTreeMap<String, ParamValue>,
}

#[derive(Debug, Serialize)]
pub struct DetachReport {
    pub recipe: String,
    pub accepted_source_digest: String,
    pub released_entities: Vec<String>,
    pub released_bindings: Vec<LengthBinding>,
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub token: Vec<u8>,
    pub project_revision: String,
    pub accepted_source_digest: String,
    pub candidate_pin: SourcePin,
    pub candidate_changed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detach: Option<DetachReport>,
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
        self.prepare_luma_with_jobs(document, expected_project, source, false, &[], ctx)
    }

    /// Prepare a command and re-run the exact accepted recipe against its override overlay.
    pub fn prepare_luma_command(
        &self,
        document: &str,
        expected_project: u64,
        source: &str,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        self.prepare_luma_with_jobs(document, expected_project, source, true, &[], ctx)
    }

    /// Release the complete native region while retaining its frozen definitions
    /// and the exact source and IDs of its former owner in the checkpoint.
    pub fn prepare_luma_detach(
        &self,
        document: &str,
        expected_project: u64,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        let registry = self.registry()?;
        if registry.revision != expected_project {
            return Err(error("Conflict", "Project revision changed before detach"));
        }
        let (head, _, _) = part_head(&registry, document)?;
        bindings::reject_followers(&registry, document)?;
        ctx.check(Revision(head.native_revision), PreparationStage::Validation)
            .map_err(overlay_error)?;
        let mut recipe = registry
            .luma_recipes
            .get(document)
            .filter(|recipe| !recipe.detached)
            .cloned()
            .ok_or_else(|| error("MissingAcceptedSource", "Part has no managed recipe"))?;
        let pin = SourcePin {
            project_id: registry.project_id.clone(),
            document_id: document.into(),
            native_revision: head.native_revision,
            definition_hash: head.definition_hash,
            output_ref: OutputRef {
                entity_id: recipe.public_output.clone(),
                port: "solid".into(),
            },
        };
        let detach = DetachReport {
            recipe: recipe.instance.clone(),
            accepted_source_digest: recipe.source_digest.clone(),
            released_bindings: recipe.bindings.clone(),
            released_entities: recipe
                .features
                .values()
                .map(|feature| feature.id.clone())
                .chain(recipe.parameter_ids.values().cloned())
                .collect(),
        };
        // Native parameter inputs are already literal at the accepted state.
        // Preserve the historical View intent and stop subsequent recipe writes.
        recipe.detached = true;
        let candidate = Candidate {
            version: 1,
            profile: PROFILE.into(),
            project: registry.project_id,
            document: document.into(),
            expected_project,
            expires: now()?.saturating_add(TTL_SECONDS),
            parts: vec![PartChange {
                document: document.into(),
                recipe,
                snapshot: None,
                queued: None,
                changed: true,
            }],
            command_source: None,
        };
        Ok(Preview {
            token: seal(&self.key, TOKEN_DOMAIN, &candidate)?,
            project_revision: expected_project.to_string(),
            accepted_source_digest: detach.accepted_source_digest.clone(),
            candidate_pin: pin,
            candidate_changed: true,
            detach: Some(detach),
        })
    }

    /// Admit an explicit set of job inputs alongside the ordinary source bundle.
    pub fn prepare_luma_with_jobs(
        &self,
        document: &str,
        expected_project: u64,
        source: &str,
        command: bool,
        job_keys: &[String],
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        self.prepare_luma(
            PreviewRequest {
                document,
                expected: expected_project,
                source,
                command,
                job_keys,
                bindings: None,
            },
            ctx,
        )
    }

    /// Replace the explicit Length bindings and prepare the whole resulting closure.
    pub fn prepare_luma_bindings(
        &self,
        document: &str,
        expected: u64,
        bindings: &[LengthBinding],
        job_keys: &[String],
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        self.prepare_luma(
            PreviewRequest {
                document,
                expected,
                source: "",
                command: false,
                job_keys,
                bindings: Some(bindings),
            },
            ctx,
        )
    }

    pub fn luma_recipe_info(&self, document: &str) -> Result<Value, Error> {
        uuid(document)?;
        let registry = self.registry()?;
        let recipe = registry
            .luma_recipes
            .get(document)
            .ok_or_else(|| error("MissingAcceptedSource", "Part has no accepted recipe"))?;
        Ok(json!({
            "recipe":recipe.instance, "recipe_revision":recipe.accepted_revision,
            "detached":recipe.detached, "bindings":recipe.bindings,
            "outputs":bindings::published(&recipe.parameters()?)
        }))
    }

    fn prepare_luma(
        &self,
        request: PreviewRequest<'_>,
        ctx: &PreparationContext,
    ) -> Result<Preview, Error> {
        let registry = self.registry()?;
        if registry.revision != request.expected {
            return Err(error(
                "Conflict",
                "Project revision changed before Luma preparation",
            ));
        }
        part_head(&registry, request.document)?;
        let primary_source = if request.bindings.is_some() {
            &registry
                .luma_recipes
                .get(request.document)
                .ok_or_else(|| error("MissingAcceptedSource", "Part has no managed recipe"))?
                .source
        } else {
            request.source
        };
        let graph = bindings::graph(&registry, request.document, request.bindings)?;
        let edited = [request.document.to_owned()].into();
        let batch = graph
            .prepare(&edited, |document, values| {
                let primary = document == request.document;
                let admitted = registry.luma_recipes.get(document);
                let effective_bindings =
                    admitted.map_or(&[][..], |record| record.bindings.as_slice());
                let part_request = PartRequest {
                    document,
                    source: if primary {
                        primary_source
                    } else {
                        &admitted.ok_or_else(Error::internal)?.source
                    },
                    command: primary && request.command,
                    job_keys: if primary { request.job_keys } else { &[] },
                    bindings: if primary {
                        request.bindings.unwrap_or(effective_bindings)
                    } else {
                        effective_bindings
                    },
                    inputs: bindings::input_values(values)?,
                };
                let (candidate, pin) = self.prepare_part(&registry, part_request, ctx)?;
                let outputs = bindings::prepared_outputs(&candidate.recipe)?;
                Ok(PreparedRecipe {
                    change: (candidate, pin),
                    outputs,
                })
            })
            .map_err(bindings::closure_error)?;

        let mut parts = Vec::with_capacity(batch.changes.len());
        let mut primary = None;
        for (_, prepared) in batch.changes {
            let (candidate, pin) = prepared.change;
            if candidate.document == request.document {
                primary = Some((pin, candidate.recipe.source_digest.clone()));
            }
            parts.push(candidate);
        }
        let (pin, accepted_source_digest) = primary.ok_or_else(Error::internal)?;
        let candidate_changed = parts.iter().any(|part| part.changed);
        let candidate = Candidate {
            version: 1,
            profile: PROFILE.into(),
            project: registry.project_id,
            document: request.document.into(),
            expected_project: batch.project_revision,
            expires: now()?.saturating_add(TTL_SECONDS),
            parts,
            command_source: request.command.then(|| request.source.to_owned()),
        };
        let token = seal(&self.key, TOKEN_DOMAIN, &candidate)?;
        if token.len() > MAX_BYTES {
            return Err(error(
                "ResourceLimit",
                "Luma prepared closure exceeds size limit",
            ));
        }
        Ok(Preview {
            token,
            project_revision: request.expected.to_string(),
            accepted_source_digest,
            candidate_pin: pin,
            candidate_changed,
            detach: None,
        })
    }

    fn prepare_part(
        &self,
        registry: &Registry,
        request: PartRequest<'_>,
        ctx: &PreparationContext,
    ) -> Result<(PartChange, SourcePin), Error> {
        let PartRequest {
            document,
            source,
            command,
            job_keys,
            bindings,
            inputs,
        } = request;
        let (head, part, native_bytes) = part_head(registry, document)?;
        let revision = Revision(head.native_revision);
        ctx.check(revision, PreparationStage::Validation)
            .map_err(overlay_error)?;
        let previous = registry.luma_recipes.get(document);
        if previous.is_some_and(|record| record.detached) {
            return Err(error("DetachedRecipe", "The Part's recipe was detached"));
        }
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
            .map(RecipeRecord::parameters)
            .transpose()?
            .unwrap_or_default();
        let mut inherited = ParameterSnapshot::new(parameters).map_err(luma_error)?;
        if let Some(previous) = previous {
            let mut base = Vec::with_capacity(previous.binding_base.len());
            for (key, value) in &previous.binding_base {
                let value = value
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(json_error)?;
                base.push((key.as_str(), value));
            }
            inherited = inherited.with_overrides(base).map_err(luma_error)?;
        }
        let binding_base = inputs
            .keys()
            .map(|key| {
                let value = inherited.entries().get(key).ok_or_else(|| {
                    error("UnknownParameter", "Binding target parameter is absent")
                })?;
                let saved = value
                    .override_value
                    .map(|value| serde_json::to_string(&value))
                    .transpose()
                    .map_err(json_error)?;
                Ok((key.clone(), saved))
            })
            .collect::<Result<BTreeMap<_, _>, Error>>()?;
        let inherited = inherited
            .with_overrides(
                inputs
                    .iter()
                    .map(|(key, &value)| (key.as_str(), Some(value))),
            )
            .map_err(luma_error)?;
        let admitted = admit(source)?;
        let instance = living_cad_features::derived_id(uuid(document)?, "luma/recipe/1");
        let scope = jobs::scope(&registry.project_id, &instance.to_string());
        let captured = self.capture_luma_jobs(&scope, job_keys)?;
        if !job_keys.is_empty() && self.registry()?.revision != registry.revision {
            return Err(error(
                "Conflict",
                "Project revision changed during job admission",
            ));
        }
        let tasks = jobs::catalog(&luma_cad_jobs::SourcePin {
            document: document.into(),
            revision: head.native_revision,
            definition_digest: jobs::normalized_hash(&head.definition_hash),
        });
        let mut intents = Vec::new();
        let mut evaluate = |bundle: &SourceBundle, mode, native: &dyn Dispatch| {
            let host = luma_cad_jobs::JobsHost::new(native, &tasks, &captured)
                .and_then(luma_cad_jobs::JobsHost::with_catalog)
                .map_err(luma_error)?;
            let result = run_with_host(bundle, mode, &host, 100_000).map_err(source_error)?;
            // The accepted recipe's intents are declared again on a command-driven rerun.
            let mut added = host.into_intents().map_err(jobs::queue_error)?;
            intents.append(&mut added);
            Ok::<_, Error>(result)
        };
        let (exact_source, bundle, values) = if command {
            let previous = previous.ok_or_else(|| {
                error("MissingAcceptedSource", "Part has no accepted Luma recipe")
            })?;
            let host = CommandHost::new(document.into(), inherited)
                .map_err(luma_error)?
                .with_read_only(inputs.keys().cloned().collect());
            evaluate(&admitted, Mode::Command, &host)?;
            (
                &*previous.source,
                admit(&previous.source)?,
                host.finish().map_err(luma_error)?,
            )
        } else {
            (source, admitted, inherited)
        };
        let host = GeometryHost::new(values).map_err(luma_error)?;
        let execution = evaluate(&bundle, Mode::Recipe, &host)?;
        let plan = host
            .finish(&execution.outputs, bundle.source_digest())
            .map_err(luma_error)?;
        if inputs
            .keys()
            .any(|key| !plan.parameters.parameters.entries().contains_key(key))
        {
            return Err(error(
                "ReferencedEntityRemoval",
                "Recipe removed a bound parameter",
            ));
        }
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
        let mut retired_features = BTreeSet::new();
        if let Some(previous) = previous {
            retired_parameters.extend(previous.retired_parameters.iter().cloned());
            retired_parameters.extend(
                previous
                    .parameter_ids
                    .keys()
                    .filter(|key| !parameter_ids.contains_key(*key))
                    .cloned(),
            );
            retired_features.extend(previous.retired_features.iter().cloned());
            let removed = previous
                .features
                .iter()
                .filter(|(key, _)| !features.contains_key(*key))
                .map(|(key, feature)| (key.clone(), feature.id.as_str()))
                .collect::<BTreeMap<_, _>>();
            let removed_ids = removed.values().copied().collect();
            let new_root = &features[&plan.outputs["solid"]].id;
            if !removed.is_empty() || new_root != &previous.public_output {
                reject_referenced_replacement(
                    &registry,
                    document,
                    &removed_ids,
                    (new_root != &previous.public_output)
                        .then_some(previous.public_output.as_str()),
                    &Cancellation::from_shared_flag(ctx.cancellation_flag()),
                )?;
            }
            retired_features.extend(removed.into_keys());
            if features
                .keys()
                .chain(parameter_ids.keys())
                .any(|key| retired_parameters.contains(key) || retired_features.contains(key))
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
        let mut pin = source_pin(registry, &model.pin);
        pin.output_ref = output;
        let mut record = RecipeRecord {
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
            retired_features,
            detached: false,
            accepted_revision: previous.map_or(0, |record| record.accepted_revision),
            bindings: {
                let mut sorted = bindings.to_vec();
                sorted.sort_by(|a, b| a.target.cmp(&b.target));
                sorted
            },
            binding_base,
            historical_outputs: previous
                .map(|record| record.historical_outputs.clone())
                .unwrap_or_default(),
            view: view_requested.then(|| pin.clone()),
        };
        let changed = match previous {
            Some(previous) => {
                changed_model
                    || previous.source_digest != record.source_digest
                    || previous.parameter_json != record.parameter_json
                    || previous.view != record.view
                    || previous.bindings != record.bindings
                    || previous.binding_base != record.binding_base
            }
            None => true,
        };
        if changed {
            record.accepted_revision = registry
                .revision
                .checked_add(1)
                .ok_or_else(|| error("ResourceLimit", "Project revision exhausted"))?;
            if let Some(previous) = previous {
                record.historical_outputs.insert(
                    previous.accepted_revision,
                    serde_json::to_string(&bindings::published(&previous.parameters()?))
                        .map_err(json_error)?,
                );
            }
        }
        Ok((
            PartChange {
                document: document.into(),
                recipe: record,
                snapshot: native,
                queued: jobs::QueueRequest::prepare(&pin, intents)?,
                changed,
            },
            pin,
        ))
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
            store.commit_with(
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
                    let mut primary_rebuilt = false;
                    for change in &candidate.parts {
                        let entry = registry
                            .documents
                            .get_mut(&change.document)
                            .ok_or_else(|| error("MissingDocument", "Part document is absent"))?;
                        if let Some(ref native) = change.snapshot {
                            let part = registry
                                .parts
                                .get_mut(&change.document)
                                .ok_or_else(Error::internal)?;
                            part.snapshots.push(native.clone());
                            part.timeline = None;
                            entry.revision = native.0;
                            primary_rebuilt |= change.document == candidate.document;
                        }
                        if change.changed {
                            registry
                                .luma_recipes
                                .insert(change.document.clone(), change.recipe.clone());
                        }
                    }
                    if let Some(source) = &candidate.command_source {
                        registry
                            .luma_commands
                            .entry(candidate.document.clone())
                            .or_default()
                            .insert(operation_id.to_owned(), source.clone());
                    }
                    // Core advances the envelope once when the primary Part changes.
                    if !primary_rebuilt
                        && (candidate.command_source.is_some()
                            || candidate.parts.iter().any(|change| change.changed))
                    {
                        registry.revision = registry
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| error("ResourceLimit", "Project revision exhausted"))?;
                    }
                    Ok(registry.documents[&candidate.document].revision)
                },
                |tx, receipt| {
                    for change in &candidate.parts {
                        if let Some(request) = &change.queued {
                            let scope = jobs::scope(&candidate.project, &change.recipe.instance);
                            let deliveries = request.admit(tx, &scope)?;
                            if change.document == candidate.document {
                                receipt.luma_jobs = deliveries;
                            } else {
                                receipt
                                    .luma_dependent_jobs
                                    .insert(change.document.clone(), deliveries);
                            }
                        }
                    }
                    Ok(())
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
fn luma_error(err: luma::host_api::Error) -> Error {
    error(err.code, &err.message)
}
fn overlay_error(err: hir_transaction::OverlayError) -> Error {
    living_hir_facade::commit_error(hir_transaction::CommitError::Overlay(err))
}

fn reject_referenced_replacement(
    registry: &Registry,
    document: &str,
    removed: &BTreeSet<&str>,
    replaced_output: Option<&str>,
    cancel: &Cancellation,
) -> Result<(), Error> {
    for assembly in registry.assemblies.keys() {
        let assembly = read_assembly(registry, assembly, cancel)?;
        let referenced: BTreeSet<_> = assembly
            .definitions
            .iter()
            .flat_map(|definition| &definition.instances)
            .filter_map(|instance| match instance {
                Instance::Part(part) => Some(part.part.as_str()),
                Instance::Subassembly(_) => None,
            })
            .collect();
        if assembly.parts.iter().any(|part| {
            referenced.contains(part.reference_id.as_str())
                && part.document_id == document
                && (removed.contains(part.output_ref.entity_id.as_str())
                    || replaced_output == Some(part.output_ref.entity_id.as_str()))
        }) {
            return Err(error(
                "ReferencedEntityRemoval",
                "A project assembly refers to the removed feature or replaced output",
            ));
        }
    }
    Ok(())
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

    #[test]
    fn luma_cad1_recipe_preserves_referenced_output_and_tombstoned_keys() {
        let root = super::super::super::tests::Directory::new();
        let mut store = ProjectStore::create(&root.0.join("managed.db"), PROJECT, [9; 32]).unwrap();
        let ctx = PreparationContext::default();
        let assembly = "00000000-0000-4000-8000-000000000003";
        let instance = "00000000-0000-4000-8000-000000000004";
        store
            .create_document(
                &CreateDocument {
                    request_id: operation(1),
                    document_id: PART.into(),
                    expected_project_revision: 0,
                    kind: DocumentKind::Part,
                    name: "Plate".into(),
                },
                &ctx,
            )
            .unwrap();
        let initial = store.prepare_luma_recipe(PART, 1, RECIPE, &ctx).unwrap();
        let accepted = store
            .apply_luma(&initial.token, &operation(2), &ctx)
            .unwrap();
        let source = store.luma_view(&accepted).unwrap().unwrap();

        let mut smaller = admit(RECIPE).unwrap();
        let text = smaller.files.get_mut(&smaller.entry).unwrap();
        text.truncate(text.find("part = cad.through_hole").unwrap());
        text.push_str("cad.output(\"solid\", left)\n");
        let source_digest = smaller.source_digest();
        let smaller = json!({"bundle_version":1,"entry":smaller.entry,
            "files":smaller.files,"source_digest":source_digest})
        .to_string();

        let reference_free = store.prepare_luma_recipe(PART, 2, &smaller, &ctx).unwrap();
        let created = store
            .create_document(
                &CreateDocument {
                    request_id: operation(3),
                    document_id: assembly.into(),
                    expected_project_revision: 2,
                    kind: DocumentKind::Assembly,
                    name: "Assembly".into(),
                },
                &ctx,
            )
            .unwrap();
        let added = store
            .apply_document(
                &DocumentCommand {
                    request_id: operation(4),
                    document_id: assembly.into(),
                    expected_project_revision: created.project_revision,
                    expected_document_revision: created.document_revision,
                    action: DocumentAction::AddExistingPart {
                        instance_id: instance.into(),
                        source,
                        local: LocalTransform::default(),
                    },
                },
                &root.0,
                &ctx,
            )
            .unwrap();
        assert_eq!(
            store
                .apply_luma(&reference_free.token, &operation(5), &ctx)
                .unwrap_err()
                .code
                .as_ref(),
            "Conflict"
        );
        let blocked = store
            .prepare_luma_recipe(PART, added.project_revision, &smaller, &ctx)
            .unwrap_err();
        assert_eq!(blocked.code.as_ref(), "ReferencedEntityRemoval");
        let freed = store
            .apply_document(
                &DocumentCommand {
                    request_id: operation(6),
                    document_id: assembly.into(),
                    expected_project_revision: added.project_revision,
                    expected_document_revision: added.document_revision,
                    action: DocumentAction::RemoveOccurrence {
                        instance_id: instance.into(),
                    },
                },
                &root.0,
                &ctx,
            )
            .unwrap();
        let next = store
            .prepare_luma_recipe(PART, freed.project_revision, &smaller, &ctx)
            .unwrap();
        let removed = store.apply_luma(&next.token, &operation(7), &ctx).unwrap();
        assert_eq!(
            store
                .prepare_luma_recipe(PART, removed.project_revision, RECIPE, &ctx)
                .unwrap_err()
                .code
                .as_ref(),
            "StableKeyMigrationRequired"
        );
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
