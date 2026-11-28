//! Prepare and submit one pinned Studio still through the standalone render queue.
use luma::{
    host::{WireSourceBundle, run_with_host},
    host_api::{
        CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
        Value,
    },
};
use luma_cad_contract::SEALED_MODULES;
use presentation_core::RationalTime;
use serde::Deserialize;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::Path,
    process::{Command, ExitCode},
    sync::atomic::{AtomicBool, Ordering},
};

const PRESENTATION: &str = "PresentationRef";
const TIME: &str = "Time";
const OPTIONS: &str = "RenderOptionsRef";
const TASK: &str = "TaskSpecRef<PNGMaster>";
const PROFILE: &str = "preview-raster";
const STUDIO_PIN: &str = include_str!("luma-studio-pin.txt");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Still {
    presentation_id: String,
    revision: u64,
    time: RationalTime,
    height: u16,
}

struct StillHost {
    registry: Registry,
    declared: AtomicBool,
}

type Signature<'a> = (&'a str, &'a str, &'a [&'a str], &'a str, Effect);

impl StillHost {
    fn new() -> Result<Self, Error> {
        let specs: [Signature<'_>; 4] = [
            (
                "studio",
                "presentation",
                &[],
                PRESENTATION,
                Effect::ReadPinned,
            ),
            ("render", "time", &[], TIME, Effect::Pure),
            ("render", "options", &[], OPTIONS, Effect::Pure),
            (
                "render",
                "still",
                &[PRESENTATION, TIME, OPTIONS],
                TASK,
                Effect::DeclareTask,
            ),
        ];
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            specs
                .into_iter()
                .map(|(module, name, parameters, result, effect)| Descriptor {
                    module: module.into(),
                    name: name.into(),
                    operation: format!("{module}.{name}"),
                    major: 1,
                    parameters: parameters
                        .iter()
                        .map(|name| Parameter::Exact(Type::Opaque((*name).into())))
                        .collect(),
                    result: ResultType::Exact(Type::Opaque(result.into())),
                    effect,
                    modes: [Mode::Command].into(),
                }),
        )?;
        Ok(Self {
            registry,
            declared: AtomicBool::new(false),
        })
    }
}

impl Dispatch for StillHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        let type_name = match (descriptor.operation.as_str(), args) {
            ("studio.presentation", []) => PRESENTATION,
            ("render.time", []) => TIME,
            ("render.options", []) => OPTIONS,
            ("render.still", [presentation, time, options]) => {
                for (value, kind) in [
                    (presentation, PRESENTATION),
                    (time, TIME),
                    (options, OPTIONS),
                ] {
                    if value.value_type() != Some(Type::Opaque(kind.into())) {
                        return Err(Error::new("RenderInputType", kind));
                    }
                }
                if self.declared.swap(true, Ordering::Relaxed) {
                    return Err(Error::new(
                        "RenderTaskLimit",
                        "Expected one render.still declaration",
                    ));
                }
                TASK
            }
            _ => return Err(Error::new("RenderInputType", &descriptor.operation)),
        };
        Ok(Value::Opaque {
            type_name: type_name.into(),
            payload: Json::Null,
        })
    }
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>, Json> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > limit {
        return Err(json!({"code":"InputLimit"}));
    }
    Ok(bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T, Json> {
    serde_json::from_slice(&read(path, limit)?).map_err(invalid_json)
}

fn io_error(error: std::io::Error) -> Json {
    json!({"code":"IoFailure","message":error.to_string()})
}

fn invalid_json(error: serde_json::Error) -> Json {
    json!({"code":"InvalidInput","message":error.to_string()})
}

fn provider(command: &mut Command) -> Result<Json, Json> {
    let output = command
        .env_remove("LUMA_CAD_PROJECT_KEY")
        .env_remove("STUDIO_CAD_TOKEN")
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let native = detail
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str::<Json>(line).ok());
        return Err(
            json!({"code":"StudioRejected","diagnostic":native.unwrap_or_else(|| json!(detail))}),
        );
    }
    serde_json::from_slice(&output.stdout).map_err(invalid_json)
}

fn verify_studio(root: &Path) -> Result<(), Json> {
    let head = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(io_error)?;
    let clean = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--quiet", "HEAD", "--", "apps/standalone"])
        .status()
        .map_err(io_error)?;
    if !head.status.success()
        || String::from_utf8_lossy(&head.stdout).trim() != STUDIO_PIN.trim()
        || !clean.success()
    {
        return Err(json!({"code":"StudioPinMismatch","revision":STUDIO_PIN.trim()}));
    }
    Ok(())
}

fn requested_time(time: RationalTime, actual: &Json) -> bool {
    serde_json::from_value::<RationalTime>(actual.clone()).is_ok_and(|actual| {
        actual.scale > 0
            && i128::from(time.ticks) * i128::from(actual.scale)
                == i128::from(actual.ticks) * i128::from(time.scale)
    })
}

