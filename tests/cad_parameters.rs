use luma::{
    cad::{prepare_parameter_edits, prepare_parameters, PreparationError},
    cad_engine::{ExprOp, ParamValue, ParameterHost, ParameterSnapshot},
    host::{run_with_host, Dispatch, Mode, SourceBundle, Value},
    host_api::{CallSite, Descriptor},
};

fn bundle(source: &str) -> SourceBundle {
    SourceBundle {
        entry: "main.luma".into(),
        files: [("main.luma".into(), source.into())].into(),
    }
}

fn error(result: Result<impl Sized, PreparationError>, code: &str) {
    let failure = result.err().expect("expected preparation failure");
    match failure {
        PreparationError::Parameters(problem) => assert_eq!(problem.code, code, "{problem:?}"),
        PreparationError::Execution(problem) => assert!(
            problem
                .diagnostics
                .iter()
                .any(|(_, item)| item.code == code),
            "expected {code}, got {problem:?}",
        ),
    }
}

const PLATE: &str = r#"use units
use param

width = param.length("width", units.mm(100))
bounded = param.bounds(width, units.mm(10), units.mm(150))
hole_x = param.subtract(param.scale(bounded, 0.5), units.mm(10))
opposite = param.negate(hole_x)
"#;

#[test]
fn rerunning_recipe_preserves_overrides_and_expression_provenance() {
    let empty = ParameterSnapshot::default();
    let initial = prepare_parameters(&bundle(PLATE), &empty, 1000).unwrap();
    assert!(empty.entries().is_empty());
    let state = &initial.parameters;
    let width = &state.entries()["width"];
    assert_eq!(width.declaration.default, ParamValue::Length(0.1));
    assert_eq!(width.override_value, None);
    let expression = &initial.expressions["hole_x"];
    assert_eq!(expression.dependencies().collect::<Vec<_>>(), ["width"]);
    assert_eq!(
        expression.evaluate(state).unwrap(),
        ParamValue::Length(0.04)
    );
    assert!(expression
        .nodes
        .iter()
        .all(|node| node.site.file == "main.luma"));

    let override_state = state
        .with_override("width", Some(ParamValue::Length(0.12)))
        .unwrap();
    let updated = prepare_parameters(&bundle(PLATE), &override_state, 1000).unwrap();
    assert_eq!(state.entries()["width"].override_value, None);
    assert_eq!(
        updated.parameters.entries()["width"].declaration.default,
        ParamValue::Length(0.1)
    );
    assert_eq!(
        updated.parameters.entries()["width"].override_value,
        Some(ParamValue::Length(0.12))
    );
    assert_eq!(updated.expressions["hole_x"], *expression);
    assert!((length(expression.evaluate(&updated.parameters).unwrap()) - 0.05).abs() < 1e-14);
    assert!(
        (length(
            updated.expressions["opposite"]
                .evaluate(&updated.parameters)
                .unwrap()
        ) + 0.05)
            .abs()
            < 1e-14
    );

    let edited_source = bundle(&PLATE.replace("mm(100)", "mm(110)"));
    let revised = prepare_parameters(&edited_source, &updated.parameters, 1000).unwrap();
    assert_eq!(
        revised.parameters.entries()["width"].declaration.default,
        ParamValue::Length(0.11)
    );
    assert_eq!(
        revised.parameters.entries()["width"].effective(),
        ParamValue::Length(0.12)
    );
    let reset = revised.parameters.with_override("width", None).unwrap();
    assert_eq!(
        reset.entries()["width"].effective(),
        ParamValue::Length(0.11)
    );
}

fn length(value: ParamValue) -> f64 {
    let ParamValue::Length(value) = value else {
        panic!("expected a length")
    };
    value
}

#[test]
fn rejected_recipe_never_modifies_inherited_parameters() {
    let old = prepare_parameters(&bundle(PLATE), &ParameterSnapshot::default(), 1000)
        .unwrap()
        .parameters
        .with_override("width", Some(ParamValue::Length(0.12)))
        .unwrap();
    let before = old.clone();
    error(
        prepare_parameters(&bundle(&PLATE.replace("mm(150)", "mm(110)")), &old, 1000),
        "ParameterBounds",
    );
    error(
        prepare_parameters(
            &bundle(
                r#"use param
width = param.scalar("width", 5)
"#,
            ),
            &old,
            1000,
        ),
        "ParameterTypeMigrationRequired",
    );
    error(
        prepare_parameters(&bundle("x = 1"), &old, 1000),
        "OverrideOrphaned",
    );
    error(
        prepare_parameters(
            &bundle(&format!(
                "{PLATE}\nduplicate = param.length(\"width\", units.mm(5))"
            )),
            &old,
            1000,
        ),
        "DuplicateStableKey",
    );
    assert_eq!(old, before);
}

