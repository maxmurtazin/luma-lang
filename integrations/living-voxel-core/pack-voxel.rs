//! Pack a sampled surface as an immutable View ExternalAsset.
use living_view_contract::{AffineTransform64, SceneStamp, SourceBinding};
use living_view_core_adapter::{MeshPacket, SceneBuilder, SceneBundle};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::File,
    io::{self, Read},
    path::Path,
};

const LIMIT: usize = 24 * 1024 * 1024;

#[derive(Deserialize)]
struct Wire {
    voxel_revision: String,
    surface: Surface,
}

#[derive(Deserialize)]
struct Surface {
    schema: String,
    units: String,
    source_digest: String,
    source_manifest_digest: Option<String>,
    vertices: Vec<Vertex>,
    triangles: Vec<Triangle>,
    report: Report,
}

#[derive(Deserialize)]
struct Report {
    position_error_bound: String,
    hausdorff_bound: String,
    geometry_scope: Option<String>,
}

#[derive(Deserialize)]
struct Vertex {
    position_m: [f64; 3],
}

#[derive(Deserialize)]
struct Triangle {
    vertices: [u32; 3],
}

#[derive(Deserialize)]
struct AdaptiveEnvelope {
    result: AdaptiveWire,
}

#[derive(Deserialize)]
struct AdaptiveWire {
    profile: String,
    voxel_revision: String,
    part_source: Value,
    provider_binary_sha256: String,
    digests: AdaptiveDigests,
    tree_digest: String,
    surface: AdaptiveSurface,
}

#[derive(Deserialize)]
struct AdaptiveDigests {
    blob_digest: String,
    manifest_digest: String,
}

#[derive(Deserialize)]
struct AdaptiveSurface {
    schema: String,
    profile: String,
    source_digest: String,
    source_tree_digest: String,
    vertices: Vec<Vertex>,
    triangles: Vec<Triangle>,
    report: Value,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::new();
    io::stdin().take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err("Voxel surface exceeds the View input limit".into());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mask = match args.as_slice() {
        [mode, tree] if mode == "adaptive" => return pack_adaptive(&bytes, Path::new(tree)),
        [mode, tree] if mode == "mask" => Some(Path::new(tree)),
        [] => None,
        _ => return Err("usage: pack-voxel [adaptive|mask INPUT.lvxadap] < report.json".into()),
    };
    let wire: Wire = serde_json::from_slice(&bytes)?;
    let surface = wire.surface;
    if wire.voxel_revision != include_str!("voxel-pin.txt").trim()
        || surface.units != "m"
        || !is_digest(&surface.source_digest)
        || surface.vertices.is_empty()
        || surface.vertices.len() > 100_000
        || surface.triangles.is_empty()
        || surface.triangles.len() > 200_000
        || surface.report.position_error_bound != "Unavailable"
        || surface.report.hausdorff_bound != "Unavailable"
    {
        return Err("Unsupported Voxel surface profile".into());
    }
    let asset_provider = match (mask, surface.schema.as_str()) {
        (None, "living.voxel.mesh/v1") => format!("living-voxel-core@{}", wire.voxel_revision),
        (Some(tree), "luma.cad.adaptive-mask-surface/1")
            if surface.report.geometry_scope.as_deref() == Some("SampledCellUnion")
                && surface
                    .source_manifest_digest
                    .as_deref()
                    .is_some_and(is_digest) =>
        {
            check_tree(tree, &surface.source_digest)?;
            format!("luma.cad.mask-surface/1@{}", wire.voxel_revision)
        }
        _ => return Err("Unsupported Voxel surface profile".into()),
    };
    let content_digest = format!("{:x}", Sha256::digest(&bytes));
    let bundle = scene(
        surface.vertices,
        surface.triangles,
        &asset_provider,
        &surface.source_digest,
        &content_digest,
    )?;
    serde_json::to_writer(io::stdout().lock(), &bundle)?;
    Ok(())
}

