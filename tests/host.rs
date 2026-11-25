use luma::{
    host::{check_with_host, run_with_host, SourceBundle},
    host_api::{
        CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
        Value,
    },
};
use luma_cad_contract::UnitsHost;
use std::{
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

fn bundle(source: &str) -> SourceBundle {
    SourceBundle {
        entry: "project/main.luma".into(),
        files: [("project/main.luma".into(), source.into())].into(),
    }
}

fn diagnostic(result: Result<impl Sized, luma::runtime::Failure>, code: &str) {
    let failure = result.err().expect("expected rejection");
    assert!(
        failure.diagnostics.iter().any(|(_, d)| d.code == code),
        "expected {code}, got {failure:?}",
    );
}

#[test]
fn same_registry_checks_and_runs_each_native_instantiation() {
    let host = UnitsHost::new();
    let mut source = bundle(
        "use units\nuse helper\nlength = helper.twice(units.cm(5))\nangle = units.add(units.deg(90), units.rad(0))\ntime = units.subtract(units.seconds(8), units.seconds(3))\nprint(7)",
    );
    source.files.insert(
        "project/helper.luma".into(),
        "use units\npub fn twice(value)\nreturn units.add(value, value)\nend".into(),
    );
    for mode in [Mode::Recipe, Mode::Command] {
        let result = run_with_host(&source, mode, &host, 100).unwrap();
        for (key, name, expected) in [
            ("length", "Quantity<Length>", 0.1),
            ("angle", "Quantity<Angle>", std::f64::consts::FRAC_PI_2),
            ("time", "Quantity<Time>", 5.0),
        ] {
            let Value::Opaque { type_name, payload } = &result.outputs[key] else {
                panic!("expected opaque quantity");
            };
            assert_eq!(type_name, name);
            assert_eq!(payload.as_str().unwrap().parse::<f64>().unwrap(), expected);
        }
        assert_eq!(result.diagnostic_output, "7\n");
        assert!(result.calls.is_empty());
        assert!(result.expression_steps > 0);
    }
}

#[test]
fn rejects_quantity_errors_and_nonfinite_results() {
    let host = UnitsHost::new();
    let registry = host.registry();
    for (text, code) in [
        ("units.add(units.mm(1), units.deg(2))", "E_TYPE"),
        ("units.add(1, 2)", "HostTypeMismatch"),
        ("units.mm(\"100\")", "E_TYPE"),
        ("units.mm(1, 2)", "E_ARITY"),
        (
            "units.multiply(units.mm(1), units.mm(2))",
            "UnsupportedCapability",
        ),
        ("units.mm", "E_TYPE"),
    ] {
        diagnostic(
            check_with_host(
                &bundle(&format!("use units\nx = {text}")),
                Mode::Recipe,
                registry,
            ),
            code,
        );
    }
    diagnostic(
        run_with_host(
            &bundle("use units\nx = units.add(units.m(10^308), units.m(10^308))"),
            Mode::Recipe,
            &host,
            100,
        ),
        "NonFiniteQuantity",
    );
    diagnostic(
        run_with_host(
            &bundle("use units\nx = units.mm(1)"),
            Mode::Recipe,
            &host,
            1,
        ),
        "HostExpressionLimit",
    );
}

#[test]
fn imports_admitted_files_and_rejects_reserved_collisions() {
    let host = UnitsHost::new();
    let mut source = bundle("use units\nx = units.mm(1)");
    source
        .files
        .insert("project/units.luma".into(), "pub x = 1".into());
    diagnostic(
        check_with_host(&source, Mode::Recipe, host.registry()),
        "ModuleNameCollision",
    );

    diagnostic(
        check_with_host(&bundle("use example"), Mode::Recipe, host.registry()),
        "E_IMPORT",
    );
    let mut source = bundle("x = 1");
    source
        .files
        .insert("project/../outside.luma".into(), "y = 2".into());
    diagnostic(
        check_with_host(&source, Mode::Recipe, host.registry()),
        "InvalidSourceBundle",
    );

    let source = bundle(&"x = 1\n".repeat(700));
    diagnostic(
        check_with_host(&source, Mode::Recipe, host.registry()),
        "SourceBundleLimit",
    );
    assert!(serde_json::from_str::<luma::host::WireSourceBundle>(
        r#"{"bundle_version":1,"entry":"main.luma","files":{"main.luma":"x=1","main.luma":"x=2"},"source_digest":"example"}"#,
    )
    .is_err());
}

struct TestHost {
    registry: Registry,
    calls: AtomicUsize,
    bad_result: bool,
}

impl TestHost {
    fn new(effect: Effect, bad_result: bool) -> Self {
        let descriptor = Descriptor {
            module: "cad".into(),
            name: "stage".into(),
            operation: "cad.stage".into(),
            major: 1,
            parameters: vec![Parameter::Exact(Type::Number)],
            result: ResultType::Exact(Type::Number),
            effect,
            modes: [Mode::Recipe].into(),
        };
        Self {
            registry: Registry::new(["cad".into()], [descriptor]).unwrap(),
            calls: AtomicUsize::new(0),
            bad_result,
        }
    }
}

impl Dispatch for TestHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }
    fn call(
        &self,
        _: &Descriptor,
        _: Mode,
        _: &CallSite,
        arguments: &[Value],
    ) -> Result<Value, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(if self.bad_result {
            Value::Bool(false)
        } else {
            arguments[0].clone()
        })
    }
}