fn execute() -> Result<Json, Json> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (root, project, queue, request, options, bundle) = match args.as_slice() {
        [action, root, project, queue, options, bundle] if action == "prepare" => {
            (root, project, queue, None, options, bundle)
        }
        [action, root, project, queue, request, options, bundle] if action == "submit" => {
            (root, project, queue, Some(request), options, bundle)
        }
        _ => {
            return Err(json!({
                "code": "Usage",
                "message": "luma-cad-studio prepare STUDIO PROJECT QUEUE STILL.json COMMAND.json | submit STUDIO PROJECT QUEUE REQUEST STILL.json COMMAND.json"
            }));
        }
    };
    let still: Still = read_json(Path::new(options), 8192)?;
    if still.presentation_id.is_empty()
        || still.time.ticks < 0
        || still.time.scale == 0
        || still.revision == 0
        || !((64..=1080).contains(&still.height) && still.height.is_multiple_of(2))
    {
        return Err(json!({"code":"InvalidStill"}));
    }

    let diagnostics = |error: luma::runtime::Failure| {
        let entries: Vec<_> = error
            .diagnostics
            .into_iter()
            .map(|(file, diagnostic)| {
                json!({
                    "file": file, "code": diagnostic.code, "message": diagnostic.message,
                    "start": diagnostic.span.start, "end": diagnostic.span.end
                })
            })
            .collect();
        json!({"code":"RenderCommandRejected","diagnostics":entries})
    };
    let wire: WireSourceBundle = read_json(Path::new(bundle), 256 * 1024)?;
    let bundle = wire.admit().map_err(diagnostics)?;
    let host =
        StillHost::new().map_err(|error| json!({"code":error.code,"message":error.message}))?;
    run_with_host(&bundle, Mode::Command, &host, 100_000).map_err(diagnostics)?;
    if !host.declared.into_inner() {
        return Err(
            json!({"code":"MissingRenderTask","message":"Expected one render.still declaration"}),
        );
    }

    let root = Path::new(root).canonicalize().map_err(io_error)?;
    verify_studio(&root)?;
    let studio = std::env::current_exe()
        .map_err(io_error)?
        .with_file_name("product-studio");
    let scratch = tempfile::tempdir().map_err(io_error)?;
    let packet = scratch.path().join("packet");
    let revision = still.revision.to_string();
    let capture = provider(
        Command::new(studio)
            .arg("preview-data")
            .arg(project)
            .arg("--revision")
            .arg(&revision)
            .arg("--time")
            .arg(format!("{}/{}", still.time.ticks, still.time.scale))
            .arg("--output")
            .arg(&packet),
    )?;
    let scene_bytes = read(&packet.join("scene.json"), 8 * 1024 * 1024)?;
    let scene_digest = format!("sha256:{:x}", Sha256::digest(&scene_bytes));
    let scene: Json = serde_json::from_slice(&scene_bytes).map_err(invalid_json)?;
    let frame = &scene["frame"];
    if frame["presentation_id"] != still.presentation_id
        || frame["revision"] != revision.as_str()
        || !requested_time(still.time, &frame["time"])
        || !frame["source_snapshot_digest"].is_string()
    {
        return Err(json!({"code":"PresentationPinMismatch"}));
    }

    let mut command = Command::new("node");
    command
        .arg(root.join("apps/standalone/tools/render-queue.mjs"))
        .arg("--root")
        .arg(queue)
        .arg(if request.is_some() {
            "submit"
        } else {
            "preflight"
        })
        .arg(packet)
        .arg("--profile")
        .arg(PROFILE)
        .arg("--height")
        .arg(still.height.to_string())
        .arg("--png-only");
    if let Some(request) = request {
        command.arg("--key").arg(request);
    }
    let receipt = provider(&mut command)?;
    if receipt["presentation_id"] != still.presentation_id
        || receipt["revision"] != frame["revision"]
        || receipt["source_snapshot_digest"] != frame["source_snapshot_digest"]
        || receipt["profile"] != PROFILE
        || receipt["height"] != still.height
        || receipt["frames"] != 1
        || receipt["outputs"] != json!(["png"])
        || !requested_time(still.time, &receipt["sample_plan"]["exact_time"])
        || !receipt["frame_recipe_hash"].is_string()
        || receipt["inputs"]["scene.json"] != scene_digest
        || (request.is_some() && !receipt["id"].is_string())
    {
        return Err(json!({"code":"InvalidRenderReceipt"}));
    }
    let mut report = json!({
        "studio_revision": STUDIO_PIN.trim(),
        "command_digest": bundle.source_digest(),
        "task": {
            "operation": "render.still", "result_type": TASK, "policy": "historical_pinned",
            "frame_recipe_hash": receipt["frame_recipe_hash"], "inputs": receipt["inputs"],
            "presentation_id": frame["presentation_id"], "revision": frame["revision"],
            "source_snapshot_digest": frame["source_snapshot_digest"], "time": frame["time"],
            "camera": frame["camera"],
            "profile": PROFILE, "width": receipt["width"], "height": receipt["height"],
            "estimated_bytes": receipt["estimated_bytes"], "outputs": receipt["outputs"]
        },
        "capture": {
            "provider": capture,
            "cad_document_id": scene["cad_document_id"],
            "cad_revision": scene["cad_revision"],
            "cad_geometry": scene["cad_geometry"]
        }
    });
    if request.is_some() {
        report["receipt"] = receipt;
    }
    Ok(report)
}

fn main() -> ExitCode {
    let (status, envelope) = match execute() {
        Ok(result) => (
            ExitCode::SUCCESS,
            json!({"protocol":"luma.cad.studio-render/1","result":result}),
        ),
        Err(error) => (
            ExitCode::from(2),
            json!({"protocol":"luma.cad.studio-render/1","error":error}),
        ),
    };
    println!("{envelope}");
    status
}
