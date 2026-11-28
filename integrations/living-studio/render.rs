//! Prepare and submit pinned Studio stills and videos through the standalone render queue.
mod rebind;

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
const CLIP: &str = "ClipRef";
const STUDIO_PIN: &str = include_str!("luma-studio-pin.txt");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderInput {
    presentation_id: String,
    revision: u64,
    time: Option<RationalTime>,
    height: u16,
    #[serde(default)]
    profile: Profile,
    samples: Option<u16>,
    threads: Option<u16>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Profile {
    #[default]
    PreviewRaster,
    FinalPathtrace,
}

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Self::PreviewRaster => "preview-raster",
            Self::FinalPathtrace => "final-pathtrace",
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Still,
    Video,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::Still => "still",
            Self::Video => "video",
        }
    }

    fn result_type(self) -> &'static str {
        match self {
            Self::Still => "TaskSpecRef<PNGMaster>",
            Self::Video => "TaskSpecRef<Video>",
        }
    }

    fn outputs(self) -> Json {
        match self {
            Self::Still => json!(["png"]),
            Self::Video => json!(["png", "mp4"]),
        }
    }
}

struct RenderHost {
    registry: Registry,
    declared: AtomicBool,
    target: Target,
}

type Signature<'a> = (&'a str, &'a str, &'a [&'a str], &'a str, Effect);

impl RenderHost {
    fn new(target: Target) -> Result<Self, Error> {
        let (selector, handle) = match target {
            Target::Still => ("time", TIME),
            Target::Video => ("clip", CLIP),
        };
        let specs: [Signature<'_>; 4] = [
            (
                "studio",
                "presentation",
                &[],
                PRESENTATION,
                Effect::ReadPinned,
            ),
            ("render", selector, &[], handle, Effect::Pure),
            ("render", "options", &[], OPTIONS, Effect::Pure),
            (
                "render",
                target.name(),
                &[PRESENTATION, handle, OPTIONS],
                target.result_type(),
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
            target,
        })
    }
}

