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

#[test]
fn bound_parameters_allow_reads_and_reject_command_overrides() {
    use luma::{
        cad_engine::CommandHost,
        host::{run_with_host, Mode},
    };

    let recipe: WireSourceBundle =
        serde_json::from_str(include_str!("../examples/cad/plate-geometry.json")).unwrap();
    let plan = prepare_geometry(&recipe.admit().unwrap(), &Default::default(), 1000).unwrap();
    let host = CommandHost::new(PART.into(), plan.parameters.parameters)
        .unwrap()
        .with_read_only(["width".into()].into());
    let command: WireSourceBundle =
        serde_json::from_str(include_str!("../examples/cad/plate-width-command.json")).unwrap();
    let command = command.admit().unwrap();

    let rejected = run_with_host(&command, Mode::Command, &host, 1000).unwrap_err();
    assert_eq!(rejected.diagnostics[0].1.code, "ManagedParameterConflict");
    let mut read = command;
    let entry = read.files.get_mut(&read.entry).unwrap();
    *entry = entry.replace("param.set(width, units.mm(120))", "");
    assert!(run_with_host(&read, Mode::Command, &host, 1000).is_ok());
    assert_eq!(
        host.finish().unwrap().entries()["width"].effective(),
        ParamValue::Length(0.1),
    );
}