fn pack_adaptive(bytes: &[u8], tree: &Path) -> Result<(), Box<dyn Error>> {
    let wire: AdaptiveEnvelope = serde_json::from_slice(bytes)?;
    let report = wire.result;
    let surface = report.surface;
    let hausdorff = surface.report["hausdorff_error_upper_m"].as_f64();
    if report.voxel_revision != include_str!("voxel-pin.txt").trim()
        || report.profile != "cad3-adaptive-sphere-v1"
        || surface.schema != "living.voxel.mesh/v1"
        || surface.profile != "AdaptiveSphereDualContouring"
        || surface.source_tree_digest != report.tree_digest
        || !is_digest(&report.tree_digest)
        || !is_digest(&surface.source_digest)
        || !is_digest(&report.digests.manifest_digest)
        || !is_digest(&report.provider_binary_sha256)
        || !report.part_source["definition_hash"]
            .as_str()
            .is_some_and(is_digest)
        || report.part_source["revision"].as_u64().is_none()
        || surface.vertices.is_empty()
        || surface.vertices.len() > 8192
        || surface.triangles.is_empty()
        || surface.triangles.len() > 16384
        || surface.report["boundary_edges"] != 0
        || surface.report["connected_components"] != 1
        || surface.report["euler_characteristic"] != 2
        || surface.report["topology_checks"] != "RadiallyEmbeddedClosedGenusZero"
        || !hausdorff.is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        return Err("Unsupported adaptive Voxel report".into());
    }
    check_tree(tree, &report.digests.blob_digest)?;
    let content_digest = format!("{:x}", Sha256::digest(bytes));
    let bundle = scene(
        surface.vertices,
        surface.triangles,
        &format!("living-voxel-core@{}", report.voxel_revision),
        &report.digests.blob_digest,
        &content_digest,
    )?;
    serde_json::to_writer(
        io::stdout().lock(),
        &json!({
            "bundle":bundle,
            "evidence":{
                "schema":"luma.cad.adaptive-view/1",
                "part_source":report.part_source,
                "voxel_revision":report.voxel_revision,
                "provider_binary_sha256":report.provider_binary_sha256,
                "provider_report_sha256":content_digest,
                "lvxadap_sha256":report.digests.blob_digest,
                "manifest_digest":report.digests.manifest_digest,
                "tree_digest":report.tree_digest,
                "analytic_source_digest":surface.source_digest,
                "provider_surface_report":surface.report,
                "view_mesh_accuracy":"Unavailable"
            }
        }),
    )?;
    Ok(())
}

fn check_tree(tree: &Path, digest: &str) -> Result<(), Box<dyn Error>> {
    const MAX_TREE: u64 = 8 * 1024 * 1024;
    let mut tree_bytes = Vec::new();
    File::open(tree)?
        .take(MAX_TREE + 1)
        .read_to_end(&mut tree_bytes)?;
    if tree_bytes.len() as u64 > MAX_TREE
        || !tree_bytes.starts_with(b"LVXADAP\0")
        || digest != format!("{:x}", Sha256::digest(&tree_bytes))
    {
        return Err("LVXADAP digest differs from the adaptive report".into());
    }
    Ok(())
}

fn scene(
    vertices: Vec<Vertex>,
    triangles: Vec<Triangle>,
    asset_provider: &str,
    asset_id: &str,
    content_digest: &str,
) -> Result<SceneBundle, Box<dyn Error>> {
    let positions: Vec<_> = vertices
        .into_iter()
        .map(|vertex| vertex.position_m)
        .collect();
    let triangles: Vec<_> = triangles
        .into_iter()
        .map(|triangle| triangle.vertices)
        .collect();
    let mut builder = SceneBuilder::new(SceneStamp {
        scene_id: format!("voxel-surface:{content_digest}"),
        revision: 1,
    });
    let geometry = builder.mesh(MeshPacket::from_si(&positions, &triangles, 0, || false)?)?;
    builder.occurrence(
        vec![format!("voxel-{asset_id}")],
        AffineTransform64::IDENTITY,
        geometry,
        Some(SourceBinding::ExternalAsset {
            provider: asset_provider.into(),
            asset_id: asset_id.into(),
            content_digest: content_digest.into(),
        }),
    )?;
    Ok(builder.finish())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| matches!(ch, b'0'..=b'9' | b'a'..=b'f'))
}