#[test]
fn parameter_kinds_and_deferred_host_schemes_share_checker_and_runtime() {
    let source = bundle(
        r#"use units
use param

fn twice(x)
  return param.add(x, x)
end
angle = param.angle("angle", units.rad(1))
scalar = param.scalar("scalar", 2)
count = param.count("count", 9007199254740991)
enabled = param.boolean("enabled", true)
value_of_count = param.value(count)
value_of_enabled = param.value(enabled)
angle_expr = twice(angle)
scalar_expr = param.divide_scalar(param.add(scalar, 10), 4)
param.expose(count, false)
"#,
    );
    let host = ParameterHost::new(ParameterSnapshot::default()).unwrap();
    for descriptor in host.registry().descriptors() {
        let json = serde_json::to_string(descriptor).unwrap();
        assert_eq!(
            serde_json::from_str::<Descriptor>(&json).unwrap(),
            *descriptor
        );
    }
    let output = run_with_host(&source, Mode::Recipe, &host, 1000).unwrap();
    assert_eq!(
        output.outputs["value_of_count"],
        Value::Number(9007199254740991.0)
    );
    assert_eq!(output.outputs["value_of_enabled"], Value::Bool(true));
    let plan = host.finish(&output.outputs).unwrap();
    assert_eq!(
        plan.expressions["angle_expr"]
            .evaluate(&plan.parameters)
            .unwrap(),
        ParamValue::Angle(2.0)
    );
    assert_eq!(
        plan.expressions["scalar_expr"]
            .evaluate(&plan.parameters)
            .unwrap(),
        ParamValue::Scalar(3.0)
    );
    assert_eq!(
        plan.parameters
            .with_override("count", Some(ParamValue::Count(2)))
            .unwrap_err()
            .code,
        "ParameterNotExposed"
    );

    for (text, code) in [
        (
            "param.count(\"invalid\", 9007199254740992)",
            "ParameterCountRange",
        ),
        ("param.count(\"invalid\", 1.5)", "ParameterCountRange"),
        (
            "param.divide_scalar(units.mm(3), 0)",
            "ParameterDivisionByZero",
        ),
        ("param.add(units.mm(3), units.rad(1))", "E_TYPE"),
    ] {
        let source = bundle(&format!("use units\nuse param\nx = {text}"));
        error(
            prepare_parameters(&source, &ParameterSnapshot::default(), 1000),
            code,
        );
    }
}

#[test]
fn expression_arenas_share_reused_subexpressions_and_reject_invalid_edges() {
    let text = format!("{PLATE}\nshared = param.add(hole_x, hole_x)");
    let plan = prepare_parameters(&bundle(&text), &ParameterSnapshot::default(), 1000).unwrap();
    let shared = &plan.expressions["shared"];
    let root = shared.nodes.len() - 1;
    assert_eq!(
        shared.nodes.len(),
        plan.expressions["hole_x"].nodes.len() + 1
    );
    assert_eq!(
        shared.nodes[root].operation,
        ExprOp::Add {
            left: root - 1,
            right: root - 1
        }
    );

    let mut invalid = shared.clone();
    invalid.nodes[root].operation = ExprOp::Negate { input: root };
    assert_eq!(
        invalid.evaluate(&plan.parameters).unwrap_err().code,
        "InvalidParameterExpression"
    );
}

#[test]
fn runtime_ref_handles_are_bound_to_one_preparation() {
    let first = ParameterHost::new(ParameterSnapshot::default()).unwrap();
    let second = ParameterHost::new(ParameterSnapshot::default()).unwrap();
    let source = bundle("use param\nwidth = param.scalar(\"width\", 1)");
    let output = run_with_host(&source, Mode::Recipe, &first, 100).unwrap();
    let site = CallSite {
        file: "main.luma".into(),
        start: 0,
        end: 0,
    };
    let descriptor = second.registry().get("param", "value").unwrap();
    let problem = second
        .call(
            descriptor,
            Mode::Recipe,
            &site,
            &[output.outputs["width"].clone()],
        )
        .unwrap_err();
    assert_eq!(problem.code, "ForeignParameterHandle");
}

#[test]
fn cli_checks_the_parameter_bundle_in_recipe_mode() {
    use std::process::Command;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/cad/plate-parameters.json"
    );
    let run = |mode| {
        Command::new(env!("CARGO_BIN_EXE_luma"))
            .args(["cad", "check", path, "--mode", mode])
            .output()
            .unwrap()
    };
    let result = run("recipe");
    assert!(result.status.success(), "{:?}", result);
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["result"]["profile"], "recipe-parameters-v1");
    let rejected = run("command");
    assert_eq!(rejected.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert!(json["error"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["code"] == "HostModeMismatch"));
}

#[test]
fn batch_prepares_final_overrides_and_reruns_once() {
    let recipe = bundle(PLATE);
    let state = prepare_parameters(&recipe, &ParameterSnapshot::default(), 1000)
        .unwrap()
        .parameters;
    let inputs = [
        ("width", Some(ParamValue::Length(0.11))),
        ("width", Some(ParamValue::Length(0.12))),
    ];
    let plan = prepare_parameter_edits(&recipe, &state, inputs, 1000).unwrap();
    assert_eq!(
        plan.parameters.entries()["width"].effective(),
        ParamValue::Length(0.12)
    );
    assert!(
        (length(
            plan.expressions["hole_x"]
                .evaluate(&plan.parameters)
                .unwrap()
        ) - 0.05)
            .abs()
            < 1e-14
    );
    error(
        prepare_parameter_edits(
            &bundle(&PLATE.replace("mm(150)", "mm(110)")),
            &state,
            inputs,
            1000,
        ),
        "ParameterBounds",
    );
    error(
        prepare_parameter_edits(&recipe, &state, [inputs[0], ("missing", None)], 1000),
        "UnknownParameter",
    );
    assert_eq!(
        state.entries()["width"].effective(),
        ParamValue::Length(0.1)
    );
    assert_eq!(state.entries()["width"].override_value, None);
}
