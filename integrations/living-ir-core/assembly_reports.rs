//! Read-only BOM for an accepted native assembly and its historical Part sources.
use super::*;

impl ProjectStore {
    pub fn assembly_bom(
        &self,
        document: &str,
        configuration: Option<&str>,
        context: &PreparationContext,
    ) -> Result<Value, Error> {
        let registry = self.registry()?;
        let cancel = Cancellation::from_shared_flag(context.cancellation_flag());
        let assembly = read_assembly(&registry, document, &cancel)?;
        let query = assembly
            .query(configuration, &HistoricalResolver(&registry), &cancel)
            .map_err(assembly_error)?;

        let mut counts = BTreeMap::new();
        let mut occurrences = Vec::new();
        for occurrence in &query.occurrences {
            let Some(pin) = &occurrence.part else {
                continue;
            };
            // Core assigns a reference_id per occurrence. Group by the complete source instead.
            let key = (
                &pin.document_id,
                pin.native_revision,
                &pin.definition_hash,
                &pin.output_ref.entity_id,
                &pin.output_ref.port,
                &pin.semantic_key_v2,
            );
            counts.entry(key).or_insert((pin, 0_u32)).1 += 1;
            let bounds = query
                .bounds(&occurrence.path, &cancel)
                .map_err(assembly_error)?
                .map(|bounds| {
                    json!({
                        "kind": "ConservativeAabb",
                        "min_m": bounds.min.xyz(),
                        "max_m": bounds.max.xyz()
                    })
                });
            occurrences.push(json!({
                "path": occurrence.path,
                "source_ref": pin.reference_id,
                "source": part_source(pin),
                "placement": {
                    "translation_m": occurrence.world.translation_m(),
                    "rotation_xyzw": occurrence.world.rotation_xyzw()
                },
                "bounds": bounds
            }));
        }
        let rows: Vec<_> = counts
            .into_values()
            .map(|(pin, count)| {
                json!({
                    "source": part_source(pin),
                    "quantity": count,
                    "mass_kg": null
                })
            })
            .collect();
        let digest = sha2::digest::Output::<Sha256>::from(query.source.sha256);
        Ok(json!({
            "schema": "luma.cad.assembly-bom/1",
            "assembly": {
                "project": registry.project_id, "document": document,
                "revision": query.source.revision, "digest": format!("sha256:{digest:x}"),
                "configuration": query.configuration
            },
            "rows": rows,
            "occurrences": occurrences,
            "mass": {"status": "UnknownDensity", "total_kg": null}
        }))
    }
}

