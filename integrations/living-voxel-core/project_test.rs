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
        self.apply(DocumentAction::Editor(command));
    }

    fn apply(&mut self, action: DocumentAction) {
        let registry = self.store.registry().unwrap();
        self.store
            .apply_document(
                &DocumentCommand {
                    request_id: format!("{:032x}", registry.revision + 1),
                    document_id: PART.into(),
                    expected_project_revision: registry.revision,
                    expected_document_revision: registry.documents[PART].revision,
                    action,
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
fn rejects_active_lattice_before_creating_a_voxel_job() {
    let provider = std::env::var("LUMA_VOXEL_PROVIDER").expect("absolute path to the Voxel CLI");
    let mut fixture = Fixture::new();
    let stock = "00000000-0000-4000-8000-000000000031";
    fixture.apply(
        serde_json::from_value(json!({"primitive":{
            "document_id":PART,"feature_id":stock,
            "geometry":{"kind":"create_plate","width":100.0,"height":60.0,"thickness":8.0}
        }}))
        .unwrap(),
    );
    fixture.apply(
        serde_json::from_value(json!({"lattice":{
            "document_id":PART,"feature_id":"00000000-0000-4000-8000-000000000032",
            "base":stock,"update":false,"intent":{
                "version":1,"region":{"min_m":[-0.04,-0.025,-0.004],"max_m":[0.04,0.025,0.004]},
                "cell":{"gyroid":{"threshold":0.2}},"cell_size_m":0.005,"origin_m":[0.0,0.0,0.0],
                "rotation_xyzw":[0.0,0.0,0.0,1.0],"shell_m":0.001,
                "exclusions":[null,null,null,null],"max_cells":100000,"max_sample_work":8192
            }
        }}))
        .unwrap(),
    );
    let head = fixture.store.with_part_head(PART, |head| head).unwrap();
    let grid = fixture.dir.path().join("grid.json");
    let example: Value = serde_json::from_slice(include_bytes!("sphere.json")).unwrap();
    fs::write(&grid, example["grid"].to_string()).unwrap();
    let error = fixture.call(
        &[
            "submit",
            &provider,
            REQUEST,
            PART,
            &head.native_revision.to_string(),
            &head.definition_hash,
            grid.to_str().unwrap(),
        ],
        2,
    );
    assert_eq!(error["code"], "UnsupportedGeometry");
    assert_eq!(fixture.call(&["receipt", REQUEST], 2)["code"], "UnknownJob");
    assert!(fixture.call(&["work", &provider], 0).is_null());
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
    let cli = std::env::var("LUMA_VOXEL_CLI").expect("absolute path to pinned upstream voxel-cli");
    let second = fixture.call(
        &[
            "submit",
            &provider,
            "00000000-0000-4000-8000-000000000013",
            PART,
            &fixture.call(&["source", PART], 0)["revision"].to_string(),
            fixture.call(&["source", PART], 0)["definition_hash"]
                .as_str()
                .unwrap(),
            grid.to_str().unwrap(),
        ],
        0,
    )["job"]
        .to_string();
    assert_eq!(fixture.call(&["work", &provider], 0)["state"], "published");
    assert_eq!(
        fixture.call(
            &[
                "field-csg",
                &cli,
                "difference",
                &second,
                &job,
                "exact-distance"
            ],
            2
        )["code"],
        "UnsupportedCapability"
    );
    let csg = fixture.call(
        &["field-csg", &cli, "difference", &second, &job, "implicit"],
        0,
    );
    let channel = &csg["result"]["field"]["channels"][0];
    assert_eq!(channel["semantic_kind"], "ImplicitScalar");
    assert_eq!(channel["units"], "m");
    assert_eq!(channel["iso"], 0.0);
    assert!(channel["blocks"].as_array().unwrap().iter().any(|block| {
        block["data"]["samples"].as_array().is_some_and(|samples| {
            samples
                .iter()
                .any(|sample| sample["metric_status"] == "NotDistance")
        })
    }));
    assert_eq!(
        csg["sources"][0]["report"]["part_source"]["radius_m"],
        0.005
    );
    assert_eq!(csg["sources"][1]["report"]["part_source"], pin);
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

#[test]
#[cfg(target_os = "linux")]
fn project_mask_edits_and_paged_gc_preserve_historical_sources() {
    use sha2::{Digest, Sha256};

    let provider =
        std::env::var("LUMA_VOXEL_CLI").expect("absolute path to pinned upstream voxel-cli");
    let mut fixture = Fixture::new();
    fixture.edit(json!({"op":"add","kind":"sphere"}));
    let pin = fixture.call(&["source", PART], 0);
    let revision = pin["revision"].as_u64().unwrap().to_string();
    let hash = pin["definition_hash"].as_str().unwrap();
    let path = |name: &str| fixture.dir.path().join(name).to_str().unwrap().to_owned();
    let db = path("publications.db");
    let storage = path("objects");
    let mask =
        |args: &[&str], code| fixture.call(&[&["mask", &db, &provider], args].concat(), code);
    let paged = |args: &[&str], code| {
        fixture.call(&[&["paged", &db, &provider, &storage], args].concat(), code)
    };
    let write = |name, value: Value| {
        let file = path(name);
        fs::write(&file, value.to_string()).unwrap();
        file
    };
    let bool_block = json!({
        "kind":"Constant",
        "sample":{"state":"Known","metric_status":"NotDistance","value":{"dtype":"Bool","value":true}}
    });
    let grid = json!({
        "layout":"CellCenter", "index_min":[0,0,0], "index_max":[40,8,8], "origin_m":[0.0,0.0,0.0],
        "axes":[[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]], "spacing_m":[0.001,0.001,0.001], "block_edge":8
    });
    let field = write(
        "mask.json",
        json!({
            "schema":{"id":"living.voxel.resident-field","version":1},
            "numeric_profile":{"id":"grid-si-v1","version":1},"grid":grid,
            "channels":[
                {"id":"mask","dtype":"Bool","semantic_kind":"CellUnionMask","units":"1",
                 "background":{"state":"Unknown"},"blocks":[]},
                {"id":"material","dtype":"U16","semantic_kind":"MaterialLabel","units":"1",
                 "material_labels":[7,11],
                 "background":{"state":"Known","metric_status":"NotDistance","value":{"dtype":"U16","value":7}},
                 "blocks":[{"coord":[0,0,0],"data":{"kind":"Constant",
                     "sample":{"state":"Known","metric_status":"NotDistance","value":{"dtype":"U16","value":11}}}}]}
            ]
        }),
    );
    let lvx = path("mask.lvx");
    let encoded = Command::new(&provider)
        .args(["encode", &field, &lvx])
        .output()
        .unwrap();
    assert!(
        encoded.status.success(),
        "{}",
        String::from_utf8_lossy(&encoded.stdout)
    );
    let digest = format!("{:x}", Sha256::digest(fs::read(&lvx).unwrap()));
    let job = |id| format!("00000000-0000-4000-8000-{id:012}");
    let (base, neighbor, halo, far, undo, stale, copy) = (
        job(21),
        job(22),
        job(23),
        job(24),
        job(25),
        job(26),
        job(27),
    );
    let seed = [
        "seed", &base, PART, &revision, hash, "mask", "0", &lvx, &digest,
    ];
    let initial = mask(&seed, 0);
    assert_eq!(initial["state"], "published");
    assert_eq!(mask(&seed, 0), initial);

    let replace = |name, block| {
        write(
            name,
            json!({
                "operation":"replace","channel":"mask",
                "edits":[{"coord":[block,0,0],"data":bool_block}]
            }),
        )
    };
    let near_edit = replace("near.json", 1);
    let far_edit = replace("far.json", 4);
    let halo_edit = write(
        "halo.json",
        json!({
            "operation":"copy_face","channel":"mask","from":[1,0,0],"to":[0,0,0]
        }),
    );
    let accepted = mask(&["patch", &neighbor, &base, &near_edit], 0);
    assert_eq!(accepted["binding"]["patch"]["scope"], "pointwise");
    assert_eq!(
        mask(&["patch", &halo, &base, &halo_edit], 4)["code"],
        "InputChanged"
    );
    assert_eq!(
        mask(&["patch", &stale, &base, &near_edit], 4)["code"],
        "InputChanged"
    );
    let disjoint = mask(&["patch", &far, &base, &far_edit], 0);
    assert_eq!(disjoint["binding"]["patch"]["predecessor_job"], neighbor);
    assert_eq!(mask(&["patch", &neighbor, &base, &near_edit], 0), accepted);
    assert_eq!(
        mask(&["patch", &neighbor, &base, &far_edit], 4)["code"],
        "RequestIdReuse"
    );
    let reversed = mask(&["undo", &undo, &neighbor], 0);
    assert_eq!(reversed["binding"]["patch"]["undo_of"], neighbor);
    assert_eq!(mask(&["undo", &undo, &neighbor], 0), reversed);
    assert_eq!(
        mask(&["patch", &stale, &base, &near_edit], 4)["code"],
        "InputChanged"
    );
    assert_eq!(
        mask(&["head", PART, "mask"], 0),
        json!({"generation":4,"job_id":undo})
    );
    // Both blocks at X=3 and X=4 are in bounds. The locally computed copy
    // reads the prior LVX and records the neighboring block in its footprint.
    let clear_halo = write(
        "copy.json",
        json!({
            "operation":"copy_face","channel":"mask","from":[4,0,0],"to":[3,0,0]
        }),
    );
    let copied = mask(&["patch", &copy, &undo, &clear_halo], 0);
    assert_eq!(copied["binding"]["patch"]["scope"], "face_halo");
    assert!(copied["binding"]["patch"]["reads"]
        .as_array()
        .unwrap()
        .iter()
        .any(|read| { read["coord"] == json!([4, 0, 0]) && read["sha256"].is_array() }));
    let boolean = mask(&["boolean", "or", &copy, &undo, "mask"], 0);
    assert_eq!(boolean["result"]["field"]["grid"], grid);
    let channel = &boolean["result"]["field"]["channels"][0];
    assert_eq!(channel["semantic_kind"], "CellUnionMask");
    assert_eq!(channel["background"]["state"], "Unknown");
    assert_eq!(boolean["sources"][1], reversed);

    let limits = |file, max| {
        write(
            file,
            json!({
                "max_objects":max,"max_object_bytes":65_536,"max_total_bytes":2_097_152
            }),
        )
    };
    let full = limits("limits.json", 256);
    let low = limits("low.json", 0);
    let indices = write(
        "points.json",
        json!([[0, 0, 0], [8, 0, 0], [32, 0, 0], [40, 0, 0]]),
    );
    assert_eq!(paged(&["pack", &undo, &low], 2)["code"], "LimitExceeded");
    assert_eq!(
        paged(&["query", &undo, "mask", &indices, &full], 2)["code"],
        "NoSources"
    );
    let original_root = paged(&["pack", &base, &full], 0)["root"].clone();
    let root = paged(&["pack", &undo, &full], 0)["root"].clone();
    assert_ne!(original_root, root);
    let observations = paged(&["query", &undo, "mask", &indices, &full], 0);
    assert_eq!(observations["grid"], grid);
    assert_eq!(observations["root"], root);
    assert_eq!(
        observations["observations"][1]["sample"]["state"],
        "Unknown"
    );
    assert_eq!(
        observations["observations"][2]["sample"]["value"]["value"],
        true
    );
    assert_eq!(observations["observations"][3]["state"], "OutsideDomain");
    let material = paged(&["query", &undo, "material", &indices, &full], 0);
    assert_eq!(material["observations"][0]["sample"]["value"]["value"], 11);
    assert_eq!(material["observations"][1]["sample"]["value"]["value"], 7);
    let root_file: Value = serde_json::from_slice(
        &fs::read(Path::new(&storage).join(root.as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let mask_index = root_file["indices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["channel"] == "mask")
        .unwrap();
    let mask_page = Path::new(&storage).join(mask_index["page"]["digest"].as_str().unwrap());
    let held_page = path("mask-page");
    fs::rename(&mask_page, &held_page).unwrap();
    let incomplete = paged(&["query", &undo, "mask", &indices, &full], 0);
    assert_eq!(incomplete["observations"][2]["state"], "Unloaded");
    fs::rename(&held_page, &mask_page).unwrap();

    let orphan = |bytes: &[u8]| {
        let name = format!("{:x}", Sha256::digest(bytes));
        fs::write(Path::new(&storage).join(&name), bytes).unwrap();
        name
    };
    let first = orphan(b"abandoned object");
    let plan = paged(&["gc", &full], 0);
    let second = orphan(b"additional abandoned object");
    assert_eq!(
        paged(&["gc", &full, plan["plan_digest"].as_str().unwrap()], 4)["code"],
        "InputChanged"
    );
    let plan = paged(&["gc", &full], 0);
    for retained in [&original_root, &root] {
        assert!(plan["plan"]["retained_roots"]
            .as_array()
            .unwrap()
            .contains(retained));
    }
    let plan_digest = plan["plan_digest"].as_str().unwrap();
    assert_eq!(
        paged(&["gc", &low, plan_digest], 2)["code"],
        "LimitExceeded"
    );
    for orphan in [&first, &second] {
        assert!(Path::new(&storage).join(orphan).is_file());
    }
    let gc = paged(&["gc", &full, plan_digest], 0);
    let quarantine = Path::new(gc["quarantine"].as_str().unwrap());
    for orphan in [&first, &second] {
        assert!(quarantine.join(orphan).is_file());
        assert!(!Path::new(&storage).join(orphan).exists());
    }
    assert_eq!(
        paged(&["query", &undo, "mask", &indices, &full], 0),
        observations
    );
    assert_eq!(
        paged(&["query", &base, "mask", &indices, &full], 0)["root"],
        original_root
    );
    let history = path("undo.lvx");
    assert_eq!(mask(&["export", &undo, &history], 0), reversed);
    assert_eq!(mask(&["head", PART, "mask"], 0)["job_id"], copy);
}
