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
        let key: String = KEY.into_iter().map(|byte| format!("{byte:02x}")).collect();
        let process = Command::new(env!("CARGO_BIN_EXE_luma-cad-brep"))
            .arg(self.model())
            .arg(PROJECT)
            .arg(self.output())
            .args(args)
            .env("LUMA_CAD_PROJECT_KEY", key)
            .output()
            .unwrap();
        assert_eq!(
            process.status.code(),
            Some(code),
            "{args:?}\n{}\n{}",
            String::from_utf8_lossy(&process.stdout),
            String::from_utf8_lossy(&process.stderr)
        );
        let response: Value = serde_json::from_slice(&process.stdout).unwrap();
        assert_eq!(response["protocol"], "luma.cad.brep-project/1");
        response[if code == 0 { "result" } else { "error" }].clone()
    }

    fn apply(&self, store: &mut ProjectStore, action: DocumentAction) {
        let registry = store.registry().unwrap();
        store
            .apply_document(
                &DocumentCommand {
                    request_id: format!("{:032x}", registry.revision + 1),
                    document_id: PART.into(),
                    expected_project_revision: registry.revision,
                    expected_document_revision: registry.documents[PART].revision,
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
