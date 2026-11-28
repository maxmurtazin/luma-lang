//! Publish a checked adaptive sphere container from a historical Core revision.
use super::{core_error, encode, io_error, number, provider, publish, read, usage, Result};
use living_cad_viewer::checkpoint::project::luma_cad::voxel_jobs::SphereSource;
use living_cad_viewer::checkpoint::project::ProjectStore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const MAX_PROVIDER: u64 = 128 * 1024 * 1024;
const MAX_TREE_BYTES: u64 = 8 * 1024 * 1024;
const SURFACE: &str = "living.voxel.surface.extract.sphere.adaptive";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    grid: Value,
    refinement: Refinement,
    max_hausdorff_error_m: f64,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Refinement {
    surface_level: u8,
    max_level: u8,
    #[serde(default)]
    regions: Vec<Value>,
}

pub(super) fn publish_sphere(
    executable: &Path,
    source: &SphereSource,
    profile: &Path,
    output: &Path,
) -> Result<Value> {
    let settings = parse_profile(load_profile(profile)?)?;
    let worker = Worker::new(executable)?;
    let (bytes, report) = worker.compute(source, settings, &|| true)?;
    publish(output, &bytes)?;
    Ok(report)
}

pub(super) fn execute(store: &ProjectStore, action: &str, args: &[String]) -> Result<Value> {
    match (action, args) {
        ("adaptive-submit", [provider, request, part, revision, hash, profile]) => {
            let settings = load_profile(Path::new(profile))?;
            parse_profile(settings.clone())?;
            let (_, digest) = capture_program(Path::new(provider))?;
            encode(
                store
                    .admit_adaptive_voxel_job(
                        request,
                        &fingerprint(&digest),
                        part,
                        (revision.parse().map_err(|_| usage())?, hash),
                        settings,
                    )
                    .map_err(core_error)?,
            )
        }
        ("adaptive-work", [provider]) => {
            let worker = Worker::new(Path::new(provider))?;
            encode(
                store
                    .run_adaptive_voxel_job(&fingerprint(&worker.digest), |input, current| {
                        let profile = parse_profile(input.profile.clone())
                            .map_err(|_| "InvalidAdaptiveProfile")?;
                        let (bytes, report) = worker
                            .compute(&input.source, profile, current)
                            .map_err(|fault| match fault["code"].as_str() {
                                Some("LimitExceeded") => "LimitExceeded",
                                Some("UnsupportedCapability") => "UnsupportedCapability",
                                Some("Cancelled") => "Cancelled",
                                _ => "AdaptiveProviderRejected",
                            })?;
                        Ok(json!({"report":report,"lvx_hex":hex::encode(bytes)}))
                    })
                    .map_err(core_error)?,
            )
        }
        ("adaptive-receipt", [request]) => {
            encode(store.adaptive_voxel_receipt(request).map_err(core_error)?)
        }
        ("adaptive-status", [job]) => encode(
            store
                .adaptive_voxel_status(number(job)?)
                .map_err(core_error)?,
        ),
        ("adaptive-cancel", [job]) => {
            let job = number(job)?;
            store.adaptive_voxel_status(job).map_err(core_error)?;
            encode(store.cancel_luma_job(job).map_err(core_error)?)
        }
        ("adaptive-retry", [job, attempt]) => {
            let job = number(job)?;
            store.adaptive_voxel_status(job).map_err(core_error)?;
            encode(
                store
                    .retry_luma_job(job, number(attempt)?)
                    .map_err(core_error)?,
            )
        }
        ("adaptive-export", [job, output]) => {
            let result = store
                .adaptive_voxel_result(number(job)?)
                .map_err(core_error)?;
            let report = &result["result"]["report"];
            let bytes = hex::decode(
                result["result"]["lvx_hex"]
                    .as_str()
                    .ok_or_else(invalid_artifact)?,
            )
            .map_err(|_| invalid_artifact())?;
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if !bytes.starts_with(b"LVXADAP\0") || report["digests"]["blob_digest"] != digest {
                return Err(invalid_artifact());
            }
            let mut report = report.clone();
            report["job_manifest"] = result["manifest"].clone();
            publish(Path::new(output), &bytes)?;
            Ok(report)
        }
        _ => Err(usage()),
    }
}

fn load_profile(profile: &Path) -> Result<Value> {
    serde_json::from_slice(&read(profile, 16 * 1024)?).map_err(invalid_profile)
}

fn parse_profile(settings: Value) -> Result<Profile> {
    let profile: Profile = serde_json::from_value(settings).map_err(invalid_profile)?;
    if profile.grid["layout"] != "CellCenter"
        || profile.refinement.max_level > 5
        || profile.refinement.regions.len() > 8
        || !profile.max_hausdorff_error_m.is_finite()
        || profile.max_hausdorff_error_m < 0.0
    {
        return Err(json!({"code":"UnsupportedAdaptiveProfile",
            "message":"Expected a CellCenter grid, depth at most 5, at most 8 regions, and a finite nonnegative tolerance"}));
    }
    Ok(profile)
}

