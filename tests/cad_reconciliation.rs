use luma::{
    cad::{prepare_command, prepare_geometry, prepare_geometry_edits},
    cad_engine::{
        AdmittedRecipe, AdmittedRecipes, BindingRevision, ClosureError, GeometryDefinition,
        OwnedKey, OwnedRegion, Ownership, ParamValue, PreparedRecipe, Provenance, RecipeBinding,
        Reconciliation,
    },
    host::{SourceBundle, WireSourceBundle},
    host_api::{Type, Value},
};
use std::collections::{BTreeMap, BTreeSet};

fn recipe() -> SourceBundle {
    serde_json::from_str::<WireSourceBundle>(include_str!("../examples/cad/plate-geometry.json"))
        .unwrap()
        .admit()
        .unwrap()
}

fn width(value: f64) -> Value {
    Value::Opaque {
        type_name: "Quantity<Length>".into(),
        payload: serde_json::Value::String(value.to_string()),
    }
}

fn binding(producer: &str, revision: BindingRevision) -> RecipeBinding {
    RecipeBinding {
        producer: producer.into(),
        output: "width".into(),
        revision,
        expected_type: Type::Opaque("Quantity<Length>".into()),
    }
}

fn head(revision: u64, input: Option<RecipeBinding>) -> AdmittedRecipe {
    AdmittedRecipe {
        revision,
        bindings: input
            .into_iter()
            .map(|input| ("value".into(), input))
            .collect(),
        outputs: [("width".into(), width(0.1))].into(),
    }
}

#[test]
fn closure_runs_follow_current_and_keeps_exact_pins_through_a_command_override() {
    let source = recipe();
    let plan = prepare_geometry(&source, &Default::default(), 1000).unwrap();
    let snapshot = &plan.parameters.parameters;
    let command = serde_json::from_str::<WireSourceBundle>(include_str!(
        "../examples/cad/plate-width-command.json"
    ))
    .unwrap()
    .admit()
    .unwrap();
    let overlay = prepare_command(
        &command,
        "00000000-0000-4000-8000-000000000002".into(),
        snapshot,
        1000,
    )
    .unwrap();

    let mut graph = AdmittedRecipes {
        project_revision: 8,
        current: [
            (
                "a_follow".into(),
                head(
                    2,
                    Some(binding("z_producer", BindingRevision::FollowCurrent)),
                ),
            ),
            (
                "b_pinned".into(),
                head(
                    4,
                    Some(binding("z_producer", BindingRevision::ExactPinned(7))),
                ),
            ),
            ("z_producer".into(), head(7, None)),
        ]
        .into(),
        ..Default::default()
    };

    let prepare = |id: &str, inputs: &BTreeMap<String, &Value>| {
        let plan = if id == "z_producer" {
            prepare_geometry(&source, &overlay, 1000)?
        } else {
            let Value::Opaque { payload, .. } = inputs["value"] else {
                unreachable!()
            };
            let value = payload.as_str().unwrap().parse().unwrap();
            prepare_geometry_edits(
                &source,
                snapshot,
                [("width", Some(ParamValue::Length(value)))],
                1000,
            )?
        };
        let ParamValue::Length(value) = plan.parameters.parameters.entries()["width"].effective()
        else {
            unreachable!()
        };
        Ok::<_, luma::cad::PreparationError>(PreparedRecipe {
            change: plan,
            outputs: [("width".into(), width(value))].into(),
        })
    };

    let edit_producer = ["z_producer".into()].into();
    let batch = graph.prepare(&edit_producer, prepare).unwrap();
    assert_eq!(batch.project_revision, 8);
    assert_eq!(
        batch.heads,
        [("z_producer".into(), 7), ("a_follow".into(), 2)].into()
    );
    assert_eq!(
        batch
            .changes
            .iter()
            .map(|(id, prepared)| (id.as_str(), &prepared.outputs["width"]))
            .collect::<Vec<_>>(),
        [("z_producer", &width(0.12)), ("a_follow", &width(0.12))]
    );
    assert!(batch.pins.is_empty());

    // During a simultaneous edit of both recipes the pin still sees an
    // admitted revision, even when that revision is the producer's current head.
    let same_head_pin = graph
        .prepare(&["z_producer".into(), "b_pinned".into()].into(), prepare)
        .unwrap();
    assert_eq!(same_head_pin.pins, [("z_producer".into(), 7)].into());
    assert_eq!(same_head_pin.changes[0].1.outputs["width"], width(0.1));

    graph.history.insert(
        ("z_producer".into(), 7),
        [("width".into(), width(0.1))].into(),
    );
    let producer = graph.current.get_mut("z_producer").unwrap();
    producer.revision = 8;
    producer.outputs.insert("width".into(), width(0.11));
    let historical_pin = graph.prepare(&["b_pinned".into()].into(), prepare).unwrap();
    assert_eq!(historical_pin.changes[0].1.outputs["width"], width(0.1));

    let mut attempted = Vec::new();
    let failure = graph.prepare(&edit_producer, |id, inputs| {
        attempted.push(id.to_owned());
        if id == "a_follow" {
            assert_eq!(inputs["value"], &width(0.12));
            return prepare_geometry_edits(
                &source,
                snapshot,
                [("width", Some(ParamValue::Length(0.0)))],
                1000,
            )
            .map(|_| unreachable!());
        }
        prepare(id, inputs)
    });
    assert!(matches!(failure, Err(ClosureError::Recipe { id, .. }) if id == "a_follow"));
    assert_eq!(attempted, ["z_producer", "a_follow"]);
    assert_eq!(graph.current["z_producer"].outputs["width"], width(0.11));
    assert_eq!(snapshot.entries()["width"].override_value, None);
}