fn part_source(pin: &PartDefinitionRef) -> Value {
    json!({
        "document": pin.document_id, "revision": pin.native_revision,
        "definition_hash": pin.definition_hash,
        "output": {"entity": pin.output_ref.entity_id, "port": pin.output_ref.port},
        "semantic_key_v2": pin.semantic_key_v2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
    const PART: &str = "00000000-0000-4000-8000-000000000002";
    const ASSEMBLY: &str = "00000000-0000-4000-8000-000000000003";
    const GROUP: &str = "00000000-0000-4000-8000-000000000004";
    const FIRST: &str = "00000000-0000-4000-8000-000000000005";
    const SECOND: &str = "00000000-0000-4000-8000-000000000006";

    #[test]
    fn cad5_bom_counts_nested_occurrences_and_explicit_repins() {
        let directory = crate::checkpoint::tests::Directory::new();
        let database = directory.0.join("assembly.db");
        let key = [19; 32];
        let mut store = ProjectStore::create(&database, PROJECT, key).unwrap();
        let context = PreparationContext::default();
        for (revision, (document, kind)) in [
            (PART, DocumentKind::Part),
            (ASSEMBLY, DocumentKind::Assembly),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .create_document(
                    &CreateDocument {
                        request_id: format!("{:032x}", revision + 1),
                        document_id: document.into(),
                        expected_project_revision: revision as u64,
                        kind,
                        name: document.into(),
                    },
                    &context,
                )
                .unwrap();
        }
        let edit = |store: &mut ProjectStore, document: &str, action| {
            let registry = store.registry().unwrap();
            store
                .apply_document(
                    &DocumentCommand {
                        request_id: format!("{:032x}", registry.revision + 1),
                        document_id: document.into(),
                        expected_project_revision: registry.revision,
                        expected_document_revision: registry.documents[document].revision,
                        action,
                    },
                    &directory.0,
                    &context,
                )
                .unwrap();
        };
        edit(
            &mut store,
            PART,
            DocumentAction::Editor(json!({"op":"add","kind":"box"})),
        );
        let original = store.source_pin(PART, 2).unwrap();
        let translation = |x| LocalTransform {
            translation_m: [x, 0.0, 0.0],
            ..Default::default()
        };
        edit(
            &mut store,
            ASSEMBLY,
            DocumentAction::AddAssemblyGroup {
                instance_id: GROUP.into(),
                local: translation(10.0),
            },
        );
        for (id, x) in [(FIRST, 1.0), (SECOND, 2.0)] {
            edit(
                &mut store,
                ASSEMBLY,
                DocumentAction::AddExistingPart {
                    instance_id: id.into(),
                    source: original.clone(),
                    local: translation(x),
                },
            );
        }
        edit(
            &mut store,
            ASSEMBLY,
            DocumentAction::ReparentOccurrence {
                instance_id: FIRST.into(),
                parent: Some(GROUP.into()),
            },
        );

        let before = store.assembly_bom(ASSEMBLY, None, &context).unwrap();
        assert_eq!(before["rows"].as_array().unwrap().len(), 1);
        assert_eq!(before["rows"][0]["quantity"], 2);
        assert_eq!(
            before["rows"][0]["source"]["definition_hash"],
            original.definition_hash
        );
        assert_eq!(
            before["mass"],
            json!({"status":"UnknownDensity","total_kg":null})
        );
        assert_eq!(before["occurrences"][0]["path"], json!([GROUP, FIRST]));
        assert_eq!(
            before["occurrences"][0]["placement"]["translation_m"],
            json!([11.0, 0.0, 0.0])
        );
        assert_eq!(
            before["occurrences"][0]["bounds"]["kind"],
            "ConservativeAabb"
        );
        assert_eq!(before["occurrences"][1]["path"], json!([SECOND]));
        assert!(store
            .assembly_bom(ASSEMBLY, Some("unknown"), &context)
            .is_err());

        edit(
            &mut store,
            PART,
            DocumentAction::Editor(json!({
                "op":"properties","node":original.output_ref.entity_id,"values":{"depth":19.0}
            })),
        );
        let replacement = store.source_pin(PART, 3).unwrap();
        drop(store);
        let mut store = ProjectStore::open(&database, PROJECT, key).unwrap();
        assert_eq!(
            store.assembly_bom(ASSEMBLY, None, &context).unwrap(),
            before
        );

        edit(
            &mut store,
            ASSEMBLY,
            DocumentAction::UpdateReference {
                instance_id: FIRST.into(),
                expected: original.clone(),
                replacement: replacement.clone(),
            },
        );
        let after = store.assembly_bom(ASSEMBLY, None, &context).unwrap();
        assert_ne!(before["assembly"]["digest"], after["assembly"]["digest"]);
        assert_eq!(after["rows"].as_array().unwrap().len(), 2);
        assert_eq!(after["rows"][0]["quantity"], 1);
        assert_eq!(
            after["rows"][0]["source"]["definition_hash"],
            original.definition_hash
        );
        assert_eq!(after["rows"][1]["quantity"], 1);
        assert_eq!(
            after["rows"][1]["source"]["definition_hash"],
            replacement.definition_hash
        );
        assert_eq!(
            after["occurrences"][0]["path"],
            before["occurrences"][0]["path"]
        );
        assert_eq!(
            after["occurrences"][0]["source"]["definition_hash"],
            replacement.definition_hash
        );
        assert_eq!(after["occurrences"][1], before["occurrences"][1]);
    }
}