impl Dispatch for RenderHost {
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
            ("render.clip", []) => CLIP,
            ("render.options", []) => OPTIONS,
            ("render.still" | "render.video", [presentation, selector, options]) => {
                let kind = match self.target {
                    Target::Still => TIME,
                    Target::Video => CLIP,
                };
                for (value, kind) in [
                    (presentation, PRESENTATION),
                    (selector, kind),
                    (options, OPTIONS),
                ] {
                    if value.value_type() != Some(Type::Opaque(kind.into())) {
                        return Err(Error::new("RenderInputType", kind));
                    }
                }
                if self.declared.swap(true, Ordering::Relaxed) {
                    return Err(Error::new(
                        "RenderTaskLimit",
                        "Expected one render task declaration",
                    ));
                }
                self.target.result_type()
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

fn check_command(path: &Path, host: &impl Dispatch, code: &str) -> Result<String, Json> {
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
        json!({"code": code, "diagnostics": entries})
    };
    let wire: WireSourceBundle = read_json(path, 256 * 1024)?;
    let bundle = wire.admit().map_err(diagnostics)?;
    run_with_host(&bundle, Mode::Command, host, 100_000).map_err(diagnostics)?;
    Ok(bundle.source_digest())
}

fn studio_binary() -> Result<std::path::PathBuf, Json> {
    Ok(std::env::current_exe()
        .map_err(io_error)?
        .with_file_name("product-studio"))
}

fn execute() -> Result<Json, Json> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|action| action.to_str()) {
        Some("presentation-apply") => return rebind::apply(&args[1..]),
        Some("rebind-preview") => return rebind::preview(&args[1..]),
        Some("rebind-accept") => return rebind::accept(&args[1..]),
        _ => {}
    }
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
                "message": "luma-cad-studio prepare STUDIO PROJECT QUEUE RENDER.json COMMAND.json | submit STUDIO PROJECT QUEUE REQUEST RENDER.json COMMAND.json"
            }));
        }
    };
    let input: RenderInput = read_json(Path::new(options), 8192)?;
    let target = if input.time.is_some() {
        Target::Still
    } else {
        Target::Video
    };
    let profile = input.profile.name();
    let samples = input.samples.unwrap_or(16);
    let threads = input.threads.unwrap_or(4);
    if input.presentation_id.is_empty()
        || input
            .time
            .is_some_and(|time| time.ticks < 0 || time.scale == 0)
        || input.revision == 0
        || !((64..=1080).contains(&input.height) && input.height.is_multiple_of(2))
        || !(1..=128).contains(&samples)
        || !(1..=16).contains(&threads)
        || (input.profile == Profile::PreviewRaster
            && (input.samples.is_some() || input.threads.is_some()))
    {
        return Err(json!({"code":"InvalidRenderInput"}));
    }

    let host = RenderHost::new(target)
        .map_err(|error| json!({"code":error.code,"message":error.message}))?;
    let command_digest = check_command(Path::new(bundle), &host, "RenderCommandRejected")?;
    if !host.declared.into_inner() {
        return Err(
            json!({"code":"MissingRenderTask","message":"Expected one render task declaration"}),
        );
    }

    let root = Path::new(root).canonicalize().map_err(io_error)?;
    verify_studio(&root)?;
    let studio = studio_binary()?;
    let scratch = tempfile::tempdir().map_err(io_error)?;
    let packet = scratch.path().join("packet");
    let revision = input.revision.to_string();
    let mut capture_command = Command::new(studio);
    capture_command
        .arg("preview-data")
        .arg(project)
        .arg("--revision")
        .arg(&revision)
        .arg("--output")
        .arg(&packet);
    if let Some(time) = input.time {
        capture_command
            .arg("--time")
            .arg(format!("{}/{}", time.ticks, time.scale));
    } else {
        capture_command.arg("--sequence");
    }
    if input.profile == Profile::FinalPathtrace {
        capture_command.arg("--final-geometry");
    }
    let capture = provider(&mut capture_command)?;
    let scene_bytes = read(&packet.join("scene.json"), 32 * 1024 * 1024)?;
    let scene_digest = format!("sha256:{:x}", Sha256::digest(&scene_bytes));
    let scene: Json = serde_json::from_slice(&scene_bytes).map_err(invalid_json)?;
    let frame = &scene["frame"];
    if frame["presentation_id"] != input.presentation_id
        || frame["revision"] != revision.as_str()
        || input
            .time
            .is_some_and(|time| !requested_time(time, &frame["time"]))
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
        .arg(profile)
        .arg("--height")
        .arg(input.height.to_string());
    if input.profile == Profile::FinalPathtrace {
        command
            .arg("--samples")
            .arg(samples.to_string())
            .arg("--threads")
            .arg(threads.to_string());
    }
    if let Some(request) = request {
        command.arg("--key").arg(request);
    }
    let receipt = provider(&mut command)?;
    let sample_plan = &receipt["sample_plan"];
    let valid_sample_plan = match input.time {
        Some(time) => {
            sample_plan["mode"] == "still"
                && requested_time(time, &sample_plan["exact_time"])
                && receipt["frames"] == 1
                && receipt["fps"] == scene["still"]["fps"]
        }
        None => {
            sample_plan["mode"] == "sequence"
                && scene["sequence"]["frames"]
                    .as_array()
                    .is_some_and(|frames| receipt["frames"] == frames.len())
                && receipt["fps"] == scene["sequence"]["fps"]
        }
    };
    if receipt["presentation_id"] != input.presentation_id
        || receipt["revision"] != frame["revision"]
        || receipt["source_snapshot_digest"] != frame["source_snapshot_digest"]
        || receipt["profile"] != profile
        || receipt["height"] != input.height
        || receipt["outputs"] != target.outputs()
        || !valid_sample_plan
        || !receipt["frame_recipe_hash"].is_string()
        || receipt["inputs"]["scene.json"] != scene_digest
        || (request.is_some() && !receipt["id"].is_string())
    {
        return Err(json!({"code":"InvalidRenderReceipt"}));
    }
    let mut report = json!({
        "studio_revision": STUDIO_PIN.trim(),
        "command_digest": command_digest,
        "task": {
            "operation": format!("render.{}", target.name()), "result_type": target.result_type(), "policy": "historical_pinned",
            "frame_recipe_hash": receipt["frame_recipe_hash"], "inputs": receipt["inputs"],
            "presentation_id": frame["presentation_id"], "revision": frame["revision"],
            "source_snapshot_digest": frame["source_snapshot_digest"], "time": sample_plan["exact_time"],
            "fps": receipt["fps"], "frames": receipt["frames"],
            "camera": frame["camera"],
            "profile": profile, "width": receipt["width"], "height": receipt["height"],
            "cycles": (input.profile == Profile::FinalPathtrace).then(|| json!({"samples": samples, "threads": threads})),
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
