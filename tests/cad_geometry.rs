use luma::{
    cad::{prepare_geometry, prepare_geometry_edits, PreparationError},
    cad_engine::{Geometry, GeometryHost, ParamValue, ParameterSnapshot},
    host::{run_with_host, CallSite, Dispatch, Mode, SourceBundle, Value, WireSourceBundle},
};

fn example() -> SourceBundle {
    serde_json::from_str::<WireSourceBundle>(include_str!("../examples/cad/plate-geometry.json"))
        .unwrap()
        .admit()
        .unwrap()
}

fn expect_error(source: &SourceBundle, code: &str) {
    match prepare_geometry(source, &Default::default(), 1000).unwrap_err() {
        PreparationError::Parameters(error) => assert_eq!(error.code, code),
        PreparationError::Execution(failure) => assert!(
            failure
                .diagnostics
                .iter()
                .any(|(_, diagnostic)| diagnostic.code == code),
            "expected {code}, got {failure:?}",
        ),
    }
}

fn replace(before: &str, after: &str) -> SourceBundle {
    let mut bundle = example();
    let source = bundle.files.get_mut(&bundle.entry).unwrap();
    assert!(source.contains(before));
    *source = source.replace(before, after);
    bundle
}

#[test]
fn plate_plan_preserves_graph_outputs_and_si_dimensions_after_override() {
    let recipe = example();
    let base = prepare_geometry(&recipe, &Default::default(), 1000).unwrap();
    let edited = prepare_geometry_edits(
        &recipe,
        &base.parameters.parameters,
        [("width", Some(ParamValue::Length(0.12)))],
        1000,
    )
    .unwrap();

    assert_eq!(base.source_digest, edited.source_digest);
    assert_eq!(edited.outputs["solid"], "right_hole");
    assert_eq!(edited.view_intents.len(), 1);
    assert_eq!(edited.view_intents[0].body, "right_hole");
    assert_eq!(
        edited
            .features
            .iter()
            .map(|feature| feature.key.as_str())
            .collect::<Vec<_>>(),
        ["blank", "left_hole", "right_hole"]
    );
    let Geometry::Plate { size_m } = edited.features[0].geometry else {
        panic!("expected plate")
    };
    assert_eq!(size_m, [0.12, 0.06, 0.008]);
    for (plan, x) in [(&base, 0.04_f64), (&edited, 0.05_f64)] {
        for (index, predecessor, expected_x) in [(1, "blank", -x), (2, "left_hole", x)] {
            let feature = &plan.features[index];
            let Geometry::ThroughHole {
                base,
                diameter_m,
                centre_m,
                entry_z_m,
            } = &feature.geometry
            else {
                panic!("expected through-hole")
            };
            assert_eq!(base, predecessor);
            assert_eq!(*diameter_m, 0.006);
            assert_eq!(*entry_z_m, 0.004);
            assert!((centre_m[0] - expected_x).abs() < 1e-14);
            assert_eq!(centre_m[1], 0.0);
            assert_eq!(feature.site.file, "plate.luma");
            assert_eq!(
                feature.inputs["x"].dependencies().collect::<Vec<_>>(),
                ["width"]
            );
        }
    }
    assert_eq!(
        base.parameters.parameters.entries()["width"].override_value,
        None
    );

    let repeated = prepare_geometry(&recipe, &Default::default(), 1000).unwrap();
    assert_eq!(
        serde_json::to_value(repeated).unwrap(),
        serde_json::to_value(base).unwrap()
    );
}

#[test]
fn invalid_profile_and_geometry_keys_reject_the_whole_plan() {
    for (before, after, code) in [
        ("units.mm(8)", "units.mm(0)", "InvalidPlateSize"),
        ("units.mm(6)", "units.mm(0)", "InvalidHoleDiameter"),
        ("units.mm(10)", "units.mm(1)", "HoleEdgeClearance"),
        ("units.mm(10)", "units.mm(50)", "HolePairClearance"),
        ("\"left_hole\"", "\"blank\"", "DuplicateStableKey"),
        ("\"left_hole\"", "\"width\"", "DuplicateStableKey"),
        (
            "cad.output(\"solid\", part)",
            "cad.output(\"solid\", part)\ncad.output(\"solid\", left)",
            "DuplicateOutput",
        ),
        (
            "cad.plate(\"blank\", width, height, thickness)",
            "cad.plate(\"blank\", units.rad(1), height, thickness)",
            "HostTypeMismatch",
        ),
    ] {
        expect_error(&replace(before, after), code);
    }
    let recipe = example();
    let plan = prepare_geometry(&recipe, &Default::default(), 1000).unwrap();
    let result = prepare_geometry_edits(
        &recipe,
        &plan.parameters.parameters,
        [("diameter", Some(ParamValue::Length(0.025)))],
        1000,
    )
    .unwrap_err();
    assert!(
        matches!(result, PreparationError::Parameters(error) if error.code == "HoleEdgeClearance")
    );
    assert_eq!(
        plan.parameters.parameters.entries()["diameter"].override_value,
        None
    );
}

#[test]
fn a_provisional_body_cannot_be_used_in_another_preparation() {
    let first = GeometryHost::new(ParameterSnapshot::default()).unwrap();
    let next = GeometryHost::new(ParameterSnapshot::default()).unwrap();
    let recipe = example();
    let first_run = run_with_host(&recipe, Mode::Recipe, &first, 1000).unwrap();
    run_with_host(&recipe, Mode::Recipe, &next, 1000).unwrap();
    let output = next.registry().get("cad", "output").unwrap();
    let site = CallSite {
        file: "plate.luma".into(),
        start: 0,
        end: 0,
    };
    let err = next
        .call(
            output,
            Mode::Recipe,
            &site,
            &[
                Value::Text("foreign".into()),
                first_run.outputs["part"].clone(),
            ],
        )
        .unwrap_err();
    assert_eq!(err.code, "ForeignGeometryHandle");
}

#[test]
fn cli_exposes_one_machine_readable_plan_for_the_plate() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/cad/plate-geometry.json"
    );
    let invoke = |command| {
        std::process::Command::new(env!("CARGO_BIN_EXE_luma"))
            .args(["cad", command, path, "--mode", "recipe"])
            .output()
            .unwrap()
    };
    assert!(invoke("check").status.success());
    let result = invoke("plan");
    assert!(result.status.success());
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["protocol"], "luma.cad.geometry-plan/1");
    assert_eq!(json["result"]["profile"], "recipe-geometry-v1");
    assert_eq!(json["result"]["features"].as_array().unwrap().len(), 3);
    assert!(json.get("error").is_none());
}