#[test]
fn invalid_graphs_fail_before_evaluation_and_invalid_candidate_ports_abort_the_batch() {
    let mut graph = AdmittedRecipes {
        current: [
            (
                "a".into(),
                head(1, Some(binding("b", BindingRevision::FollowCurrent))),
            ),
            (
                "b".into(),
                head(1, Some(binding("a", BindingRevision::FollowCurrent))),
            ),
        ]
        .into(),
        ..Default::default()
    };
    let edited = ["a".into()].into();
    let unevaluated = |_: &str, _: &BTreeMap<String, &Value>| -> Result<PreparedRecipe<()>, ()> {
        panic!("invalid admission must fail before evaluation")
    };
    let check = |result: Result<_, ClosureError<()>>, code| {
        assert!(matches!(result, Err(ClosureError::Graph(error)) if error.code == code));
    };
    check(graph.prepare(&edited, unevaluated), "RecipeDependencyCycle");

    graph.current.get_mut("a").unwrap().bindings.clear();
    check(
        graph.prepare(&edited, |_, _| {
            Ok(PreparedRecipe {
                change: (),
                outputs: Default::default(),
            })
        }),
        "MissingRecipeOutput",
    );
    let b = graph.current.get_mut("b").unwrap();
    b.bindings.get_mut("value").unwrap().expected_type = Type::Number;
    check(
        graph.prepare(&edited, unevaluated),
        "RecipeOutputTypeMismatch",
    );

    let b = graph.current.get_mut("b").unwrap();
    b.bindings.insert(
        "value".into(),
        binding("a", BindingRevision::ExactPinned(0)),
    );
    check(
        graph.prepare(&["b".into()].into(), unevaluated),
        "MissingRecipeRevision",
    );

    graph.current.get_mut("b").unwrap().bindings.clear();
    for index in 0..64 {
        graph.current.insert(
            format!("consumer-{index}"),
            head(1, Some(binding("a", BindingRevision::FollowCurrent))),
        );
    }
    check(graph.prepare(&edited, unevaluated), "AffectedRecipeLimit");

    graph.current.retain(|id, _| id == "a" || id == "b");
    let b = graph.current.get_mut("b").unwrap();
    for index in 0..4097 {
        b.bindings.insert(
            format!("read-{index}"),
            binding("a", BindingRevision::FollowCurrent),
        );
    }
    check(graph.prepare(&edited, unevaluated), "RecipeReadLimit");
}

