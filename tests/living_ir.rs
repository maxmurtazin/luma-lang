use serde_json::Value;
use std::process::Command;

fn bridge(bundle: &str) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_luma"))
        .args(["cad", "living-ir", bundle, "--mode", "recipe"])
        .output()
        .unwrap();
    (
        output.status.success(),
        serde_json::from_slice(&output.stdout).unwrap(),
    )
}

#[test]
fn fixed_rectangle_preserves_source_order_ids_and_si_dimensions() {
    let (accepted, report) = bridge("examples/cad/living-ir-rectangle.json");
    assert!(accepted, "{report}");
    assert_eq!(report["protocol"], "luma.cad.living-ir-plan/1");
    let result = &report["result"];
    assert_eq!(
        result["source_digest"],
        "sha256:0554805bbbb56e8b01590dc72cf749aabdd6e4f0ae7a1891bad4e2dc00626111"
    );
    assert_eq!(
        result["source_ids"]["points"][0],
        "98420000-0000-4000-8000-000000000014"
    );
    assert_eq!(
        result["source_ids"]["segments"][3],
        "98420000-0000-4000-8000-00000000002b"
    );
    assert_eq!(
        result["rectangle"]["b"]["xy"],
        serde_json::json!([0.04, 0.03])
    );
    assert_eq!(result["extrusion"]["length_m"], "0.004");
    assert_eq!(
        result["extrusion"]["feature_id"],
        "98420000-0000-4000-8000-000000000050"
    );
}

#[test]
fn sketch_with_a_hole_rejects_without_a_partial_proposal() {
    let (accepted, report) = bridge("examples/cad/sketch-recipe.json");
    assert!(!accepted);
    assert_eq!(report["error"]["code"], "UnsupportedLivingIrRecipe");
    assert!(report.get("result").is_none());
}

#[test]
fn existing_luma_parameter_command_keeps_the_http_pin_and_emits_integral_mm() {
    use luma::{host::WireSourceBundle, living_ir};
    let source: WireSourceBundle =
        serde_json::from_str(include_str!("../examples/cad/plate-width-command.json")).unwrap();
    let bundle = source.admit().unwrap();
    let mut context = living_ir::CommandContext {
        document: "00000000-0000-4000-8000-000000000002".into(),
        revision: 12,
        epoch: 3,
        incarnation: "inc-1".into(),
        definition: "a".repeat(64),
        parameters_mm: [("width".into(), "81.125".into())].into(),
    };
    let mut foreign = serde_json::to_value(&context).unwrap();
    foreign["document"] = serde_json::json!("00000000-0000-4000-8000-000000000003");
    assert!(living_ir::prepare_literal(&bundle, serde_json::from_value(foreign).unwrap()).is_err());
    let proposal = living_ir::prepare_literal(
        &bundle,
        serde_json::from_value(serde_json::to_value(&context).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(proposal["scope"]["revision"], 12);
    assert_eq!(proposal["scope"]["epoch"], 3);
    assert_eq!(
        proposal["intent"],
        serde_json::json!({"operation":"literal_mm","parameter":"width","decimal":"120"})
    );
    context.parameters_mm.insert("width".into(), "120".into());
    assert!(living_ir::prepare_literal(&bundle, context).is_err());
}
