//! Publish a checked adaptive sphere container from a historical Core revision.
use super::{encode, io_error, provider, publish, read, Result};
use living_cad_viewer::checkpoint::project::luma_cad::voxel_jobs::SphereSource;
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
    let profile: Profile = serde_json::from_slice(&read(profile, 16 * 1024)?)
        .map_err(|error| json!({"code":"InvalidRequest","message":error.to_string()}))?;
    if profile.grid["layout"] != "CellCenter"
        || profile.refinement.max_level > 5
        || profile.refinement.regions.len() > 8
        || !profile.max_hausdorff_error_m.is_finite()
        || profile.max_hausdorff_error_m < 0.0
    {
        return Err(json!({"code":"UnsupportedAdaptiveProfile",
            "message":"Expected a CellCenter grid, depth at most 5, at most 8 regions, and a finite nonnegative tolerance"}));
    }
    let directory = tempfile::tempdir().map_err(io_error)?;
    // Every provider step uses the same captured executable.
    let captured = directory
        .path()
        .join(if cfg!(windows) { "voxel.exe" } else { "voxel" });
    let program = read(executable, MAX_PROVIDER)?;
    let provider_sha256 = format!("{:x}", Sha256::digest(&program));
    fs::write(&captured, program).map_err(io_error)?;
    fs::set_permissions(
        &captured,
        fs::metadata(executable).map_err(io_error)?.permissions(),
    )
    .map_err(io_error)?;

    let plan = directory.path().join("plan.json");
    let container = directory.path().join("sphere.lvxadap");
    let sphere = json!({"center_m":source.center_m,"radius_m":source.radius_m});
    fs::write(&plan, json!({
        "source":sphere,"grid":profile.grid,"refinement":profile.refinement,
        "numeric_profile":{"id":"grid-si-v1","version":1},
        "budget":{"max_tiles":16000,"max_leaves":4096,"max_hermite_samples":4096,"max_balance_checks":400000}
    }).to_string()).map_err(io_error)?;
    let digests = provider::run(
        &captured,
        &[
            "encode-adaptive-sphere".as_ref(),
            plan.as_os_str(),
            container.as_os_str(),
        ],
    )?;
    let tree_bytes = read(&container, MAX_TREE_BYTES)?;
    let blob_digest = format!("{:x}", Sha256::digest(&tree_bytes));
    if !tree_bytes.starts_with(b"LVXADAP\0") || digests["blob_digest"] != blob_digest {
        return Err(invalid_artifact());
    }
    let decoded = provider::run(
        &captured,
        &[
            "decode-adaptive-sphere".as_ref(),
            container.as_os_str(),
            blob_digest.as_ref(),
        ],
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
    let mesh = provider::operate(
        &captured,
        SURFACE,
        &json!({
            "encoded_tree":container,"expected_blob_sha256":blob_digest,"expected_tree_sha256":tree_digest,
            "max_hausdorff_error_m":profile.max_hausdorff_error_m,
            "budget":{"max_primal_crossings":8192,"max_vertices":8192,"max_triangles":16384}
        }),
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
        "provider_binary_sha256":provider_sha256,
        "digests":digests,"tree_digest":tree_digest,"tree_report":artifact["report"],"surface":mesh
    });
    publish(output, &tree_bytes)?;
    Ok(report)
}

fn invalid_artifact() -> Value {
    json!({"code":"CorruptArtifact","message":"Adaptive tree or surface failed provenance or geometry checks"})
}