fn invalid_profile(error: serde_json::Error) -> Value {
    json!({"code":"InvalidRequest","message":error.to_string()})
}

fn capture_program(executable: &Path) -> Result<(Vec<u8>, String)> {
    let program = read(executable, MAX_PROVIDER)?;
    let digest = format!("{:x}", Sha256::digest(&program));
    Ok((program, digest))
}

fn fingerprint(digest: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"luma.voxel.adaptive.provider/1\0");
    hash.update(digest.as_bytes());
    format!("sha256:{:x}", hash.finalize())
}

struct Worker {
    directory: tempfile::TempDir,
    executable: std::path::PathBuf,
    digest: String,
}

impl Worker {
    fn new(executable: &Path) -> Result<Self> {
        let directory = tempfile::tempdir().map_err(io_error)?;
        let captured = directory
            .path()
            .join(if cfg!(windows) { "voxel.exe" } else { "voxel" });
        let (program, digest) = capture_program(executable)?;
        fs::write(&captured, program).map_err(io_error)?;
        fs::set_permissions(
            &captured,
            fs::metadata(executable).map_err(io_error)?.permissions(),
        )
        .map_err(io_error)?;
        Ok(Self {
            directory,
            executable: captured,
            digest,
        })
    }

    fn compute(
        &self,
        source: &SphereSource,
        profile: Profile,
        current: &dyn Fn() -> bool,
    ) -> Result<(Vec<u8>, Value)> {
        let directory = &self.directory;
        let captured = &self.executable;
        let plan = directory.path().join("plan.json");
        let container = directory.path().join("sphere.lvxadap");
        let sphere = json!({"center_m":source.center_m,"radius_m":source.radius_m});
        fs::write(&plan, json!({
        "source":sphere,"grid":profile.grid,"refinement":profile.refinement,
        "numeric_profile":{"id":"grid-si-v1","version":1},
        "budget":{"max_tiles":16000,"max_leaves":4096,"max_hermite_samples":4096,"max_balance_checks":400000}
    }).to_string()).map_err(io_error)?;
        let digests = provider::run_while(
            captured,
            &[
                "encode-adaptive-sphere".as_ref(),
                plan.as_os_str(),
                container.as_os_str(),
            ],
            current,
        )?;
        let tree_bytes = read(&container, MAX_TREE_BYTES)?;
        let blob_digest = format!("{:x}", Sha256::digest(&tree_bytes));
        if !tree_bytes.starts_with(b"LVXADAP\0") || digests["blob_digest"] != blob_digest {
            return Err(invalid_artifact());
        }
        let decoded = provider::run_while(
            captured,
            &[
                "decode-adaptive-sphere".as_ref(),
                container.as_os_str(),
                blob_digest.as_ref(),
            ],
            current,
        )?;
        let artifact = &decoded["artifact"];
        let tree_digest = artifact["tree_digest"]
            .as_str()
            .ok_or_else(invalid_artifact)?;
        if artifact["schema"] != "living.voxel.adaptive.sphere/v1"
            || artifact["balance_policy"] != "StrongTwoToOne"
            || artifact["source"] != sphere
            || decoded["digests"] != digests
        {
            return Err(invalid_artifact());
        }
        let mesh = provider::operate_while(
            captured,
            SURFACE,
            &json!({
                "encoded_tree":container,"expected_blob_sha256":blob_digest,"expected_tree_sha256":tree_digest,
                "max_hausdorff_error_m":profile.max_hausdorff_error_m,
                "budget":{"max_primal_crossings":8192,"max_vertices":8192,"max_triangles":16384}
            }),
            current,
        )?;
        let bound = mesh["report"]["hausdorff_error_upper_m"].as_f64();
        if mesh["profile"] != "AdaptiveSphereDualContouring"
            || mesh["source_tree_digest"] != tree_digest
            || mesh["source_digest"] != artifact["source_digest"]
            || mesh["report"]["boundary_edges"] != 0
            || mesh["report"]["connected_components"] != 1
            || mesh["report"]["euler_characteristic"] != 2
            || mesh["report"]["topology_checks"] != "RadiallyEmbeddedClosedGenusZero"
            || !bound.is_some_and(|value| (0.0..=profile.max_hausdorff_error_m).contains(&value))
        {
            return Err(invalid_artifact());
        }
        let report = json!({
            "profile":"cad3-adaptive-sphere-v1","part_source":encode(source)?,
            "provider_binary_sha256":self.digest,"voxel_revision":include_str!("voxel-pin.txt").trim(),
            "digests":digests,"tree_digest":tree_digest,"tree_report":artifact["report"],"surface":mesh
        });
        Ok((tree_bytes, report))
    }
}

fn invalid_artifact() -> Value {
    json!({"code":"CorruptArtifact","message":"Adaptive tree or surface failed provenance or geometry checks"})
}
