    /// Resolve analytic drawing sources from one accepted Part revision.
    /// Callers that publish a derived sheet must recheck the returned head.
    pub fn drawing_plate_sources(
        &self,
        document: &str,
        features: &[hir_types::EntityId],
        context: &PreparationContext,
    ) -> Result<(PartHead, Vec<living_hir_facade::DrawingPlateSource>), Error> {
        let registry = self.registry()?;
        let (head, part, _) = part_head(&registry, document)?;
        let (facade, store) = restore_part(part, document, head.native_revision, context)?;
        let snapshot = store.snapshot().map_err(living_hir_facade::commit_error)?;
        let cancel = Cancellation::from_shared_flag(context.cancellation_flag());
        let sources = features
            .iter()
            .map(|feature| {
                let output = living_hir_facade::WireOutputRef {
                    entity_id: feature.to_string(),
                    port: "solid".into(),
                };
                facade.drawing_plate_source(&snapshot, &output, &cancel)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((head, sources))
    }
