//! Pack a sampled surface as an immutable View ExternalAsset.
use living_view_contract::{AffineTransform64, SceneStamp, SourceBinding};
use living_view_core_adapter::{MeshPacket, SceneBuilder};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    io::{self, Read},
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
    vertices: Vec<Vertex>,
    triangles: Vec<Triangle>,
    report: Report,
}

#[derive(Deserialize)]
struct Report {
    position_error_bound: String,
    hausdorff_bound: String,
}

#[derive(Deserialize)]
struct Vertex {
    position_m: [f64; 3],
}

#[derive(Deserialize)]
struct Triangle {
    vertices: [u32; 3],
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::new();
    io::stdin().take(LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err("Voxel surface exceeds the View input limit".into());
    }
    let wire: Wire = serde_json::from_slice(&bytes)?;
    let surface = wire.surface;
    if wire.voxel_revision != include_str!("voxel-pin.txt").trim()
        || surface.schema != "living.voxel.mesh/v1"
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
    let positions: Vec<_> = surface
        .vertices
        .into_iter()
        .map(|vertex| vertex.position_m)
        .collect();
    let triangles: Vec<_> = surface
        .triangles
        .into_iter()
        .map(|triangle| triangle.vertices)
        .collect();
    let content_digest = format!("{:x}", Sha256::digest(&bytes));
    let mut builder = SceneBuilder::new(SceneStamp {
        scene_id: format!("voxel-surface:{content_digest}"),
        revision: 1,
    });
    let geometry = builder.mesh(MeshPacket::from_si(&positions, &triangles, 0, || false)?)?;
    builder.occurrence(
        vec![format!("voxel-{}", surface.source_digest)],
        AffineTransform64::IDENTITY,
        geometry,
        Some(SourceBinding::ExternalAsset {
            provider: format!("living-voxel-core@{}", wire.voxel_revision),
            asset_id: surface.source_digest,
            content_digest,
        }),
    )?;
    serde_json::to_writer(io::stdout().lock(), &builder.finish())?;
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| matches!(ch, b'0'..=b'9' | b'a'..=b'f'))
}
