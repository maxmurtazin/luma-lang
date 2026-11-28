use hir_transaction::PreparationContext;
use living_cad_viewer::checkpoint::project::{
    CreateDocument, DocumentAction, DocumentCommand, DocumentKind, ProjectStore,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet, f64::consts::PI, path::PathBuf, process::Command, time::SystemTime,
};

const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
const PART: &str = "00000000-0000-4000-8000-000000000002";
const FEATURE: &str = "00000000-0000-4000-8000-000000000003";
const FIRST: &str = "00000000-0000-4000-8000-000000000004";
const SECOND: &str = "00000000-0000-4000-8000-000000000005";
const KEY: [u8; 32] = [19; 32];

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("luma-cad2-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }

    fn model(&self) -> PathBuf {
        self.0.join("model.db")
    }

    fn output(&self) -> PathBuf {
        self.0.join("derived.db")
    }

    fn call(&self, args: &[&str], code: i32) -> Value {
        response(self.command().args(args).output().unwrap(), code)
    }

    fn command(&self) -> Command {
        let key: String = KEY.into_iter().map(|byte| format!("{byte:02x}")).collect();
        let mut process = Command::new(env!("CARGO_BIN_EXE_luma-cad-brep"));
        process
            .arg(self.model())
            .arg(PROJECT)
            .arg(self.output())
            .env("LUMA_CAD_PROJECT_KEY", key);
        process
    }

    #[cfg(all(feature = "luma-tasks", feature = "checkpoint-fault-injection"))]
    fn crash(&self, stage: &str, args: &[&str]) {
        let process = self
            .command()
            .env("LUMA_CAD_TEST_CRASH_AT", stage)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(process.status.code(), Some(73), "{process:?}");
        assert!(process.stdout.is_empty(), "{process:?}");
    }

    fn apply(&self, store: &mut ProjectStore, action: DocumentAction) {
        self.apply_part(store, PART, action);
    }

    fn apply_part(&self, store: &mut ProjectStore, part: &str, action: DocumentAction) {
        let registry = store.registry().unwrap();
        store
            .apply_document(
                &DocumentCommand {
                    request_id: format!("{:032x}", registry.revision + 1),
                    document_id: part.into(),
                    expected_project_revision: registry.revision,
                    expected_document_revision: registry.documents[part].revision,
                    action,
                },
                &self.0,
                &PreparationContext::default(),
            )
            .unwrap();
    }

    fn publish(&self, pin: &Value, job: &str, code: i32) -> Value {
        self.call(
            &[
                "publish",
                PART,
                FEATURE,
                pin["revision"].as_str().unwrap(),
                pin["definition_hash"].as_str().unwrap(),
                job,
            ],
            code,
        )
    }
}

