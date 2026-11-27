//! Native CAD-3 bootstrap: sample a Living IR Core F-Rep sphere with Living Voxel.
use living_frep::{FieldSemantics, GeometryPrimitive, Point3, Pose3};
use living_symbolic::{Dimension, Quantity};
use serde::Deserialize;
use serde_json::{json, Value as Json};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::ExitCode,
    sync::atomic::AtomicBool,
};
use voxel_core::{
    field::{
        extract_uniform_surface, slice_sample, FieldOperand, GridAxis, ResidentField, Sample,
        SliceOptions, SlicePlacement, SlicePolicy, SurfaceBudget, SurfaceOptions,
        UniformSurfaceProfile, Value,
    },
    native::{
        decode_field, encode_field, hash, hash_reader, BackendFingerprint, Digest, EncodedField,
        Limits,
    },
    registry::Registry,
    sampling::{sample_field, FieldDescriptor, FieldEvaluator, SamplingRequest},
    types::{ChannelSemantic, DType, GridSpec, Layout, MetricStatus, VersionedId},
    Diagnostic, DiagnosticCode,
};

const CHANNEL: &str = "distance";
const PINS: &str = include_str!("provider-pins.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u32,
    radius_m: f64,
    center_m: [f64; 3],
    grid: GridSpec,
    #[serde(default)]
    part_source: Option<Json>,
}

struct Sphere {
    primitive: GeometryPrimitive,
    descriptor: FieldDescriptor,
}

impl FieldEvaluator for Sphere {
    fn descriptor(&self) -> FieldDescriptor {
        self.descriptor.clone()
    }

