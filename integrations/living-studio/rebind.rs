//! Explicit Studio source rebind with native conflict checks and a reviewed plan file.
use super::{
    CallSite, Descriptor, Dispatch, Effect, Error, Json, Mode, PRESENTATION, Parameter, Path,
    Registry, ResultType, SEALED_MODULES, STUDIO_PIN, Signature, Type, Value, check_command,
    invalid_json, io_error, json, provider, read_json, studio_binary, verify_studio,
};
use presentation_core::rebind::RebindPlan;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io::Write,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

const SOURCE: &str = "CadSnapshotRef";
const PLAN: &str = "RebindPlanRef";

#[derive(Clone, Copy)]
enum Action {
    Rebind,
    Edit,
}

impl Action {
    fn spec(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Rebind => ("captured_source", SOURCE, "rebind", PLAN),
            Self::Edit => (
                "edits",
                "PresentationEditsRef",
                "apply",
                "PresentationPlanRef",
            ),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewedPlan {
    studio_revision: String,
    command_digest: String,
    plan: RebindPlan,
}

struct PresentationHost {
    registry: Registry,
    declared: AtomicBool,
    action: Action,
}

impl PresentationHost {
    fn new(action: Action) -> Result<Self, Error> {
        let (input, input_type, operation, result) = action.spec();
        let specs: [Signature<'_>; 3] = [
            (
                "studio",
                "presentation",
                &[],
                PRESENTATION,
                Effect::ReadPinned,
            ),
            ("studio", input, &[], input_type, Effect::ReadPinned),
            (
                "studio",
                operation,
                &[PRESENTATION, input_type],
                result,
                Effect::BuildPlan,
            ),
        ];
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            specs
                .into_iter()
                .map(|(module, name, parameters, result, effect)| Descriptor {
                    module: module.into(),
                    name: name.into(),
                    operation: format!("{module}.{name}"),
                    major: 1,
                    parameters: parameters
                        .iter()
                        .map(|name| Parameter::Exact(Type::Opaque((*name).into())))
                        .collect(),
                    result: ResultType::Exact(Type::Opaque(result.into())),
                    effect,
                    modes: [Mode::Command].into(),
                }),
        )?;
        Ok(Self {
            registry,
            declared: AtomicBool::new(false),
            action,
        })
    }
}

impl Dispatch for PresentationHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        let (input, input_type, operation, result) = self.action.spec();
        let type_name = match (descriptor.name.as_str(), args) {
            ("presentation", []) => PRESENTATION,
            (name, []) if name == input => input_type,
            (name, [presentation, source])
                if name == operation
                    && presentation.value_type() == Some(Type::Opaque(PRESENTATION.into()))
                    && source.value_type() == Some(Type::Opaque(input_type.into())) =>
            {
                if self.declared.swap(true, Ordering::Relaxed) {
                    return Err(Error::new(
                        "PresentationPlanLimit",
                        "Expected one Studio plan declaration",
                    ));
                }
                result
            }
            _ => return Err(Error::new("PresentationInputType", &descriptor.operation)),
        };
        Ok(Value::Opaque {
            type_name: type_name.into(),
            payload: Json::Null,
        })
    }
}

fn presentation_command(command: &Path, action: Action) -> Result<String, Json> {
    let host = PresentationHost::new(action)
        .map_err(|error| json!({"code":error.code,"message":error.message}))?;
    let digest = check_command(command, &host, "PresentationCommandRejected")?;
    if !host.declared.into_inner() {
        return Err(json!({"code":"MissingPresentationPlan"}));
    }
    Ok(digest)
}

pub(super) fn apply(args: &[OsString]) -> Result<Json, Json> {
    let [root, project, edits, command] = args else {
        return Err(
            json!({"code":"Usage","message":"presentation-apply STUDIO PROJECT EDITS.json COMMAND.json"}),
        );
    };
    verify_studio(Path::new(root))?;
    let command_digest = presentation_command(Path::new(command), Action::Edit)?;
    let receipt = provider(
        Command::new(studio_binary()?)
            .arg("apply")
            .arg(project)
            .arg(edits),
    )?;
    Ok(json!({
        "studio_revision": STUDIO_PIN.trim(),
        "command_digest": command_digest,
        "receipt": receipt
    }))
}

pub(super) fn preview(args: &[OsString]) -> Result<Json, Json> {
    let [root, project, source, command, destination, decisions @ ..] = args else {
        return Err(
            json!({"code":"Usage","message":"rebind-preview STUDIO PROJECT CAPTURE COMMAND.json PLAN.json [DECISIONS.json]"}),
        );
    };
    verify_studio(Path::new(root))?;
    let command_digest = presentation_command(Path::new(command), Action::Rebind)?;
    let mut native = Command::new(studio_binary()?);
    native.arg("rebind-preview").arg(project).arg(source);
    match decisions {
        [] => {}
        [path] => {
            native.arg("--decisions").arg(path);
        }
        _ => return Err(json!({"code":"Usage"})),
    }
    let destination = Path::new(destination);
    if destination.try_exists().map_err(io_error)? {
        return Err(json!({"code":"RebindPlanExists"}));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    let plan: RebindPlan = serde_json::from_value(provider(&mut native)?).map_err(invalid_json)?;
    let document = serde_json::to_value(ReviewedPlan {
        studio_revision: STUDIO_PIN.trim().into(),
        command_digest,
        plan,
    })
    .map_err(invalid_json)?;
    serde_json::to_writer(&mut staged, &document).map_err(invalid_json)?;
    staged.write_all(b"\n").map_err(io_error)?;
    staged.as_file().sync_all().map_err(io_error)?;
    staged
        .persist_noclobber(destination)
        .map_err(|err| io_error(err.error))?;
    Ok(document)
}

pub(super) fn accept(args: &[OsString]) -> Result<Json, Json> {
    let [root, project, reviewed, header] = args else {
        return Err(
            json!({"code":"Usage","message":"rebind-accept STUDIO PROJECT PLAN.json HEADER.json"}),
        );
    };
    verify_studio(Path::new(root))?;
    let reviewed: ReviewedPlan = read_json(Path::new(reviewed), 8 * 1024 * 1024)?;
    if reviewed.studio_revision != STUDIO_PIN.trim() {
        return Err(json!({"code":"StudioPinMismatch","revision":STUDIO_PIN.trim()}));
    }
    let mut file = tempfile::NamedTempFile::new().map_err(io_error)?;
    serde_json::to_writer(&mut file, &reviewed.plan).map_err(invalid_json)?;
    let receipt = provider(
        Command::new(studio_binary()?)
            .arg("rebind-accept")
            .arg(project)
            .arg(file.path())
            .arg(header),
    )?;
    Ok(json!({
        "studio_revision": reviewed.studio_revision,
        "command_digest": reviewed.command_digest,
        "plan_digest": reviewed.plan.plan_digest,
        "receipt": receipt
    }))
}