fn response(process: std::process::Output, code: i32) -> Value {
    assert_eq!(
        process.status.code(),
        Some(code),
        "{}\n{}",
        String::from_utf8_lossy(&process.stdout),
        String::from_utf8_lossy(&process.stderr)
    );
    let response: Value = serde_json::from_slice(&process.stdout).unwrap();
    assert_eq!(response["protocol"], "luma.cad.brep-project/1");
    response[if code == 0 { "result" } else { "error" }].clone()
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn corners_and_holes(artifact: &Value, height: f64) -> BTreeSet<String> {
    let area = 0.04 * 0.04 - PI * 0.005 * 0.005;
    let interval = artifact["volume_m3"].as_array().unwrap();
    let exact = area * height;
    assert!(interval[0].as_f64().unwrap() <= exact && exact <= interval[1].as_f64().unwrap());
    assert_eq!(artifact["report"]["status"], "Pass");
    assert_eq!(artifact["report"]["level"], "L3K1Solid");
    assert_eq!(artifact["hole_loops"].as_array().unwrap().len(), 1);
    artifact["outer_loop"]["segments"]
        .as_array()
        .unwrap()
        .iter()
        .chain(artifact["hole_loops"][0]["segments"].as_array().unwrap())
        .map(|edge| edge["source_id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn publishes_pinned_normal_extrusion_and_measures_historical_brep() {
    let fixture = Fixture::new();
    let mut store = ProjectStore::create(&fixture.model(), PROJECT, KEY).unwrap();
    store
        .create_document(
            &CreateDocument {
                request_id: format!("{:032x}", 1),
                document_id: PART.into(),
                expected_project_revision: 0,
                kind: DocumentKind::Part,
                name: "CAD-2 flange".into(),
            },
            &PreparationContext::default(),
        )
        .unwrap();
    assert_eq!(
        fixture.call(&["source", PART, FEATURE], 3)["code"],
        "UnsupportedGeometry"
    );
    assert_eq!(fixture.call(&["latest", PART, FEATURE], 0), Value::Null);
    assert_eq!(
        fixture.call(&["measure", FIRST], 3)["code"],
        "MissingPublication"
    );
    assert!(!fixture.output().exists());

    for edit in [
        json!({"op":"create","plane":"XY"}),
        json!({"op":"add","element":{"kind":"rectangle","a":{"xy":[-0.02,-0.02]},
            "b":{"xy":[0.02,0.02]}}}),
        json!({"op":"add","element":{"kind":"circle","center":{"xy":[0.0,0.0]},"radius_m":0.005}}),
    ] {
        fixture.apply(&mut store, DocumentAction::Sketch(edit));
    }
    let digest = store.part_sketch_digest(PART).unwrap().unwrap();
    fixture.apply(
        &mut store,
        DocumentAction::Extrude(json!({
            "op":"create_extrude","feature_id":FEATURE,"source_digest":digest,
            "length_m":"0.004","direction":"one_sided"
        })),
    );
    let first = fixture.call(&["source", PART, FEATURE], 0);
    let wire = fixture.call(&["sketch-profile", PART, FEATURE], 0);
    assert_eq!(wire["sketch"]["id"], first["sketch"]);
    assert_eq!(wire["profile"]["source"]["revision"], first["revision"]);
    assert_eq!(wire["source_definition_hash"], first["definition_hash"]);
    assert_eq!(wire["sketch"]["curves"].as_array().unwrap().len(), 5);
    let loops = wire["profile"]["loops"].as_array().unwrap();
    assert_eq!(loops.len(), 2);
    let outer = loops.iter().find(|item| item["role"] == "outer").unwrap();
    let inner = loops.iter().find(|item| item["role"] == "hole").unwrap();
    assert_eq!(inner["parent_id"], outer["id"]);
    let edges: BTreeSet<_> = loops
        .iter()
        .flat_map(|item| item["edges"].as_array().unwrap())
        .map(|edge| edge["curve_id"].as_str().unwrap().to_owned())
        .collect();
    let published = fixture.publish(&first, FIRST, 0);
    assert_eq!(published["state"], "published");
    assert_eq!(fixture.publish(&first, FIRST, 0), published);
    let artifact = fixture.call(&["measure", FIRST], 0);
    assert_eq!(artifact["receipt"], published["data"]);
    assert_eq!(artifact["provenance"]["sketch"], first["sketch"]);
    assert_eq!(
        artifact["provenance"]["job"]["project_definition_hash"],
        first["definition_hash"]
    );
    let ids = corners_and_holes(&artifact, 0.004);
    assert_eq!(ids, edges);
    let preview = fixture.call(&["view", FIRST], 0);
    let digest = preview["source"]["geometry_sha256"].as_str().unwrap();
    assert_eq!(preview["source"]["source_volume_m3"], artifact["volume_m3"]);
    assert_eq!(preview["source"]["source_revision"], first["revision"]);
    assert_eq!(
        preview["triangles"].as_array().unwrap().len(),
        preview["face_ordinals"].as_array().unwrap().len()
    );
    let distance = fixture.call(&["distance", FIRST, digest, "solid", "0", "0", "0.002"], 0);
    let limits = distance["distance_m"].as_array().unwrap();
    assert!(limits[0].as_f64().unwrap() <= 0.005 && limits[1].as_f64().unwrap() >= 0.005);
    let face = format!("face:{}", preview["face_ordinals"][0].as_u64().unwrap());
    let face_distance = fixture.call(&["distance", FIRST, digest, &face, "0", "0", "0.002"], 0);
    assert_eq!(face_distance["geometry_sha256"], digest);
    let section = |height| {
        json!({"origin_m":[0.,0.,height], "x_axis":[1.,0.,0.],
        "y_axis":[0.,1.,0.], "z_axis":[0.,0.,1.]})
        .to_string()
    };
    assert_eq!(
        fixture.call(&["section", FIRST, digest, &section(0.002)], 0)["kind"],
        "RegionContours"
    );
    assert_eq!(
        fixture.call(&["section", FIRST, digest, &section(0.005)], 0)["kind"],
        "Empty"
    );
    #[cfg(feature = "luma-tasks")]
    let queued = {
        let entry = "engineering.luma".to_string();
        let bundle = luma::host::SourceBundle {
            entry: entry.clone(),
            files: [(
                entry,
                r#"
use units
use brep
use cad
body = brep.published_body()
options = brep.task_options()
cad.mesh(body, options)
cad.section_task(body, brep.xy_section(units.mm(2)), options)
cad.query_task(body, brep.volume(), options)
cad.query_task(body, brep.distance(units.mm(0), units.mm(0), units.mm(2)), options)
"#
                .into(),
            )]
            .into(),
        };
        let file = fixture.0.join("engineering.json");
        let wire = json!({"bundle_version":1,"entry":bundle.entry,
                         "files":bundle.files,"source_digest":bundle.source_digest()});
        std::fs::write(&file, serde_json::to_vec(&wire).unwrap()).unwrap();
        let report = fixture.call(&["luma-tasks", FIRST, digest, file.to_str().unwrap()], 0);
        assert_eq!(report["geometry_sha256"], digest);
        assert_eq!(report["results"][0]["result"], preview);
        assert_eq!(report["results"][1]["result"]["kind"], "RegionContours");
        assert_eq!(
            report["results"][2]["result"]["volume_m3"],
            artifact["volume_m3"]
        );
        assert_eq!(report["results"][3]["result"], distance);
        assert_eq!(
            fixture.call(
                &[
                    "luma-tasks",
                    FIRST,
                    &"00".repeat(32),
                    file.to_str().unwrap()
                ],
                3
            )["code"],
            "StaleTopologyRef"
        );

        let request = "00000000-0000-4000-8000-000000000071";
        let submit = [
            "jobs-submit",
            request,
            FIRST,
            digest,
            file.to_str().unwrap(),
        ];
        #[cfg(feature = "checkpoint-fault-injection")]
        {
            fixture.crash("brep-job-before-admission", &submit);
            assert_eq!(
                fixture.call(&["jobs-receipt", request], 3)["code"],
                "UnknownJob"
            );
            fixture.crash("brep-job-after-admission", &submit);
        }
        let accepted = fixture.call(&submit, 0);
        assert_eq!(fixture.call(&submit, 0), accepted);
        assert_eq!(fixture.call(&["jobs-receipt", request], 0), accepted);
        let job = accepted["job"].as_i64().unwrap().to_string();
        assert_eq!(fixture.call(&["jobs-status", &job], 0)["state"], "admitted");
        assert_eq!(
            fixture.call(&["jobs-result", &job], 3)["code"],
            "JobPending"
        );
        let mut changed = bundle;
        let source = changed.files.get_mut(&changed.entry).unwrap();
        *source = source.replace("units.mm(2)", "units.mm(3)");
        let wire = json!({"bundle_version":1,"entry":changed.entry,
                         "files":changed.files,"source_digest":changed.source_digest()});
        std::fs::write(&file, serde_json::to_vec(&wire).unwrap()).unwrap();
        assert_eq!(fixture.call(&submit, 4)["code"], "RequestIdReuse");
        std::fs::remove_file(file).unwrap();
        (job, report)
    };
    let oblique = json!({"origin_m":[0.,0.,0.002],"x_axis":[1.,0.,0.],
        "y_axis":[0.,0.,1.],"z_axis":[0.,-1.,0.]})
    .to_string();
    assert_eq!(
        fixture.call(&["section", FIRST, digest, &oblique], 3)["code"],
        "UnsupportedBrepOperation"
    );
    assert_eq!(fixture.call(&["latest", PART, FEATURE], 0)["current"], true);

    fixture.apply(
        &mut store,
        DocumentAction::Extrude(json!({
            "op":"update_extrude_length","feature_id":FEATURE,"length_m":"0.008"
        })),
    );
    assert_eq!(
        fixture.call(&["latest", PART, FEATURE], 0)["current"],
        false
    );
    assert_eq!(fixture.publish(&first, SECOND, 4)["code"], "Conflict");
    assert_eq!(
        fixture.call(&["measure", SECOND], 3)["code"],
        "MissingPublication"
    );

    let second = fixture.call(&["source", PART, FEATURE], 0);
    assert_ne!(first["revision"], second["revision"]);
    assert_eq!(first["sketch"], second["sketch"]);
    assert_eq!(fixture.publish(&second, FIRST, 4)["code"], "RequestIdReuse");
    assert_eq!(fixture.publish(&second, SECOND, 0)["state"], "published");
    let next = fixture.call(&["measure", SECOND], 0);
    assert_eq!(corners_and_holes(&next, 0.008), ids);
    let next_wire = fixture.call(&["sketch-profile", PART, FEATURE], 0);
    assert_eq!(next_wire["sketch"], wire["sketch"]);
    assert_eq!(next_wire["profile"]["loops"], wire["profile"]["loops"]);
    assert_eq!(
        fixture.call(&["distance", SECOND, digest, &face, "0", "0", "0.002"], 3)["code"],
        "StaleTopologyRef"
    );
    assert_eq!(fixture.call(&["measure", FIRST], 0), artifact);
    #[cfg(feature = "luma-tasks")]
    {
        let (job, original) = queued;
        let previous_attempt = 0;
        #[cfg(feature = "checkpoint-fault-injection")]
        let previous_attempt = {
            let stages = ["brep-job-after-claim", "brep-job-before-publication"];
            for (attempt, stage) in (previous_attempt + 1..).zip(stages) {
                fixture.crash(stage, &["jobs-work"]);
                let processing = fixture.call(&["jobs-status", &job], 0);
                assert_eq!(processing["state"], "running");
                assert_eq!(processing["attempt"], attempt);
                assert_eq!(
                    fixture.call(&["jobs-result", &job], 3)["code"],
                    "JobPending"
                );
                assert_eq!(
                    fixture.call(&["jobs-cancel", &job], 0)["state"],
                    "cancelled"
                );
                assert_eq!(
                    fixture.call(&["jobs-retry", &job, &attempt.to_string()], 0)["state"],
                    "admitted"
                );
            }
            previous_attempt + stages.len()
        };
        let published = fixture.call(&["jobs-work"], 0);
        assert_eq!(published["state"], "published");
        assert_eq!(published["publication"], "Historical");
        assert_eq!(published["attempt"], previous_attempt + 1);
        assert_eq!(fixture.call(&["jobs-work"], 0), Value::Null);
        let recovered = fixture.call(&["jobs-result", &job], 0);
        assert_eq!(recovered["manifest"], published["artifact"]);
        assert_eq!(recovered["result"], original);
        assert_eq!(fixture.call(&["jobs-cancel", &job], 0), published);
        assert_eq!(
            fixture.call(
                &["jobs-retry", &job, &(previous_attempt + 1).to_string()],
                4
            )["code"],
            "Conflict"
        );
    }
    assert_eq!(
        fixture.call(&["latest", PART, FEATURE], 0)["receipt"],
        next["receipt"]
    );
    assert_eq!(
        store.registry().unwrap().documents[PART]
            .revision
            .to_string(),
        second["revision"]
    );
    if let Some(path) = std::env::var_os("LUMA_CAD2_EVIDENCE") {
        let directory = PathBuf::from(path);
        for (filename, value) in [
            ("sketch-profile.json", wire),
            ("brep-publication.json", preview),
        ] {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(filename))
                .unwrap();
            serde_json::to_writer(file, &value).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires pinned Living IR 2D checkout, renderer and font"]
fn publishes_associative_sheet_only_at_current_part_revision() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::Path,
        process::Stdio,
        thread,
        time::{Duration, Instant},
    };

    const SHEET: &str = "56000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000099";
    let env_path =
        |name| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    let renderer = env_path("LUMA_CAD2_LIR2D");
    let font = env_path("LUMA_CAD2_DRAWING_FONT");
    let fixtures = env_path("LUMA_CAD2_IR2D").join("fixtures");

    let fixture = Fixture::new();
    let context = PreparationContext::default();
    let mut store = ProjectStore::create(&fixture.model(), PROJECT, KEY).unwrap();
    for (revision, document, kind) in [
        (0, PART, DocumentKind::Part),
        (1, OTHER, DocumentKind::Assembly),
    ] {
        store
            .create_document(
                &CreateDocument {
                    request_id: format!("{:032x}", revision + 1),
                    document_id: document.into(),
                    expected_project_revision: revision,
                    kind,
                    name: "Drawing fixture".into(),
                },
                &context,
            )
            .unwrap();
    }
    let plate = |width| {
        serde_json::from_value(json!({
            "document_id":PART,"feature_id":FEATURE,
            "geometry":{"kind":"create_plate","width":width,"height":60.0,"thickness":8.0}
        }))
        .unwrap()
    };
    fixture.apply(&mut store, DocumentAction::Primitive(plate(100.0)));
    let outline =
        living_cad_features::derived_id(FEATURE.parse().unwrap(), "drawing/v1/top/bottom");
    let prepare_layout = |part: &str, output: &Path, anchor: &str| {
        let mut batch: Value = serde_json::from_slice(
            &fs::read(fixtures.join("sheet/create-core-plate-a4.json")).unwrap(),
        )
        .unwrap();
        let views = batch["commands"][0]["sheet"]["views"]
            .as_array_mut()
            .unwrap();
        for view in views.iter_mut() {
            view["core_projection"]["document_id"] = json!(part);
            view["core_projection"]["feature_id"] = json!(FEATURE);
        }
        views[0]["projection_dimensions"] = json!([{"source_entity_id":anchor}]);
        let batch_path = output.with_extension("json");
        fs::write(&batch_path, serde_json::to_vec(&batch).unwrap()).unwrap();
        let applied = Command::new(&renderer)
            .arg("apply")
            .arg(fixtures.join("plate/parameters-100mm.json"))
            .arg("--commands")
            .arg(batch_path)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap();
        assert!(
            applied.status.success(),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
    };
    let layout = fixture.0.join("layout.lir2d");
    prepare_layout(PART, &layout, &outline.to_string());
    let validate = |layout: &Path, code| {
        fixture.call(
            &[
                "sheet-validate",
                PART,
                renderer.to_str().unwrap(),
                layout.to_str().unwrap(),
                SHEET,
                "review",
                "--font",
                font.to_str().unwrap(),
            ],
            code,
        )
    };
    let publish_checked = |layout: &Path, output: &Path, source: &Value, code| {
        fixture.call(
            &[
                "sheet-publish",
                PART,
                renderer.to_str().unwrap(),
                layout.to_str().unwrap(),
                SHEET,
                "review",
                output.to_str().unwrap(),
                "--expect-source",
                source["native_revision"].as_str().unwrap(),
                source["definition_hash"].as_str().unwrap(),
                source["drawing_sha256"].as_str().unwrap(),
                "--font",
                font.to_str().unwrap(),
            ],
            code,
        )
    };
    let validated = validate(&layout, 0);
    assert_eq!(validated["published"], false);
    assert_eq!(validated["page_mm"], json!([297.0, 210.0]));
    assert_eq!(
        validated["views"][1]["scale"],
        json!({"numerator": 1, "denominator": 2})
    );
    assert_eq!(
        validated["revision_policy"],
        "require_current_at_validation"
    );
    assert_eq!(
        validated["views"][0]["projection_dimensions"][0]["label"],
        "100 mm"
    );
    let run = |layout: &Path, policy: &str, output: &Path, code| {
        fixture.call(
            &[
                "sheet-publish",
                PART,
                renderer.to_str().unwrap(),
                layout.to_str().unwrap(),
                SHEET,
                policy,
                output.to_str().unwrap(),
                "--font",
                font.to_str().unwrap(),
            ],
            code,
        )
    };
    let first = fixture.0.join("first.svg");
    let before = run(&layout, "review", &first, 0);
    assert_eq!(before["validated_source"], validated["validated_source"]);
    assert_eq!(before["published"], true);
    assert_eq!(before["core_source_status"], "verified_current");
    assert_eq!(before["revision_policy"], "require_current_at_publish");
    assert_eq!(
        before["views"][0]["projection_dimensions"][0]["label"],
        "100 mm"
    );
    let first_bytes = fs::read(&first).unwrap();
    assert_eq!(
        run(&layout, "review", &first, 3)["code"],
        "DrawingOutputExists"
    );

    let wrapped = fixture.0.join("renderer");
    let mark = fixture.0.join("rendered");
    let resume = fixture.0.join("resume");
    fs::write(
        &wrapped,
        r#"#!/bin/sh
set -eu
test -z "${LUMA_CAD_PROJECT_KEY+x}"
"$LUMA_TEST_RENDERER" "$@"
if [ "$1" = sheet.export-svg ]; then
    touch "$LUMA_TEST_MARK"
    for step in $(seq 300); do
        test ! -e "$LUMA_TEST_RESUME" || exit 0
        sleep .05
    done
    exit 1
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapped, fs::Permissions::from_mode(0o700)).unwrap();
    let stale = fixture.0.join("stale.svg");
    let mut rendering = fixture
        .command()
        .args(["sheet-publish", PART])
        .arg(&wrapped)
        .arg(&layout)
        .args([SHEET, "review"])
        .arg(&stale)
        .arg("--font")
        .arg(&font)
        .env("LUMA_TEST_RENDERER", &renderer)
        .env("LUMA_TEST_MARK", &mark)
        .env("LUMA_TEST_RESUME", &resume)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !mark.exists() {
        if rendering.try_wait().unwrap().is_some() || Instant::now() >= deadline {
            let _ = rendering.kill();
            let output = rendering.wait_with_output().unwrap();
            panic!(
                "renderer did not stage the sheet: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    fixture.apply(&mut store, DocumentAction::UpdatePrimitive(plate(120.0)));
    fs::write(&resume, []).unwrap();
    assert_eq!(
        response(rendering.wait_with_output().unwrap(), 4)["code"],
        "Conflict"
    );
    assert!(!stale.exists());
    assert_eq!(fs::read(&first).unwrap(), first_bytes);
    drop(store);

    assert_eq!(
        publish_checked(&layout, &stale, &validated["validated_source"], 4)["code"],
        "Conflict"
    );
    assert!(!stale.exists());
    let rebound = validate(&layout, 0);
    assert_eq!(
        rebound["views"][0]["projection_dimensions"][0]["label"],
        "120 mm"
    );
    assert_ne!(
        rebound["validated_source"]["definition_hash"],
        validated["validated_source"]["definition_hash"]
    );
    assert_eq!(
        rebound["validated_source"]["drawing_sha256"],
        validated["validated_source"]["drawing_sha256"]
    );
    let updated = fixture.0.join("updated.pdf");
    let after = publish_checked(&layout, &updated, &rebound["validated_source"], 0);
    assert!(fs::read(updated).unwrap().starts_with(b"%PDF-"));
    assert_eq!(after["core_project_id"], PROJECT);
    let before_view = &before["views"][0];
    let after_view = &after["views"][0];
    assert_ne!(
        before_view["projection"]["source_revision"],
        after_view["projection"]["source_revision"]
    );
    assert_eq!(
        after_view["projection_dimensions"][0]["source_entity_id"],
        outline.to_string()
    );
    assert_eq!(after_view["projection_dimensions"][0]["label"], "120 mm");

    let changed = fixture.0.join("changed.lir2d");
    let left = living_cad_features::derived_id(FEATURE.parse().unwrap(), "drawing/v1/top/left");
    prepare_layout(PART, &changed, &left.to_string());
    assert_eq!(
        validate(&changed, 0)["views"][0]["projection_dimensions"][0]["label"],
        "60 mm"
    );
    assert_eq!(
        publish_checked(&changed, &stale, &rebound["validated_source"], 4)["code"],
        "Conflict"
    );
    assert!(!stale.exists());
    let missing = fixture.0.join("missing.lir2d");
    prepare_layout(PART, &missing, OTHER);
    let unresolved = validate(&missing, 3);
    assert_eq!(unresolved["code"], "UnresolvedDrawingReference");
    assert_eq!(
        unresolved["details"]["message_key"],
        "projection.anchor_missing"
    );
    assert_eq!(
        run(&missing, "review", &stale, 3)["code"],
        "UnresolvedDrawingReference"
    );
    assert!(!stale.exists());
    let blocked = fixture.0.join("manufacturing.svg");
    let denied = run(&layout, "manufacturing", &blocked, 3);
    assert_eq!(denied["code"], "DrawingRenderer");
    assert!(denied["details"]["details"]["reason"]
        .as_str()
        .unwrap()
        .contains("projection.quality_unproven"));
    assert!(!blocked.exists());
    let foreign = fixture.0.join("foreign.lir2d");
    prepare_layout(OTHER, &foreign, &outline.to_string());
    assert_eq!(
        run(&foreign, "review", &blocked, 3)["code"],
        "UnsupportedDrawingSource"
    );
    assert!(!blocked.exists());
}

#[cfg(all(
    feature = "luma-tasks",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
#[test]
#[ignore = "requires the qualified M7 binary at LUMA_CAD2_M7"]
fn evaluates_luma_m7_using_two_pinned_native_publications() {
    let provider = std::env::var("LUMA_CAD2_M7").expect("set LUMA_CAD2_M7");
    let fixture = Fixture::new();
    let mut store = ProjectStore::create(&fixture.model(), PROJECT, KEY).unwrap();

    let publish = |part: &str, operation: &str| {
        let pin = fixture.call(&["source", part, FEATURE], 0);
        let view = fixture.call(
            &[
                "publish",
                part,
                FEATURE,
                pin["revision"].as_str().unwrap(),
                pin["definition_hash"].as_str().unwrap(),
                operation,
            ],
            0,
        );
        assert_eq!(view["state"], "published");
        fixture.call(&["view", operation], 0)["source"]["geometry_sha256"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    let shape = |store: &mut ProjectStore,
                 part: &str,
                 operation: &str,
                 plane: &str,
                 min: [f64; 2],
                 max: [f64; 2],
                 round: bool| {
        let registry = store.registry().unwrap();
        store
            .create_document(
                &CreateDocument {
                    request_id: format!("{:032x}", registry.revision + 1),
                    document_id: part.into(),
                    expected_project_revision: registry.revision,
                    kind: DocumentKind::Part,
                    name: "M7 operand".into(),
                },
                &PreparationContext::default(),
            )
            .unwrap();
        let element = if round {
            json!({"kind":"circle","center":{"xy":min},"radius_m":max[0]})
        } else {
            json!({"kind":"rectangle","a":{"xy":min},"b":{"xy":max}})
        };
        for action in [
            json!({"op":"create","plane":plane}),
            json!({"op":"add","element":element}),
        ] {
            fixture.apply_part(store, part, DocumentAction::Sketch(action));
        }
        let source = store.part_sketch_digest(part).unwrap().unwrap();
        fixture.apply_part(
            store,
            part,
            DocumentAction::Extrude(json!({
                "op":"create_extrude","feature_id":FEATURE,
                "source_digest":source,"length_m":"0.005","direction":"one_sided"
            })),
        );
        publish(part, operation)
    };

    let bundle_file = fixture.0.join("m7-plan.json");
    let plan = |operation: &str, reversed: bool| {
        let entry = "boolean-plan.luma".to_string();
        let args = if reversed {
            "right, left"
        } else {
            "left, right"
        };
        let source = format!("use brep\nleft = brep.left_operand()\nright = brep.right_operand()\nresult = brep.{operation}(\"result\", {args})\n");
        let bundle = luma::host::SourceBundle {
            entry: entry.clone(),
            files: [(entry, source)].into(),
        };
        let wire = json!({"bundle_version":1,"entry":bundle.entry,"files":bundle.files,
            "source_digest":bundle.source_digest()});
        std::fs::write(&bundle_file, serde_json::to_vec(&wire).unwrap()).unwrap();
    };
    let run = |left: &str, digest: &str, right: &str, other: &str, code| {
        fixture.call(
            &[
                "luma-m7",
                &provider,
                left,
                digest,
                right,
                other,
                bundle_file.to_str().unwrap(),
            ],
            code,
        )
    };
    let expect_volume = |report: &Value, expected: f64| {
        let interval = &report["result"]["volume_m3"];
        assert!(
            interval[0].as_f64().unwrap() <= expected && expected <= interval[1].as_f64().unwrap(),
            "{expected} not in {interval}"
        );
        assert_eq!(report["result"]["validation"]["level"], "L3K1Solid");
        assert_eq!(report["result"]["validation"]["complete"], true);
        assert_eq!(report["result"]["native"]["format"], "brep-cbor-v1");
    };

    const RIGHT_PART: &str = "00000000-0000-4000-8000-000000000010";
    const ISLAND_PART: &str = "00000000-0000-4000-8000-000000000011";
    const ISLAND: &str = "00000000-0000-4000-8000-000000000012";
    const OBLIQUE_PART: &str = "00000000-0000-4000-8000-000000000013";
    const OBLIQUE: &str = "00000000-0000-4000-8000-000000000014";
    const TALL: &str = "00000000-0000-4000-8000-000000000015";
    const DISK_PART: &str = "00000000-0000-4000-8000-000000000016";
    const DISK: &str = "00000000-0000-4000-8000-000000000017";

    let left = shape(&mut store, PART, FIRST, "XY", [0., 0.], [0.04, 0.03], false);
    let right = shape(
        &mut store,
        RIGHT_PART,
        SECOND,
        "XY",
        [0.03, 0.01],
        [0.05, 0.04],
        false,
    );
    let original = store.registry().unwrap();
    for (operation, selector, reversed, area) in [
        ("planar_union", "union", false, 0.0016),
        ("planar_intersection", "intersection", false, 0.0002),
        ("planar_difference", "difference", false, 0.001),
        ("planar_difference", "difference", true, 0.0004),
    ] {
        plan(operation, reversed);
        let report = run(FIRST, &left, SECOND, &right, 0);
        assert_eq!(
            report["operation_id"],
            format!("living.brep.boolean.planar.{selector}")
        );
        assert_eq!(
            report["operands"][0]["operation"],
            if reversed { SECOND } else { FIRST }
        );
        assert_eq!(
            report["operands"][0]["geometry_sha256"],
            if reversed {
                right.as_str()
            } else {
                left.as_str()
            }
        );
        assert_eq!(report["request_sha256"], report["result"]["request_sha256"]);
        expect_volume(&report, area * 0.005);
        assert_eq!(report, run(FIRST, &left, SECOND, &right, 0));
    }
    assert_eq!(store.registry().unwrap(), original);

    plan("planar_union", false);
    let island = shape(
        &mut store,
        ISLAND_PART,
        ISLAND,
        "XY",
        [0.2, 0.2],
        [0.24, 0.23],
        false,
    );
    let rejected = run(FIRST, &left, ISLAND, &island, 3);
    assert_eq!(rejected["code"], "UnsupportedM7Operation");
    assert_eq!(
        rejected["details"]["provider_code"],
        "UnsupportedResultTopology"
    );
    assert!(rejected["details"]["native"].is_null());

    let oblique = shape(
        &mut store,
        OBLIQUE_PART,
        OBLIQUE,
        "XZ",
        [0.03, 0.01],
        [0.05, 0.04],
        false,
    );
    assert_eq!(
        run(FIRST, &left, OBLIQUE, &oblique, 3)["code"],
        "UnsupportedM7Operands"
    );
    fixture.apply_part(
        &mut store,
        RIGHT_PART,
        DocumentAction::Extrude(json!({
            "op":"update_extrude_length","feature_id":FEATURE,"length_m":"0.01"
        })),
    );
    let tall = publish(RIGHT_PART, TALL);
    assert_eq!(
        run(FIRST, &left, TALL, &tall, 3)["code"],
        "UnsupportedM7Operands"
    );
    assert_eq!(
        run(FIRST, &tall, SECOND, &right, 3)["code"],
        "StaleTopologyRef"
    );

    plan("planar_intersection", true);
    let disk = shape(
        &mut store,
        DISK_PART,
        DISK,
        "XY",
        [0.02, 0.015],
        [0.005, 0.],
        true,
    );
    let mixed = run(FIRST, &left, DISK, &disk, 0);
    expect_volume(&mixed, PI * 0.005 * 0.005 * 0.005);
}