#[test]
fn mismatched_registry_rejects_before_dispatch_and_mode_or_effect_is_checked() {
    let source = bundle("use cad\nx = cad.stage(42)");
    let planning = TestHost::new(Effect::BuildPlan, false);
    let pure = TestHost::new(Effect::Pure, false);
    let checked = check_with_host(&source, Mode::Recipe, planning.registry()).unwrap();
    diagnostic(checked.execute(&pure, 100), "HostRegistryMismatch");
    assert_eq!(pure.calls.load(Ordering::Relaxed), 0);
    diagnostic(
        run_with_host(&source, Mode::Command, &planning, 100),
        "HostModeMismatch",
    );

    let parallel = bundle("use cad\nfn stage(x)\nreturn cad.stage(x)\nend\nvalues = parallel([1,2], stage)\nprint(values)");
    diagnostic(
        run_with_host(&parallel, Mode::Recipe, &planning, 100),
        "E_EFFECT",
    );
    assert_eq!(planning.calls.load(Ordering::Relaxed), 0);
    let result = run_with_host(&parallel, Mode::Recipe, &pure, 100).unwrap();
    assert_eq!(result.diagnostic_output, "[1, 2]\n");
    assert_eq!(pure.calls.load(Ordering::Relaxed), 2);

    let result = run_with_host(&source, Mode::Recipe, &planning, 100).unwrap();
    assert_eq!(result.outputs["x"], Value::Number(42.0));
    assert_eq!(result.calls.len(), 1);
    assert_eq!(result.calls[0].operation, "cad.stage");
    assert_eq!(result.calls[0].arguments, [Value::Number(42.0)]);
    assert_eq!(result.calls[0].site.file, "project/main.luma");
    assert_eq!(result.calls[0].site.start, "use cad\nx = ".len());

    let broken = TestHost::new(Effect::BuildPlan, true);
    diagnostic(
        run_with_host(&source, Mode::Recipe, &broken, 100),
        "HostResultMismatch",
    );
}

#[test]
fn cli_returns_one_json_outcome_for_explicit_mode() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/cad/units-check.json");
    let run = |mode| {
        Command::new(env!("CARGO_BIN_EXE_luma"))
            .args(["cad", "check", path, "--mode", mode])
            .output()
            .unwrap()
    };
    let passed = run("recipe");
    assert!(passed.status.success());
    let result: serde_json::Value = serde_json::from_slice(&passed.stdout).unwrap();
    assert_eq!(result["result"]["valid"], true);
    assert!(result.get("error").is_none());
    let failed = run("auto");
    assert_eq!(failed.status.code(), Some(2));
    let result: serde_json::Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(result["error"]["code"], "CadCheckUsage");
    assert!(result.get("result").is_none());
}

#[test]
fn wire_source_admission_matches_cad0_lcan_vectors() {
    let vectors: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../conformance/cad/lcan-vectors-v1.json")).unwrap();
    let mut admitted = 0;
    for vector in vectors
        .iter()
        .filter(|vector| vector["domain"] == "luma.cad.source")
    {
        let wire = serde_json::json!({
            "bundle_version": 1,
            "entry": vector["typed"]["entry"],
            "files": vector["typed"]["files"],
            "source_digest": vector["digest"],
        });
        let parse = |value| serde_json::from_value::<luma::host::WireSourceBundle>(value).unwrap();
        assert!(parse(wire.clone()).admit().is_ok());
        admitted += 1;

        let mut edited = wire.clone();
        edited["files"]["main.luma"] = "x = 1\n".into();
        diagnostic(parse(edited).admit(), "SourceDigestMismatch");

        let mut version = wire;
        version["bundle_version"] = 2.into();
        diagnostic(parse(version).admit(), "UnsupportedSchemaVersion");
    }
    assert_eq!(admitted, 2);

    let files = [
        ("project/écran.luma".into(), "# 𝄞\t\\\"".into()),
        ("project/main.luma".into(), "# e\u{301}\r\n".into()),
    ]
    .into();
    assert_eq!(
        luma_cad_contract::source_digest("project/main.luma", &files),
        "sha256:d7c639fdd1bd895cf85cd1afd573120f8a7e5613ee21c8db58d08b7a18b9126b",
    );
}