    fn evaluate_batch(&self, points: &[[f64; 3]]) -> Result<Vec<Sample>, Diagnostic> {
        points
            .iter()
            .map(|point| {
                let value = self
                    .primitive
                    .sample_at(Point3::new(*point).map_err(geometry)?)
                    .map_err(geometry)?;
                if value.semantics != FieldSemantics::KnownSdf {
                    return Err(Diagnostic::new(
                        DiagnosticCode::UnsupportedCapability,
                        "Expected a Core sphere SDF",
                    ));
                }
                Ok(Sample::Known {
                    value: Value::F64(value.value_m),
                    metric_status: MetricStatus::Estimate,
                })
            })
            .collect()
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (result, code) = match execute(&args) {
        Ok(value) => (json!({"result":value}), 0),
        Err(error) => (json!({"error":error}), 2),
    };
    println!("{result}");
    ExitCode::from(code)
}

fn execute(args: &[String]) -> Result<Json, Diagnostic> {
    match args {
        [action, request, output] if action == "sample" => {
            sample(Path::new(request), Path::new(output), false)
        }
        [action, request, output] if action == "sample-pinned" => {
            sample(Path::new(request), Path::new(output), true)
        }
        [action, path, expected, i, j, k] if action == "inspect" => {
            let mut index = [0; 3];
            for (slot, value) in index.iter_mut().zip([i, j, k]) {
                *slot = value.parse().map_err(invalid)?;
            }
            let registry = Registry::builtin()?;
            let limits = Limits::default();
            let bytes = read(Path::new(path), limits.max_file_bytes)?;
            let decoded = decode_field(
                registry,
                &bytes,
                Some(expected.parse().map_err(invalid)?),
                limits,
            )?;
            let field = ResidentField::accept(decoded.field, registry)?;
            Ok(json!({
                "digests":decoded.digests,"source":decoded.source_descriptor,
                "backend":decoded.backend_fingerprint,"observation":field.lookup(CHANNEL, index)?
            }))
        }
        [action, path, expected] if action == "surface" => {
            let registry = Registry::builtin()?;
            let digest: Digest = expected.parse().map_err(invalid)?;
            let bytes = read(Path::new(path), Limits::default().max_file_bytes)?;
            let decoded = decode_field(registry, &bytes, Some(digest), Limits::default())?;
            let field = ResidentField::accept(decoded.field, registry)?;
            let surface = extract_uniform_surface(
                FieldOperand {
                    field: &field,
                    channel: CHANNEL,
                },
                SurfaceOptions {
                    source_digest: digest,
                    level: 0.0,
                    profile: UniformSurfaceProfile::MonotoneTrilinear,
                    region: None,
                    budget: SurfaceBudget {
                        max_samples: 262_144,
                        max_cells: 262_144,
                        max_vertices: 100_000,
                        max_triangles: 200_000,
                    },
                    cancelled: &AtomicBool::new(false),
                },
            )?;
            Ok(
                json!({"surface":surface, "source":decoded.source_descriptor,
                "voxel_revision":serde_json::from_str::<Json>(PINS).map_err(invalid)?["voxel"]}),
            )
        }
        [action, path, expected, z] if action == "slice-xy" => {
            let registry = Registry::builtin()?;
            let digest = expected.parse().map_err(invalid)?;
            let bytes = read(Path::new(path), Limits::default().max_file_bytes)?;
            let decoded = decode_field(registry, &bytes, Some(digest), Limits::default())?;
            let field = ResidentField::accept(decoded.field, registry)?;
            let grid = field.grid();
            let sliced = slice_sample(
                FieldOperand {
                    field: &field,
                    channel: CHANNEL,
                },
                SliceOptions {
                    placement: SlicePlacement::Axis {
                        axis: GridAxis::Z,
                        index_coordinate: z.parse().map_err(invalid)?,
                        index_min: [grid.index_min[0], grid.index_min[1]],
                        index_max: [grid.index_max[0], grid.index_max[1]],
                    },
                    source_digest: digest,
                    level: 0.0,
                    unknown: SlicePolicy::Preserve,
                    outside: SlicePolicy::Preserve,
                    max_samples: 65_536,
                    cancelled: &AtomicBool::new(false),
                },
            )?;
            Ok(
                json!({"slice":sliced.slice,"known":sliced.known,"unknown":sliced.unknown,
                "outside":sliced.outside,"source":decoded.source_descriptor}),
            )
        }
        _ => Err(invalid(
            "luma-cad-voxel sample REQUEST.json OUTPUT.lvx | inspect INPUT.lvx SHA256 I J K",
        )),
    }
}

fn sample(request_path: &Path, output: &Path, pinned: bool) -> Result<Json, Diagnostic> {
    let bytes = read(request_path, 16 * 1024)?;
    let request: Request = serde_json::from_slice(&bytes).map_err(invalid)?;
    if request.schema_version != 1 {
        return Err(Diagnostic::new(
            DiagnosticCode::UnknownSchema,
            "Expected sphere request version 1",
        ));
    }
    if request.part_source.is_some() != pinned {
        return Err(invalid("Expected Part provenance only for sample-pinned"));
    }
    let (artifact, report) = sample_sphere(
        request.center_m,
        request.radius_m,
        request.grid,
        request.part_source.as_ref(),
    )?;
    publish(output, &artifact.bytes)?;
    Ok(report)
}

fn sample_sphere(
    center_m: [f64; 3],
    radius_m: f64,
    grid: GridSpec,
    part_source: Option<&Json>,
) -> Result<(EncodedField, Json), Diagnostic> {
    if part_source.is_some() && grid.layout != Layout::Node {
        return Err(Diagnostic::new(
            DiagnosticCode::UnsupportedCapability,
            "The project profile requires a Node grid",
        ));
    }
    let centre = Point3::new(center_m).map_err(geometry)?;
    let pose = Pose3::new(centre, [0.0, 0.0, 0.0, 1.0]).map_err(geometry)?;
    let radius = Quantity::new(radius_m, Dimension::LENGTH).map_err(invalid)?;
    let primitive = GeometryPrimitive::sphere(radius, pose).map_err(geometry)?;
    let pins: Json = serde_json::from_str(PINS).map_err(invalid)?;
    let core = pins["core"]
        .as_str()
        .ok_or_else(|| invalid("Missing Core pin"))?;
    let mut identity = b"luma.cad.core-sphere/1\0".to_vec();
    identity.extend_from_slice(core.as_bytes());
    if let Some(source) = part_source {
        identity.extend_from_slice(&serde_json::to_vec(source).map_err(invalid)?);
    }
    for coordinate in centre.xyz().into_iter().chain([radius.si]) {
        identity.extend_from_slice(&coordinate.to_bits().to_le_bytes());
    }
    let source_digest = hash(&identity);
    let descriptor = FieldDescriptor {
        source: VersionedId {
            id: "luma.core.analytic.sphere".into(),
            version: 1,
        },
        fingerprint: source_digest.to_string(),
        world_min_m: [-1000.0; 3],
        world_max_m: [1000.0; 3],
        dtype: DType::F64,
        semantic_kind: ChannelSemantic::SignedDistanceEstimate,
        units: "m".into(),
        iso: None,
        material_labels: Vec::new(),
    };
    let sphere = Sphere {
        primitive,
        descriptor,
    };
    let registry = Registry::builtin()?;
    let sampled = sample_field(
        registry,
        SamplingRequest {
            grid,
            numeric_profile: VersionedId {
                id: "grid-si-v1".into(),
                version: 1,
            },
            channel_id: CHANNEL.into(),
            max_samples: if part_source.is_some() {
                32_768
            } else {
                262_144
            },
            max_blocks: 512,
            allow_unknown: false,
        },
        &sphere,
        &AtomicBool::new(false),
    )?;
    let artifact = encode_field(
        registry,
        sampled.field,
        source_digest,
        backend()?,
        Limits::default(),
    )?;
    let report = json!({
        "profile":"cad3-core-sphere-lvx/1","provider_pins":pins,"part_source":part_source,
        "source":sampled.source,"known_samples":sampled.known_samples,"unknown_samples":sampled.unknown_samples,
        "digests":artifact.digests,"bytes":artifact.bytes.len(),
    });
    Ok((artifact, report))
}

fn publish(output: &Path, bytes: &[u8]) -> Result<(), Diagnostic> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    temp.write_all(bytes).map_err(io_error)?;
    temp.as_file().sync_all().map_err(io_error)?;
    temp.persist_noclobber(output).map_err(io_error)?;
    Ok(())
}

fn backend() -> Result<BackendFingerprint, Diagnostic> {
    #[cfg(target_os = "linux")]
    let executable = Path::new("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let executable = std::env::current_exe().map_err(io_error)?;
    Ok(BackendFingerprint {
        id: "luma-cad-voxel.core-sphere".into(),
        release: env!("CARGO_PKG_VERSION").into(),
        build_digest: hash_reader(File::open(executable).map_err(io_error)?).map_err(io_error)?,
        dependency_digest: hash(include_bytes!("Cargo.lock")),
        target: env!("CAD_VOXEL_TARGET").into(),
        precision_profile: "GridSiF64V1".into(),
        algorithm_revision: 1,
    })
}

fn read(path: &Path, max_bytes: u64) -> Result<Vec<u8>, Diagnostic> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Diagnostic::new(
            DiagnosticCode::ResourceLimit,
            "Input exceeds byte limit",
        ));
    }
    Ok(bytes)
}

fn invalid(message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::InvalidRequest, message.to_string())
}

fn geometry(problem: living_frep::GeometryError) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::NumericRange, problem.to_string())
}

fn io_error(problem: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::IoFailure, problem.to_string())
}
