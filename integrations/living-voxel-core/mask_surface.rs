//! Tessellate the boundary of a verified adaptive cell-union mask.
use super::{invalid, read, PINS};
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::{collections::BTreeSet, path::Path, sync::atomic::AtomicBool};
use voxel_core::{
    field::{decode_adaptive, AdaptiveArtifact, AdaptiveIoLimits, Sample, Value},
    native::Digest,
    registry::Registry,
    types::{ChannelSemantic, DType, MetricStatus},
    Diagnostic, DiagnosticCode,
};

#[derive(Serialize)]
struct Vertex {
    position_m: [f64; 3],
}

#[derive(Serialize)]
struct Triangle {
    vertices: [u32; 3],
}

pub(super) fn surface(path: &Path, digest: Digest) -> Result<Json, Diagnostic> {
    let limits = AdaptiveIoLimits::default();
    let bytes = read(path, 2 * 1024 * 1024)?;
    let decoded = decode_adaptive(
        Registry::builtin()?,
        &bytes,
        Some(digest),
        limits,
        &AtomicBool::new(false),
    )?;
    let tree = decoded.artifact;
    let min = tree.index_region.index_min.map(i64::from);
    let max = tree.index_region.index_max.map(i64::from);
    let side = max[0] - min[0];
    if tree.dtype != DType::Bool
        || tree.semantic_kind != ChannelSemantic::CellUnionMask
        || tree.source_channel != "occupancy"
        || !matches!(side, 2 | 4 | 8 | 16)
        || [max[1] - min[1], max[2] - min[2]] != [side; 2]
    {
        return Err(unsupported("Expected a bounded Gyroid occupancy cube"));
    }

    let mut occupied = BTreeSet::new();
    for leaf in &tree.leaves {
        let Sample::Known {
            value: Value::Bool(inside),
            metric_status: MetricStatus::NotDistance,
        } = leaf.sample
        else {
            return Err(unsupported("Mask surface requires known Bool samples"));
        };
        if !inside {
            continue;
        }
        let anchor = leaf.anchor.map(i64::from);
        let width = i64::from(leaf.finest_cells_per_axis);
        if (0..3).any(|axis| anchor[axis] == min[axis] || anchor[axis] + width == max[axis]) {
            return Err(unsupported("Mask surface requires an empty one-cell halo"));
        }
        for x in anchor[0]..anchor[0] + width {
            for y in anchor[1]..anchor[1] + width {
                for z in anchor[2]..anchor[2] + width {
                    occupied.insert([x, y, z]);
                }
            }
        }
    }
    if occupied.is_empty() {
        return Err(unsupported("Mask surface requires occupied cells"));
    }

    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for &cell in &occupied {
        for axis in 0..3 {
            for direction in [-1, 1] {
                let mut neighbor = cell;
                neighbor[axis] += direction;
                if !occupied.contains(&neighbor) {
                    append_face(&tree, cell, axis, direction, &mut vertices, &mut triangles);
                }
            }
        }
    }
    let pins: Json = serde_json::from_str(PINS).map_err(invalid)?;
    Ok(json!({
        "voxel_revision":pins["voxel"],
        "surface":{
            "schema":"luma.cad.adaptive-mask-surface/1","profile":"CellUnionBoundary","units":"m",
            "source_digest":digest,"mask_source_digest":tree.source_digest,
            "source_manifest_digest":decoded.digests.manifest_digest,
            "vertices":vertices,"triangles":triangles,
            "report":{
                "geometry_scope":"SampledCellUnion",
                "occupied_cells":occupied.len(),"exposed_faces":triangles.len()/2,
                "topology_checks":"NotCertified",
                "position_error_bound":"Unavailable","hausdorff_bound":"Unavailable"
            }
        }
    }))
}

fn append_face(
    tree: &AdaptiveArtifact,
    cell: [i64; 3],
    axis: usize,
    direction: i64,
    vertices: &mut Vec<Vertex>,
    triangles: &mut Vec<Triangle>,
) {
    let base = vertices.len() as u32;
    for (i, j) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
        let mut corner = cell;
        corner[axis] += i64::from(direction > 0);
        corner[(axis + 1) % 3] += i;
        corner[(axis + 2) % 3] += j;
        let local = corner.map(|index| index as f64 * tree.finest_spacing_m);
        let position_m = std::array::from_fn(|axis| {
            tree.axes[axis]
                .iter()
                .zip(local)
                .fold(tree.origin_m[axis], |value, (basis, position)| {
                    value + basis * position
                })
        });
        vertices.push(Vertex { position_m });
    }
    let order = if direction > 0 { [1, 3] } else { [3, 1] };
    triangles.push(Triangle {
        vertices: [base, base + order[0], base + 2],
    });
    triangles.push(Triangle {
        vertices: [base, base + 2, base + order[1]],
    });
}

fn unsupported(message: &str) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::UnsupportedCapability, message)
}