#[test]
fn recipe_reconciliation_protects_references_and_freezes_detached_geometry() {
    let mut source = recipe();
    let base = prepare_geometry(&source, &Default::default(), 1000).unwrap();
    let empty = OwnedRegion::new(
        Provenance {
            recipe: "plate".into(),
            accepted_revision: 0,
            source_digest: String::new(),
        },
        false,
        BTreeMap::new(),
    )
    .unwrap();
    let reserve = |key: &OwnedKey| Ok(format!("{key:?}"));
    let ordered = base
        .reconcile(&empty, 1, &BTreeMap::new(), reserve)
        .unwrap();
    let region = ordered.region;

    let edited_source = source.files.get_mut(&source.entry).unwrap();
    edited_source.insert(0, '\n');
    let reformatted = prepare_geometry(&source, &Default::default(), 1000).unwrap();
    let reused = reformatted
        .reconcile(&region, 2, &BTreeMap::new(), |_| {
            panic!("reuse keeps identity")
        })
        .unwrap();
    assert_ne!(
        reused.region.provenance().source_digest,
        region.provenance().source_digest
    );
    assert!(reused
        .actions
        .values()
        .all(|action| *action == Reconciliation::Reuse));
    assert_eq!(region.records(), reused.region.records());

    let body_output = OwnedKey::Output("solid".into());
    let right = OwnedKey::Entity("right_hole".into());
    let plate = OwnedKey::Entity("blank".into());
    let source_text = source.files.get_mut(&source.entry).unwrap();
    let hole = source_text.find("part = cad.through_hole").unwrap();
    source_text.truncate(hole);
    source_text.push_str("cad.output(\"solid\", left)\ncad.output(\"blank\", blank)\n");
    let shrunk = prepare_geometry(&source, &Default::default(), 1000).unwrap();
    let right_id = region.records()[&right].id.clone();
    let inbound: BTreeMap<String, BTreeSet<String>> = [
        (right_id.clone(), ["drawing".into()].into()),
        ("unrelated".into(), ["assembly".into()].into()),
    ]
    .into();
    let error = shrunk
        .reconcile(&region, 2, &inbound, |_| {
            panic!("validate before allocating")
        })
        .unwrap_err();
    assert_eq!(error.code, "ReferencedEntityRemoval");
    assert!(matches!(
        region.records()[&right].state,
        Ownership::Managed(_)
    ));

    let unrelated = inbound
        .into_iter()
        .filter(|(id, _)| id == "unrelated")
        .collect();
    let accepted = shrunk.reconcile(&region, 2, &unrelated, reserve).unwrap();
    assert_eq!(accepted.actions[&right], Reconciliation::Remove);
    assert_eq!(accepted.actions[&body_output], Reconciliation::Update);
    assert_eq!(
        accepted.region.records()[&body_output].id,
        region.records()[&body_output].id
    );
    assert_ne!(
        accepted.region.records()[&OwnedKey::Output("blank".into())].id,
        region.records()[&plate].id
    );
    assert_eq!(accepted.region.records()[&right].id, right_id);
    assert!(matches!(
        accepted.region.records()[&right].state,
        Ownership::Tombstone
    ));
    assert_eq!(
        base.reconcile(&accepted.region, 3, &unrelated, reserve)
            .unwrap_err()
            .code,
        "TombstonedStableKey"
    );

    let detached = accepted
        .region
        .detach(|_, definition| Ok(definition.value.freeze()))
        .unwrap();
    assert!(detached.region.is_detached());
    assert_eq!(detached.former_owner, *accepted.region.provenance());
    assert!(!detached.released_ids.contains(&right_id));
    assert_eq!(
        detached.region.records()[&plate].id,
        region.records()[&plate].id
    );
    assert!(matches!(
        &detached.region.records()[&plate].state,
        Ownership::Detached(definition) if matches!(&definition.value, GeometryDefinition::Feature { inputs, .. } if inputs.is_empty())
    ));
    assert_eq!(
        base.reconcile(&detached.region, 4, &unrelated, reserve)
            .unwrap_err()
            .code,
        "DetachedRecipe"
    );
}
