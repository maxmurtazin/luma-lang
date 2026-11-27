//! Prepare and accept a fixed-topology Sketch recipe through the Core writer.
use super::super::{
    luma_cad::{empty_part_graph, PROFILE},
    part_head, seal, unseal, ProjectStore, Receipt, Registry,
};
use super::{provider_error, stage_extrusion, unregistered, Embedded2dV1};
use crate::{checkpoint::unix_seconds, error, json_error, Error};
use hir_transaction::{PreparationContext, PreparationStage};
use hir_types::Revision;
use lir2d_runtime::{Command, CommandEnvelope, Runtime, SketchConstraintPatch};
use lir2d_types::{Finite, ObjectId, SketchRelation};
use luma::{
    cad_engine::ParameterSnapshot,
    host::{Mode, SourceBundle, WireSourceBundle},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const DOMAIN: &[u8] = b"luma-sketch-recipe/1";
const TTL_SECONDS: i64 = 300;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AcceptedRecipe {
    source_digest: String,
    source: String,
    topology: String,
    profile_id: String,
    feature_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parameters: Option<String>,
}

impl AcceptedRecipe {
    fn parameters(&self) -> Result<ParameterSnapshot, Error> {
        match &self.parameters {
            Some(json) => {
                let parameters = serde_json::from_str(json).map_err(json_error)?;
                ParameterSnapshot::new(parameters).map_err(|problem| {
                    error(
                        "CheckpointCorrupt",
                        &format!("Invalid Sketch parameters: {problem:?}"),
                    )
                })
            }
            // Earlier snapshots retained the source and could not carry overrides.
            None => Ok(luma::cad::prepare_sketch_recipe(
                &read_source(self.source.as_bytes())?,
                100_000,
            )
            .map_err(plan_error)?
            .parameters),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    project: String,
    part: String,
    integration: String,
    expected_project: u64,
    expected_2d: u64,
    #[serde(default = "recipe_mode")]
    mode: Mode,
    source: String,
    candidate_digest: String,
    expires_at: i64,
}

fn recipe_mode() -> Mode {
    Mode::Recipe
}

impl ProjectStore {
    pub fn preview_sketch_recipe(
        &self,
        part: &str,
        expected_project: u64,
        expected_2d: u64,
        source: &[u8],
        context: &PreparationContext,
    ) -> Result<(Value, Vec<u8>), Error> {
        self.preview_sketch_edit(
            part,
            expected_project,
            expected_2d,
            source,
            Mode::Recipe,
            context,
        )
    }

    pub fn preview_sketch_command(
        &self,
        part: &str,
        expected_project: u64,
        expected_2d: u64,
        source: &[u8],
        context: &PreparationContext,
    ) -> Result<(Value, Vec<u8>), Error> {
        self.preview_sketch_edit(
            part,
            expected_project,
            expected_2d,
            source,
            Mode::Command,
            context,
        )
    }

    fn preview_sketch_edit(
        &self,
        part: &str,
        expected_project: u64,
        expected_2d: u64,
        source: &[u8],
        mode: Mode,
        context: &PreparationContext,
    ) -> Result<(Value, Vec<u8>), Error> {
        let mut registry = self.registry()?;
        if registry.revision != expected_project {
            return Err(error("Conflict", "Project revision changed"));
        }
        let bundle = read_source(source)?;
        let source_digest = bundle.source_digest();
        let source = serde_json::to_string(&json!({
            "bundle_version":1,"entry":bundle.entry,"files":bundle.files,"source_digest":source_digest,
        }))
        .map_err(json_error)?;
        let (native, digest) = stage(&mut registry, part, expected_2d, &source, mode, context)?;
        let token = Prepared {
            project: self.project_id.clone(),
            part: part.into(),
            integration: PROFILE.into(),
            expected_project,
            expected_2d,
            mode,
            source,
            candidate_digest: digest,
            expires_at: unix_seconds()? + TTL_SECONDS,
        };
        let record = &registry.embedded_2d[part];
        let recipe = record.recipe.as_ref().ok_or_else(Error::internal)?;
        let mut preview = json!({
                "project_revision":expected_project.to_string(),
                "candidate_part_revision":native.to_string(),
                "candidate_embedded_revision":record.revision.to_string(),
                "candidate_changed":expected_2d != record.revision,
                "source_digest":recipe.source_digest,
                "candidate_digest":token.candidate_digest,
                "expires_at":token.expires_at.to_string(),
        });
        if mode == Mode::Command {
            preview["command_digest"] = json!(source_digest);
            preview["parameters"] =
                serde_json::to_value(recipe.parameters()?).map_err(json_error)?;
        }
        Ok((preview, seal(&self.key, DOMAIN, &token)?))
    }

    pub fn apply_sketch_recipe(
        &mut self,
        token: &[u8],
        operation: &str,
        context: &PreparationContext,
    ) -> Result<Receipt, Error> {
        let token: Prepared = unseal(&self.key, DOMAIN, token)?;
        if token.project != self.project_id || token.integration != PROFILE {
            return Err(error(
                "PreparedContextChanged",
                "Sketch recipe belongs to another project or integration build",
            ));
        }
        self.commit(
            (&token.part, operation, token.expected_project),
            &("accept-sketch-recipe/1", &token),
            context,
            |registry| {
                if unix_seconds()? >= token.expires_at {
                    return Err(error("PreparedExpired", "Prepare a new Sketch recipe"));
                }
                let (revision, digest) = stage(
                    registry,
                    &token.part,
                    token.expected_2d,
                    &token.source,
                    token.mode,
                    context,
                )?;
                if digest != token.candidate_digest {
                    return Err(error(
                        "Conflict",
                        "Recomputed Sketch recipe differs from preview",
                    ));
                }
                Ok(revision)
            },
        )
    }

    pub fn sketch_recipe_info(&self, part: &str) -> Result<Value, Error> {
        let registry = self.registry()?;
        let (head, _, _) = part_head(&registry, part)?;
        let record = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
        let recipe = record
            .recipe
            .as_ref()
            .ok_or_else(|| error("MissingAcceptedSource", "Part has no Sketch recipe"))?;
        let bundle: Value = serde_json::from_str(&recipe.source).map_err(json_error)?;
        Ok(json!({
            "project_revision":registry.revision.to_string(),
            "part_revision":head.native_revision.to_string(),
            "embedded_revision":record.revision.to_string(),
            "profile_id":recipe.profile_id,"feature_id":recipe.feature_id,
            "source_digest":recipe.source_digest,"bundle":bundle,"parameters":recipe.parameters()?,
        }))
    }
}

fn stage(
    registry: &mut Registry,
    part: &str,
    expected_2d: u64,
    source: &str,
    mode: Mode,
    context: &PreparationContext,
) -> Result<(u64, String), Error> {
    let (head, _, native_bytes) = part_head(registry, part)?;
    let record = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
    if record.revision != expected_2d {
        return Err(error("Conflict", "Embedded 2D revision changed"));
    }
    let revision = Revision(head.native_revision);
    context
        .check(revision, PreparationStage::Validation)
        .map_err(living_hir_facade::overlay_error)?;
    let bundle = read_source(source.as_bytes())?;
    let base = record.snapshot(part)?;
    if base
        .parameters()
        .values()
        .any(|parameter| parameter.core_source().is_some())
    {
        return Err(error(
            "UnsupportedSketchRecipe",
            "Sketch recipe requires 2D state without external parameter bindings",
        ));
    }
    let prior = record.recipe.as_ref();
    let (source, bundle, inherited) = match mode {
        Mode::Recipe => {
            if prior.is_some_and(|recipe| recipe.source_digest == bundle.source_digest()) {
                return state_digest(registry, part);
            }
            let inherited = prior
                .map(AcceptedRecipe::parameters)
                .transpose()?
                .unwrap_or_default();
            (source, bundle, inherited)
        }
        Mode::Command => {
            let prior =
                prior.ok_or_else(|| error("MissingAcceptedSource", "Part has no Sketch recipe"))?;
            let inherited = prior.parameters()?;
            let edited = luma::cad::prepare_command(&bundle, part.into(), &inherited, 100_000)
                .map_err(|problem| error("Embedded2dCommandRejected", &format!("{problem:?}")))?;
            if edited == inherited {
                return state_digest(registry, part);
            }
            (
                prior.source.as_str(),
                read_source(prior.source.as_bytes())?,
                edited,
            )
        }
    };
    if record.recipe.is_none()
        && !(base.sketches().is_empty()
            && base.workplanes().is_empty()
            && base.profiles().is_empty()
            && base.extrusions().is_empty()
            && base.drafts().is_empty()
            && base.sheets().is_empty())
    {
        return Err(error(
            "ManagedEntityConflict",
            "Sketch recipe requires an empty 2D geometry state",
        ));
    }
    if record.recipe.is_none()
        && !empty_part_graph(&serde_json::from_str(native_bytes).map_err(json_error)?)
    {
        return Err(error(
            "ManagedEntityConflict",
            "Sketch recipe requires an empty native Part",
        ));
    }
    let source_digest = bundle.source_digest();
    let plan = luma::cad::prepare_sketch_recipe_with_parameters(&bundle, inherited, 100_000)
        .map_err(plan_error)?;
    let intent = plan.extrusion.ok_or_else(unsupported)?;
    let mut batch = CommandEnvelope::from_json(
        &serde_json::to_vec(&json!({
            "api_version":"1.0", "request_id":"00000000-0000-4000-8000-000000000001",
            "project_id":registry.project_id, "document_id":part, "expected_revision":expected_2d.to_string(),
            "idempotency_key":"luma-sketch-recipe", "commands":plan.commands,
            "execution_policy":{"numeric_profile_id":"mechanical-si-v1","source_policy":"pin_current",
                "solver_policy":"preserve_branch","timeout_ms":5000,"max_memory_bytes":50331648},
        })).map_err(json_error)?,
    ).map_err(|problem| provider_error(&problem))?;
    let (topology, updates) = reconcile(&batch.commands)?;
    let recipe = AcceptedRecipe {
        source_digest,
        source: source.into(),
        topology,
        profile_id: intent.profile_id.clone(),
        feature_id: intent.feature_id.clone(),
        parameters: Some(serde_json::to_string(&plan.parameters).map_err(json_error)?),
    };
    let rebuild = if let Some(prior) = &record.recipe {
        if prior.topology != recipe.topology
            || prior.profile_id != recipe.profile_id
            || prior.feature_id != recipe.feature_id
        {
            return Err(error(
                "SketchRecipeTopologyChanged",
                "Keep Sketch, Profile, feature and curve identities",
            ));
        }
        batch.commands = updates;
        true
    } else {
        false
    };
    let project = batch.project_id.clone();
    let candidate = Runtime::prepare_embedded(project, base, &batch, &context.cancellation_flag())
        .map_err(|problem| provider_error(&problem))?;
    let mut staged = Embedded2dV1::successor(part, expected_2d, &candidate)?;
    staged.recipe = Some(recipe);
    context
        .check(revision, PreparationStage::Sealing)
        .map_err(living_hir_facade::overlay_error)?;
    stage_extrusion(registry, &head, &candidate, &intent, rebuild, context)?;
    registry.embedded_2d.insert(part.into(), staged);
    state_digest(registry, part)
}

fn reconcile(commands: &[Command]) -> Result<(String, Vec<Command>), Error> {
    let mut points = BTreeMap::<&ObjectId, [Finite; 2]>::new();
    let mut fixed = BTreeMap::new();
    let mut topology = Vec::with_capacity(commands.len());
    let mut updates = Vec::new();
    let mut sketches = 0;
    let mut profiles = 0;
    for command in commands {
        let mut identity = serde_json::to_value(command).map_err(json_error)?;
        match command {
            Command::CreateSketch { .. } => sketches += 1,
            Command::CreateProfile { .. } => profiles += 1,
            Command::CreateWorkplane { .. }
            | Command::CreateSketchSegment { .. }
            | Command::CreateSketchCurve { .. } => {}
            Command::CreateSketchPoints { variables, .. } => {
                for (point, item) in variables.iter().zip(
                    identity["variables"]
                        .as_array_mut()
                        .ok_or_else(Error::internal)?,
                ) {
                    points.insert(&point.id, point.initial_m);
                    item.as_object_mut()
                        .ok_or_else(Error::internal)?
                        .remove("initial_m");
                }
            }
            Command::CreateConstraint { constraint, .. } => {
                let SketchRelation::FixPoint {
                    point_id,
                    position_m,
                } = &constraint.relation
                else {
                    return Err(unsupported());
                };
                if fixed.insert(point_id, *position_m).is_some() {
                    return Err(unsupported());
                }
                identity["constraint"]["relation"]
                    .as_object_mut()
                    .ok_or_else(Error::internal)?
                    .remove("position_m");
                updates.push(Command::UpdateConstraint {
                    constraint_id: constraint.id.clone(),
                    patch: SketchConstraintPatch {
                        enabled: None,
                        relation: Some(constraint.relation.clone()),
                    },
                });
            }
            _ => return Err(unsupported()),
        }
        topology.push(identity.to_string());
    }
    if sketches != 1 || profiles != 1 || points.is_empty() || fixed != points {
        return Err(unsupported());
    }
    topology.sort_unstable();
    Ok((
        digest(&serde_json::to_vec(&topology).map_err(json_error)?),
        updates,
    ))
}

fn state_digest(registry: &Registry, part: &str) -> Result<(u64, String), Error> {
    let (head, _, _) = part_head(registry, part)?;
    let state = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
    let bytes = serde_json::to_vec(&(head.native_revision, head.definition_hash, state))
        .map_err(json_error)?;
    Ok((head.native_revision, digest(&bytes)))
}

fn read_source(bytes: &[u8]) -> Result<SourceBundle, Error> {
    serde_json::from_slice::<WireSourceBundle>(bytes)
        .map_err(|_| error("InvalidSourceBundle", "Expected a Luma source bundle"))?
        .admit()
        .map_err(|problem| error("InvalidSourceBundle", &format!("{:?}", problem.diagnostics)))
}

fn plan_error(problem: luma::cad::PreparationError) -> Error {
    error("Embedded2dPlanRejected", &format!("{problem:?}"))
}

fn unsupported() -> Error {
    error(
        "UnsupportedSketchRecipe",
        "Expected one extruded profile with each Sketch point fixed at its declared coordinates",
    )
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
