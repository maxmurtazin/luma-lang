//! A transient evaluator. The Core host owns authorization, publication and receipts.
use super::*;

impl Runtime {
    pub fn prepare_embedded(
        project: ObjectId,
        snapshot: DocumentSnapshot,
        request: &CommandEnvelope,
        sources: FixtureSources,
        cancellation: &AtomicBool,
    ) -> Result<DocumentSnapshot, ErrorEnvelope> {
        if request.commands.iter().any(|item| {
            matches!(
                item,
                Command::ProposeSketchBranch { .. } | Command::AcceptSketchBranch { .. }
            )
        }) {
            return Err(branch_confirmation_required());
        }
        let runtime = Self::embedded_evaluator(project, snapshot, request, sources)?;
        let preview = runtime.preview_with_cancel(request, cancellation)?;
        runtime.preview_snapshot(&preview.preview_id)
    }

    pub fn propose_embedded_branch(
        project: ObjectId,
        snapshot: DocumentSnapshot,
        request: &CommandEnvelope,
        sources: FixtureSources,
        cancellation: &AtomicBool,
    ) -> Result<(Preview, DocumentSnapshot), ErrorEnvelope> {
        let runtime = Self::embedded_evaluator(project, snapshot, request, sources)?;
        let preview = runtime.branch_preview(request, cancellation)?;
        let candidate = runtime.preview_snapshot(&preview.preview_id)?;
        Ok((preview, candidate))
    }

    /// The caller must authenticate the archived proposal and enforce its original expiry.
    pub fn accept_embedded_branch(
        project: ObjectId,
        snapshot: DocumentSnapshot,
        proposal: &CommandEnvelope,
        candidate_digest: &Digest,
        branch_token: &ObjectId,
        sources: FixtureSources,
        cancellation: &AtomicBool,
    ) -> Result<DocumentSnapshot, ErrorEnvelope> {
        let runtime = Self::embedded_evaluator(project, snapshot, proposal, sources)?;
        let preview = runtime.branch_preview(proposal, cancellation)?;
        if &preview.candidate_digest != candidate_digest {
            return Err(ErrorEnvelope::new(
                ErrorCode::SourceRevisionConflict,
                "sketch.branch_proposal_changed",
                "preview",
                "recomputed branch does not match the accepted proposal",
            ));
        }
        let branch = preview
            .branch_proposal
            .ok_or_else(branch_confirmation_required)?;
        // Rebind only this already revalidated proposal in the transient runtime.
        // Normal acceptance still runs the provider's final validation.
        {
            let mut state = runtime.lock()?;
            let saved = state
                .previews
                .get_mut(&proposal.idempotency_key)
                .ok_or_else(branch_confirmation_required)?;
            let saved_branch = saved
                .preview
                .branch_proposal
                .as_mut()
                .ok_or_else(branch_confirmation_required)?;
            saved_branch.branch_token = branch_token.clone();
            saved.preview.preview_id = branch_token.clone();
        }
        let mut acceptance = proposal.clone();
        acceptance.idempotency_key = if proposal.idempotency_key == "luma-branch-accept" {
            "luma-accept-branch"
        } else {
            "luma-branch-accept"
        }
        .into();
        acceptance.commands = vec![Command::AcceptSketchBranch {
            sketch_id: branch.sketch_id,
            branch_token: branch_token.clone(),
        }];
        let accepted = runtime.preview_with_cancel(&acceptance, cancellation)?;
        if accepted.acceptance_status != "ready" || accepted.branch_proposal.is_some() {
            return Err(branch_confirmation_required());
        }
        runtime.preview_snapshot(&accepted.preview_id)
    }

    fn embedded_evaluator(
        project: ObjectId,
        snapshot: DocumentSnapshot,
        request: &CommandEnvelope,
        sources: FixtureSources,
    ) -> Result<Self, ErrorEnvelope> {
        if snapshot.is_standalone() {
            return Err(unsupported("expected Core-owned state"));
        }
        if request
            .commands
            .iter()
            .any(|command| matches!(command, Command::Undo { .. } | Command::Redo { .. }))
        {
            return Err(unsupported(
                "embedded history commands require Core history",
            ));
        }
        Self::new_local(project, snapshot, sources)
    }

    fn branch_preview(
        &self,
        request: &CommandEnvelope,
        cancellation: &AtomicBool,
    ) -> Result<Preview, ErrorEnvelope> {
        if !matches!(
            request.commands.last(),
            Some(Command::ProposeSketchBranch { .. })
        ) {
            return Err(branch_confirmation_required());
        }
        let preview = self.preview_with_cancel(request, cancellation)?;
        if preview.branch_proposal.is_none() || preview.acceptance_status != "branch_proposal" {
            return Err(branch_confirmation_required());
        }
        Ok(preview)
    }
}

fn branch_confirmation_required() -> ErrorEnvelope {
    ErrorEnvelope::new(
        ErrorCode::BranchChangeRequired,
        "sketch.branch_confirmation_required",
        "preview",
        "embedded branch changes require the Core proposal and acceptance commands",
    )
}
