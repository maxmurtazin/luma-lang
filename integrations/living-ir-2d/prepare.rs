//! A transient evaluator. The Core host owns authorization, publication and receipts.
use super::*;

impl Runtime {
    pub fn prepare_embedded(
        project: ObjectId,
        snapshot: DocumentSnapshot,
        request: &CommandEnvelope,
        cancellation: &AtomicBool,
    ) -> Result<DocumentSnapshot, ErrorEnvelope> {
        if snapshot.is_standalone() || snapshot.dependency_count() != 0 {
            return Err(unsupported(
                "expected Core-owned state without external dependencies",
            ));
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
        let runtime = Self::new_local(project, snapshot, FixtureSources::default())?;
        let preview = runtime.preview_with_cancel(request, cancellation)?;
        if preview.branch_proposal.is_some() {
            return Err(ErrorEnvelope::new(
                ErrorCode::BranchChangeRequired,
                "sketch.branch_confirmation_required",
                "preview",
                "the accepted branch must be retained by the Core host",
            ));
        }
        if !preview.source_read_set.is_empty() {
            return Err(unsupported("embedded source reads require a Core read set"));
        }
        runtime.preview_snapshot(&preview.preview_id)
    }
}
