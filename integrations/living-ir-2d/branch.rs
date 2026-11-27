//! Prepared branch files are authority tokens, never persisted previews.
use super::super::{luma_cad::PROFILE, part_head, seal, unseal, ProjectStore, Receipt};
use super::{
    check_scope, provider_error, reject_recipe_edit, unregistered, CoreReads, Embedded2dV1,
};
use crate::{checkpoint::unix_seconds, error, Error};
use hir_transaction::{PreparationContext, PreparationStage};
use hir_types::Revision;
use lir2d_runtime::{CommandEnvelope, Runtime};
use lir2d_types::{Digest, ObjectId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TOKEN_DOMAIN: &[u8] = b"luma-embedded-2d-branch/1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BranchToken {
    integration: String,
    expected_project: u64,
    batch: CommandEnvelope,
    candidate_digest: Digest,
    branch_token: ObjectId,
    expires_at: i64,
}

impl ProjectStore {
    pub fn propose_2d_branch(
        &self,
        part: &str,
        expected_project: u64,
        bytes: &[u8],
        context: &PreparationContext,
    ) -> Result<(Value, Vec<u8>), Error> {
        let batch =
            CommandEnvelope::from_json(bytes).map_err(|problem| provider_error(&problem))?;
        if batch.project_id.as_str() != self.project_id || batch.document_id.as_str() != part {
            return Err(error(
                "InvalidPayload",
                "Branch proposal does not target this project and Part",
            ));
        }
        let registry = self.registry()?;
        if registry.revision != expected_project {
            return Err(error("Conflict", "Project revision changed"));
        }
        let (head, _, _) = part_head(&registry, part)?;
        reject_recipe_edit(&registry, part)?;
        let state = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
        if state.revision != batch.expected_revision.0 {
            return Err(error("Conflict", "Embedded 2D revision changed"));
        }
        let revision = Revision(head.native_revision);
        context
            .check(revision, PreparationStage::Validation)
            .map_err(living_hir_facade::overlay_error)?;
        let base = state.snapshot(part)?;
        let mut reads = CoreReads::new(&registry, context);
        reads.snapshot(&base, false)?;
        reads.commands(&batch.commands)?;
        let (preview, proposal) = Runtime::propose_embedded_branch(
            batch.project_id.clone(),
            base,
            &batch,
            &context.cancellation_flag(),
        )
        .map_err(|problem| provider_error(&problem))?;
        check_scope(part, &proposal)?;
        let read_set = reads.snapshot(&proposal, true)?;
        context
            .check(revision, PreparationStage::Sealing)
            .map_err(living_hir_facade::overlay_error)?;
        let branch = preview
            .branch_proposal
            .as_ref()
            .ok_or_else(|| error("InternalError", "Missing provider branch proposal"))?;
        let token = BranchToken {
            integration: PROFILE.into(),
            expected_project,
            batch,
            candidate_digest: preview.candidate_digest.clone(),
            branch_token: branch.branch_token.clone(),
            expires_at: preview.expires_at.unix_timestamp(),
        };
        Ok((
            json!({
                "project_revision":expected_project.to_string(),"embedded_revision":preview.base_revision,
                "document_id":part, "branch":branch, "candidate_digest":token.candidate_digest,
                "sketch_solutions":preview.sketch_solutions,"core_read_set":read_set,"expires_at":preview.expires_at.unix_timestamp().to_string(),
            }),
            seal(&self.key, TOKEN_DOMAIN, &token)?,
        ))
    }

    pub fn accept_2d_branch(
        &mut self,
        bytes: &[u8],
        operation: &str,
        context: &PreparationContext,
    ) -> Result<Receipt, Error> {
        let token: BranchToken = unseal(&self.key, TOKEN_DOMAIN, bytes)?;
        if token.batch.project_id.as_str() != self.project_id || token.integration != PROFILE {
            return Err(error(
                "PreparedContextChanged",
                "Branch proposal belongs to another project or integration build",
            ));
        }
        let part = token.batch.document_id.as_str();
        // The authority looks up an existing receipt before checking expiry or head.
        self.commit(
            (part, operation, token.expected_project),
            &("accept-embedded-2d-branch/1", &token),
            context,
            |registry| {
                if unix_seconds()? >= token.expires_at {
                    return Err(error("PreparedExpired", "Prepare a new branch proposal"));
                }
                let (head, _, _) = part_head(registry, part)?;
                reject_recipe_edit(registry, part)?;
                let state = registry.embedded_2d.get(part).ok_or_else(unregistered)?;
                if state.revision != token.batch.expected_revision.0 {
                    return Err(error("Conflict", "Embedded 2D revision changed"));
                }
                let revision = Revision(head.native_revision);
                context
                    .check(revision, PreparationStage::Validation)
                    .map_err(living_hir_facade::overlay_error)?;
                let base = state.snapshot(part)?;
                let mut reads = CoreReads::new(registry, context);
                reads.snapshot(&base, false)?;
                reads.commands(&token.batch.commands)?;
                let snapshot = Runtime::accept_embedded_branch(
                    token.batch.project_id.clone(),
                    base,
                    &token.batch,
                    &token.candidate_digest,
                    &token.branch_token,
                    &context.cancellation_flag(),
                )
                .map_err(|problem| provider_error(&problem))?;
                reads.snapshot(&snapshot, true)?;
                let accepted = Embedded2dV1::successor(part, state.revision, &snapshot)?;
                context
                    .check(revision, PreparationStage::Sealing)
                    .map_err(living_hir_facade::overlay_error)?;
                registry.embedded_2d.insert(part.to_owned(), accepted);
                Ok(head.native_revision)
            },
        )
    }
}
