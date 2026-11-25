use luma::{host, host::Dispatch};
use serde_json::json;
use std::{fs::File, io::Read, process::ExitCode};

const MAX_INPUT: usize = 256 * 1024;

pub fn run(args: &[String]) -> ExitCode {
    let report = || -> Result<_, (u8, serde_json::Value)> {
        let [command, path, flag, mode] = args else {
            return Err(usage());
        };
        if command != "check" || flag != "--mode" {
            return Err(usage());
        }
        let mode = match mode.as_str() {
            "recipe" => host::Mode::Recipe,
            "command" => host::Mode::Command,
            _ => return Err(usage()),
        };
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
        let wire = serde_json::from_slice::<host::WireSourceBundle>(&input).map_err(|failure| {
            (
                2,
                json!({ "code": "InvalidSourceBundle", "message": failure.to_string() }),
            )
        })?;
        let describe_failure = |failure: luma::runtime::Failure| {
            let exit_code = match failure
                .diagnostics
                .first()
                .map(|(_, diagnostic)| diagnostic.code)
            {
                Some("SourceBundleLimit") => 6,
                Some("SourceDigestMismatch") => 5,
                Some("InvalidSourceBundle" | "UnsupportedSchemaVersion") => 2,
                _ => 3,
            };
            (
                exit_code,
                json!({
                    "code": "CadCheckFailed",
                    "diagnostics": failure.diagnostics.iter().map(|(file, diagnostic)| host::diagnostic_json(file, diagnostic)).collect::<Vec<_>>(),
                }),
            )
        };
        let bundle = wire.admit().map_err(describe_failure)?;
        let units = luma_cad_contract::UnitsHost::new();
        host::check_with_host(&bundle, mode, units.registry()).map_err(describe_failure)?;
        Ok(json!({ "valid": true, "mode": mode, "profile": "units-v1" }))
    };
    let mut envelope = json!({ "protocol": "luma.cad.host-check/1", "schema_version": 1 });
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
        json!({ "code": "CadCheckUsage", "message": "Usage: luma cad check BUNDLE.json --mode recipe|command" }),
    )
}
