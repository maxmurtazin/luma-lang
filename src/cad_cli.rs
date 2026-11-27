use luma::{cad, host, host::Dispatch};
use serde_json::json;
use std::{fs::File, io::Read, process::ExitCode};

const MAX_INPUT: usize = 256 * 1024;

pub fn run(args: &[String]) -> ExitCode {
    let bridge = args.first().is_some_and(|command| command == "living-ir");
    let literal = args
        .first()
        .is_some_and(|command| command == "living-ir-command");
    let planning = bridge || literal || args.first().is_some_and(|command| command == "plan");
    let report = || -> Result<_, (u8, serde_json::Value)> {
        let [command, path, flag, mode] = args else {
            return Err(usage());
        };
        if !matches!(
            command.as_str(),
            "check" | "plan" | "living-ir" | "living-ir-command"
        ) || flag != "--mode"
        {
            return Err(usage());
        }
        let mode = match mode.as_str() {
            "recipe" => host::Mode::Recipe,
            "command" => host::Mode::Command,
            _ => return Err(usage()),
        };
        if (planning && !literal && mode != host::Mode::Recipe)
            || (literal && mode != host::Mode::Command)
        {
            return Err((
                3,
                json!({
                    "code": "UnsupportedCapability",
                    "message": if literal { "Living IR parameter planning requires command mode" } else { "geometry planning requires recipe mode" },
                }),
            ));
        }
        let mut input = Vec::new();
        File::open(path)
            .and_then(|file| file.take(MAX_INPUT as u64 + 1).read_to_end(&mut input))
            .map_err(|failure| {
                (
                    2,
                    json!({ "code": "SourceInputError", "message": failure.to_string() }),
                )
            })?;
        if input.len() > MAX_INPUT {
            return Err((
                6,
                json!({ "code": "SourceInputLimit", "message": "source input exceeds 256 KiB" }),
            ));
        }
        let decode_failure = |failure: serde_json::Error| {
            (
                2,
                json!({ "code": "InvalidSourceBundle", "message": failure.to_string() }),
            )
        };
        let describe_failure = |failure: luma::runtime::Failure| {
            let exit_code = match failure
                .diagnostics
                .first()
                .map(|(_, diagnostic)| diagnostic.code)
            {
                Some(
                    "SourceBundleLimit"
                    | "HostExpressionLimit"
                    | "HostPlanLimit"
                    | "ParameterExpressionLimit",
                ) => 6,
                Some("SourceDigestMismatch") => 5,
                Some("InvalidSourceBundle" | "UnsupportedSchemaVersion") => 2,
                _ => 3,
            };
            (
                exit_code,
                json!({
                    "code": if planning { "CadPlanFailed" } else { "CadCheckFailed" },
                    "diagnostics": failure.diagnostics.iter().map(|(file, diagnostic)| host::diagnostic_json(file, diagnostic)).collect::<Vec<_>>(),
                }),
            )
        };
        let bridge_failure = |error| match error {
            luma::living_ir::BridgeError::Preparation(cad::PreparationError::Execution(
                failure,
            )) => describe_failure(failure),
            luma::living_ir::BridgeError::Preparation(cad::PreparationError::Parameters(
                problem,
            )) => (3, json!({"code": problem.code, "message": problem.message})),
            luma::living_ir::BridgeError::Unsupported(message) => (
                3,
                json!({"code": if literal { "UnsupportedLivingIrCommand" } else { "UnsupportedLivingIrRecipe" }, "message": message}),
            ),
        };
        if literal {
            let request: luma::living_ir::CommandInput =
                serde_json::from_slice(&input).map_err(decode_failure)?;
            let bundle = request.source.admit().map_err(describe_failure)?;
            return luma::living_ir::prepare_literal(&bundle, request.context)
                .map_err(bridge_failure);
        }
        let wire =
            serde_json::from_slice::<host::WireSourceBundle>(&input).map_err(decode_failure)?;
        let bundle = wire.admit().map_err(describe_failure)?;
        if bridge {
            return luma::living_ir::prepare_recipe(&bundle).map_err(bridge_failure);
        }
        if planning {
            let plan = cad::prepare_geometry(&bundle, &Default::default(), 100_000).map_err(
                |problem| match problem {
                    cad::PreparationError::Execution(failure) => describe_failure(failure),
                    cad::PreparationError::Parameters(problem) => {
                        let status = if problem.code.ends_with("Limit") {
                            6
                        } else {
                            3
                        };
                        (
                            status,
                            json!({ "code": problem.code, "message": problem.message }),
                        )
                    }
                },
            )?;
            return serde_json::to_value(plan).map_err(|problem| {
                (
                    5,
                    json!({ "code": "GeometryPlanEncoding", "message": problem.to_string() }),
                )
            });
        }
        let session =
            luma::cad_engine::GeometryHost::new(Default::default()).map_err(|problem| {
                (
                    5,
                    json!({ "code": problem.code, "message": problem.message }),
                )
            })?;
        host::check_with_host(&bundle, mode, session.registry()).map_err(describe_failure)?;
        let profile = match mode {
            host::Mode::Recipe => luma::cad_engine::GEOMETRY_PROFILE,
            host::Mode::Command => "units-v1",
        };
        Ok(json!({ "valid": true, "mode": mode, "profile": profile }))
    };
    let protocol = if literal {
        "luma.cad.living-ir-command/1"
    } else if bridge {
        "luma.cad.living-ir-plan/1"
    } else if planning {
        "luma.cad.geometry-plan/1"
    } else {
        "luma.cad.host-check/1"
    };
    let mut envelope = json!({ "protocol": protocol, "schema_version": 1 });
    let (code, field, value) = match report() {
        Ok(result) => (0, "result", result),
        Err((code, error)) => (code, "error", error),
    };
    envelope[field] = value;
    println!("{envelope}");
    ExitCode::from(code)
}

fn usage() -> (u8, serde_json::Value) {
    (
        2,
        json!({ "code": "CadCheckUsage", "message": "Usage: luma cad check BUNDLE.json --mode recipe|command, luma cad plan|living-ir BUNDLE.json --mode recipe, or luma cad living-ir-command INPUT.json --mode command" }),
    )
}
