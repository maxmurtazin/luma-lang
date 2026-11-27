use hir_transaction::PreparationContext;
use living_cad_viewer::checkpoint::project::{
    CreateDocument, DocumentAction, DocumentCommand, DocumentKind, ProjectStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
const PART: &str = "00000000-0000-4000-8000-000000000002";
const REQUEST: &str = "00000000-0000-4000-8000-000000000011";
const KEY: [u8; 32] = [0x19; 32];

struct Fixture {
    dir: tempfile::TempDir,
    store: ProjectStore,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(&dir.path().join("model.db"), PROJECT, KEY).unwrap();
        store
            .create_document(
                &CreateDocument {
                    request_id: format!("{:032x}", 1),
                    document_id: PART.into(),
                    expected_project_revision: 0,
                    kind: DocumentKind::Part,
                    name: "Sphere".into(),
                },
                &PreparationContext::default(),
            )
            .unwrap();
        Self { dir, store }
    }

    fn edit(&mut self, command: Value) {
        let registry = self.store.registry().unwrap();
        self.store
            .apply_document(
                &DocumentCommand {
                    request_id: format!("{:032x}", registry.revision + 1),
                    document_id: PART.into(),
                    expected_project_revision: registry.revision,
                    expected_document_revision: registry.documents[PART].revision,
                    action: DocumentAction::Editor(command),
                },
                self.dir.path(),
                &PreparationContext::default(),
            )
            .unwrap();
    }

    fn run(&self, args: &[&str], crash: Option<&str>) -> Output {
        let mut command = self.command(args);
        if let Some(crash) = crash {
            command.env("LUMA_CAD_TEST_CRASH_AT", crash);
        }
        command.output().unwrap()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luma-cad-voxel-project"));
        command
            .arg(self.dir.path().join("model.db"))
            .arg(PROJECT)
            .args(args)
            .env("LUMA_CAD_PROJECT_KEY", hex::encode(KEY))
            .env_remove("LUMA_CAD_TEST_CRASH_AT");
        command
    }

    fn call(&self, args: &[&str], code: i32) -> Value {
        let output = self.run(args, None);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{args:?} {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        response[if code == 0 { "result" } else { "error" }].clone()
    }
}

#[test]
fn publishes_historical_sphere_and_recovers_cancelled_attempt() {
    let provider = std::env::var("LUMA_VOXEL_PROVIDER").expect("absolute path to the Voxel CLI");
    let mut fixture = Fixture::new();
    fixture.edit(json!({"op":"add","kind":"sphere"}));
    let node = fixture
        .store
        .voxel_sphere_source(PART, None)
        .unwrap()
        .entity;
    fixture.edit(json!({"op":"properties","node":node,"values":{"radius":4}}));
    let pin = fixture.call(&["source", PART], 0);
    assert_eq!(pin["radius_m"], 0.004);
    let grid = fixture.dir.path().join("grid.json");
    let request: Value = serde_json::from_slice(include_bytes!("sphere.json")).unwrap();
    fs::write(&grid, request["grid"].to_string()).unwrap();
    let revision = pin["revision"].as_u64().unwrap().to_string();
    let hash = pin["definition_hash"].as_str().unwrap();
    let command = fixture.dir.path().join("sample.json");
    let wire = include_bytes!("sample-command.json");
    let command_digest = serde_json::from_slice::<Value>(wire).unwrap()["source_digest"].clone();
    fs::write(&command, wire).unwrap();
    let submit = [
        "submit-luma",
        &provider,
        REQUEST,
        PART,
        revision.as_str(),
        hash,
        grid.to_str().unwrap(),
        command.to_str().unwrap(),
    ];
    let delivery = fixture.call(&submit, 0);
    let job = delivery["job"].as_i64().unwrap().to_string();
    assert_eq!(fixture.call(&submit, 0), delivery);
    assert_eq!(fixture.call(&["receipt", REQUEST], 0), delivery);
    fixture.edit(json!({"op":"properties","node":node,"values":{"radius":5}}));

    let attempt = if cfg!(feature = "checkpoint-fault-injection") {
        assert_eq!(
            fixture
                .run(&["work", &provider], Some("voxel-job-before-publication"))
                .status
                .code(),
            Some(73)
        );
        assert_eq!(fixture.call(&["status", &job], 0)["state"], "running");
        assert_eq!(
            fixture.call(&["export", &job, "unused.lvx"], 2)["code"],
            "JobPending"
        );
        assert_eq!(fixture.call(&["cancel", &job], 0)["state"], "cancelled");
        assert_eq!(fixture.call(&["retry", &job, "0"], 4)["code"], "Conflict");
        1
    } else {
        assert_eq!(fixture.call(&["cancel", &job], 0)["state"], "cancelled");
        0
    };
    assert!(fixture.call(&["work", &provider], 0).is_null());
    assert_eq!(
        fixture.call(&["retry", &job, &attempt.to_string()], 0)["state"],
        "admitted"
    );
    let published = fixture.call(&["work", &provider], 0);
    assert_eq!(published["state"], "published");
    assert_eq!(published["attempt"], attempt + 1);
    let output = fixture.dir.path().join("sphere.lvx");
    let exported = fixture.call(&["export", &job, output.to_str().unwrap()], 0);
    assert_eq!(exported["report"]["part_source"], pin);
    assert_eq!(exported["report"]["command_digest"], command_digest);
    assert_eq!(exported["report"]["known_samples"], 4913);
    assert!(Path::new(&output).is_file());
    assert_eq!(
        fixture.call(&["export", &job, output.to_str().unwrap()], 2)["code"],
        "IoFailure"
    );
    assert_eq!(fixture.call(&["source", PART], 0)["radius_m"], 0.005);
    if let Some(dir) = std::env::var_os("LUMA_VOXEL_EVIDENCE") {
        let output = Path::new(&dir).join("sphere.lvx");
        let report = fixture.call(&["export", &job, output.to_str().unwrap()], 0);
        println!("{}", report["report"]["digests"]["blob_digest"]);
    }

    #[cfg(unix)]
    {
        use std::{
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };
        let blocking = fixture.dir.path().join("blocking-provider");
        fs::write(&blocking, "#!/bin/sh\nexec sleep 20\n").unwrap();
        fs::set_permissions(&blocking, fs::Permissions::from_mode(0o700)).unwrap();
        let blocking = blocking.to_str().unwrap();
        let submitted = fixture.call(
            &[
                "submit",
                blocking,
                "00000000-0000-4000-8000-000000000012",
                PART,
                &revision,
                hash,
                grid.to_str().unwrap(),
            ],
            0,
        );
        let id = submitted["job"].as_i64().unwrap().to_string();
        assert!(fixture.call(&["work", &provider], 0).is_null());
        let started = Instant::now();
        let mut worker = fixture
            .command(&["work", blocking])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        while fixture.call(&["status", &id], 0)["state"] != "running" {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fixture.call(&["cancel", &id], 0)["state"], "cancelled");
        assert_eq!(fixture.call(&["retry", &id, "1"], 0)["state"], "admitted");
        assert_eq!(worker.wait().unwrap().code(), Some(4));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(
            fixture.call(&["export", &id, output.to_str().unwrap()], 2)["code"],
            "JobPending"
        );
    }
}
