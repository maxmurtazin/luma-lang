use luma::{
    cad::{prepare_command, prepare_geometry, PreparationError},
    cad_engine::ParamValue,
    host::{SourceBundle, WireSourceBundle},
};

const PART: &str = "00000000-0000-4000-8000-000000000002";

#[test]
fn commands_scope_reads_to_one_part_and_observe_their_own_overrides() {
    let recipe: WireSourceBundle =
        serde_json::from_str(include_str!("../examples/cad/plate-geometry.json")).unwrap();
    let plan = prepare_geometry(&recipe.admit().unwrap(), &Default::default(), 1000).unwrap();
    let checked = &plan.parameters.parameters;
    let command: WireSourceBundle =
        serde_json::from_str(include_str!("../examples/cad/plate-width-command.json")).unwrap();
    let mut command = command.admit().unwrap();
    let text = command.files.get_mut(&command.entry).unwrap();
    text.push_str("\nparam.set(width, units.add(current, units.mm(5)))\n");
    let edited = prepare_command(&command, PART.into(), checked, 1000).unwrap();
    assert_eq!(
        edited.entries()["width"].effective(),
        ParamValue::Length(0.125)
    );
    assert_eq!(
        checked.entries()["width"].effective(),
        ParamValue::Length(0.1)
    );
    let text = command.files.get_mut(&command.entry).unwrap();
    text.push_str("\nparam.reset(width)\n");
    let reset = prepare_command(&command, PART.into(), checked, 1000).unwrap();
    assert_eq!(reset, *checked);
    let result =
        prepare_command(&command, "another admitted part".into(), checked, 1000).unwrap_err();
    assert!(matches!(result, PreparationError::Execution(failure)
        if failure.diagnostics.iter().any(|(_, diagnostic)| diagnostic.code == "UnadmittedPart")));

    let mut damaged = SourceBundle {
        entry: command.entry.clone(),
        files: command.files.clone(),
    };
    damaged
        .files
        .get_mut(&damaged.entry)
        .unwrap()
        .push_str("\nparam.set(width, units.mm(1) / 0)\n");
    assert!(prepare_command(&damaged, PART.into(), checked, 1000).is_err());
    assert_eq!(checked.entries()["width"].override_value, None);
}
