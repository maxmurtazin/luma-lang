//! Durable Core scheduler for the separately linked Voxel provider.
mod adaptive;
mod blocks;
mod paged;
mod provider;

use living_cad_viewer::checkpoint::project::ProjectStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Value>;
const MAX_LVX: u64 = 2 * 1024 * 1024;
const MAX_PROVIDER: u64 = 64 * 1024 * 1024;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match execute(&args) {
        Ok(value) => {
            println!("{}", json!({"result":value}));
            ExitCode::SUCCESS
        }
        Err(error) => {
            let code = if matches!(
                error["code"].as_str(),
                Some("Conflict" | "RequestIdReuse" | "InputChanged" | "Superseded")
            ) {
                4
            } else {
                2
            };
            println!("{}", json!({"error":error}));
            ExitCode::from(code)
        }
    }
}

fn execute(args: &[String]) -> Result<Value> {
    let [database, project, action, args @ ..] = args else {
        return Err(usage());
    };
    let key = std::env::var("LUMA_CAD_PROJECT_KEY")
        .ok()
        .and_then(|value| {
            let mut key = [0; 32];
            hex::decode_to_slice(value, &mut key).ok().map(|()| key)
        })
        .ok_or_else(|| json!({"code":"ProjectKey","message":"Expected a 32-byte hex key"}))?;
    let mut store = ProjectStore::open(Path::new(database), project, key).map_err(core_error)?;
    match (action.as_str(), args) {
        ("adaptive-sphere", [provider, part, revision, hash, profile, output]) => {
            let source = store
                .voxel_sphere_source(part, Some((revision.parse().map_err(|_| usage())?, hash)))
                .map_err(core_error)?;
            adaptive::publish_sphere(
                Path::new(provider),
                &source,
                Path::new(profile),
                Path::new(output),
            )
        }
        (action, args) if action.starts_with("adaptive-") => {
            adaptive::execute(&store, action, args)
        }
        ("mask", [publications, provider, args @ ..]) => blocks::execute(
            &mut store,
            Path::new(publications),
            Path::new(provider),
            args,
        ),
        ("paged", [publications, provider, directory, args @ ..]) => paged::execute(
            &store,
            Path::new(publications),
            Path::new(provider),
            Path::new(directory),
            args,
        ),
        ("source", [part]) => encode(store.voxel_sphere_source(part, None).map_err(core_error)?),
        ("lattice-source", [part]) => {
            encode(store.voxel_lattice_source(part, None).map_err(core_error)?)
        }
        (
            "submit" | "submit-luma" | "lattice-submit" | "lattice-submit-luma",
            [provider, request, part, revision, hash, grid, bundle @ ..],
        ) => {
            let revision = revision.parse().map_err(|_| usage())?;
            let grid: Value =
                serde_json::from_slice(&read(Path::new(grid), 16 * 1024)?).map_err(|_| usage())?;
            if grid["layout"] != "Node" {
                return Err(
                    json!({"code":"UnsupportedGrid","message":"The project profile requires a Node grid"}),
                );
            }
            let command_digest = match (action.as_str(), bundle) {
                ("submit" | "lattice-submit", []) => None,
                ("submit-luma" | "lattice-submit-luma", [bundle]) => {
                    Some(check_bundle(Path::new(bundle))?)
                }
                _ => return Err(usage()),
            };
            let lattice = action.starts_with("lattice-");
            let fingerprint = provider_fingerprint(Path::new(provider), lattice)?;
            let admitted = if lattice {
                store.admit_lattice_voxel_job(
                    request,
                    &fingerprint,
                    part,
                    (revision, hash),
                    grid,
                    command_digest,
                )
            } else {
                store.admit_voxel_job(
                    request,
                    &fingerprint,
                    part,
                    (revision, hash),
                    grid,
                    command_digest,
                )
            };
            encode(admitted.map_err(core_error)?)
        }
        ("work" | "lattice-work", [provider]) => {
            let directory = tempfile::tempdir().map_err(io_error)?;
            let executable =
                directory
                    .path()
                    .join(if cfg!(windows) { "voxel.exe" } else { "voxel" });
            let provider = Path::new(provider);
            let bytes = read(provider, MAX_PROVIDER)?;
            std::fs::write(&executable, &bytes).map_err(io_error)?;
            std::fs::set_permissions(
                &executable,
                std::fs::metadata(provider).map_err(io_error)?.permissions(),
            )
            .map_err(io_error)?;
            let lattice = action == "lattice-work";
            let fingerprint = build_fingerprint(&bytes, lattice);
            let worker = SamplingWorker {
                executable: &executable,
                directory: directory.path(),
            };
            let result = if lattice {
                store.run_lattice_voxel_job(&fingerprint, |input, current| {
                    worker.sample(
                        "lattice-pinned",
                        json!({"schema_version":1,"grid":input.grid,"part_source":input.source}),
                        input.command_digest.as_deref(),
                        current,
                    )
                })
            } else {
                store.run_voxel_job(&fingerprint, |input, current| {
                    worker.sample(
                        "sample-pinned",
                        json!({"schema_version":1,"grid":input.grid,"part_source":input.source,
                        "center_m":input.source.center_m,"radius_m":input.source.radius_m}),
                        input.command_digest.as_deref(),
                        current,
                    )
                })
            };
            encode(result.map_err(core_error)?)
        }
        ("receipt", [request]) => encode(store.voxel_job_receipt(request).map_err(core_error)?),
        ("lattice-receipt", [request]) => encode(
            store
                .lattice_voxel_job_receipt(request)
                .map_err(core_error)?,
        ),
        ("status" | "lattice-status" | "cancel" | "lattice-cancel", [id]) => {
            let id = number(id)?;
            let status = if action.starts_with("lattice-") {
                store.lattice_voxel_job_status(id)
            } else {
                store.voxel_job_status(id)
            }
            .map_err(core_error)?;
            if action.ends_with("cancel") {
                return encode(store.cancel_luma_job(id).map_err(core_error)?);
            }
            encode(status)
        }
        ("retry" | "lattice-retry", [id, attempt]) => {
            let id = number(id)?;
            if action == "lattice-retry" {
                store.lattice_voxel_job_status(id).map_err(core_error)?;
            } else {
                store.voxel_job_status(id).map_err(core_error)?;
            }
            encode(
                store
                    .retry_luma_job(id, number(attempt)?)
                    .map_err(core_error)?,
            )
        }
        ("export" | "lattice-export", [id, output]) => {
            let (bytes, provenance) = historical_lvx(&store, id, action == "lattice-export")?;
            publish(Path::new(output), &bytes)?;
            Ok(provenance)
        }
        ("field-csg", [provider, operation, left, right, metric]) => {
            if metric != "implicit" {
                return Err(json!({"code":"UnsupportedCapability",
                    "message":"Grid min/max CSG supports an implicit field with no distance guarantee"}));
            }
            let operation = match operation.as_str() {
                "union" => "Union",
                "intersection" => "Intersection",
                "difference" => "Difference",
                _ => return Err(usage()),
            };
            let provider_path = Path::new(provider);
            let directory = tempfile::tempdir().map_err(io_error)?;
            let operand = |name: &str, id: &str| -> Result<(Value, Value)> {
                let (bytes, provenance) = historical_lvx(&store, id, false)?;
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                let field = provider::decode(provider_path, &bytes, &digest)?;
                let path = directory.path().join(name);
                std::fs::write(&path, field.to_string()).map_err(io_error)?;
                Ok((json!({"path":path,"channel":"distance"}), provenance))
            };
            let (left, left_source) = operand("left.json", left)?;
            let (right, right_source) = operand("right.json", right)?;
            let field = provider::operate(
                provider_path,
                "living.voxel.field.csg",
                &json!({
                    "left":left,"right":right,"operation":operation,
                    "output_channel":"csg","max_visited_blocks":512
                }),
            )?;
            Ok(json!({"result":field,"sources":[left_source,right_source]}))
        }
        _ => Err(usage()),
    }
}

struct SamplingWorker<'a> {
    executable: &'a Path,
    directory: &'a Path,
}

impl SamplingWorker<'_> {
    fn sample(
        &self,
        command: &str,
        wire: Value,
        command_digest: Option<&str>,
        current: &dyn Fn() -> bool,
    ) -> std::result::Result<Value, &'static str> {
        let compute = || -> Result<Value> {
            let request = self.directory.join("request.json");
            let artifact = self.directory.join("sample.lvx");
            std::fs::write(&request, wire.to_string()).map_err(io_error)?;
            let stdout = tempfile::tempfile().map_err(io_error)?;
            let child = Command::new(self.executable)
                .arg(command)
                .arg(request)
                .arg(&artifact)
                .env_remove("LUMA_CAD_PROJECT_KEY")
                .env_remove("LUMA_CAD_TEST_CRASH_AT")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .stdout(stdout.try_clone().map_err(io_error)?)
                .spawn()
                .map_err(io_error)?;
            let response = read_response(child, stdout, current)?;
            let mut report = response["result"].clone();
            let bytes = read(&artifact, MAX_LVX)?;
            if report["part_source"] != wire["part_source"] || !verify_lvx(&report, &bytes) {
                return Err(corrupt());
            }
            report["command_digest"] = json!(command_digest);
            Ok(json!({"report":report,"lvx_hex":hex::encode(bytes)}))
        };
        compute().map_err(|_| "VoxelProviderRejected")
    }
}

fn historical_lvx(store: &ProjectStore, id: &str, lattice: bool) -> Result<(Vec<u8>, Value)> {
    let id = number(id)?;
    let result = if lattice {
        store.lattice_voxel_job_result(id)
    } else {
        store.voxel_job_result(id)
    }
    .map_err(core_error)?;
    let artifact = &result["result"];
    let report = &artifact["report"];
    let bytes =
        hex::decode(artifact["lvx_hex"].as_str().ok_or_else(corrupt)?).map_err(|_| corrupt())?;
    if !verify_lvx(report, &bytes) {
        return Err(corrupt());
    }
    Ok((
        bytes,
        json!({"report":report,"manifest":result["manifest"]}),
    ))
}

fn verify_lvx(report: &Value, bytes: &[u8]) -> bool {
    bytes.starts_with(b"LVXGRID\0")
        && report["digests"]["blob_digest"] == format!("{:x}", Sha256::digest(bytes))
        && report["bytes"] == bytes.len()
}

fn check_bundle(path: &Path) -> Result<String> {
    let bytes = read(path, 256 * 1024)?;
    let invalid_bundle = |message: String| json!({"code":"InvalidSourceBundle","message":message});
    let wire: luma::host::WireSourceBundle =
        serde_json::from_slice(&bytes).map_err(|error| invalid_bundle(error.to_string()))?;
    let bundle = wire
        .admit()
        .map_err(|error| invalid_bundle(format!("{:?}", error.diagnostics)))?;
    luma::cad::prepare_voxel_task(&bundle, 100_000)
        .map_err(|error| json!({"code":"VoxelTaskPlanRejected","message":format!("{error:?}")}))?;
    Ok(bundle.source_digest())
}

fn publish(output: &Path, bytes: &[u8]) -> Result<()> {
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

fn read_response(
    mut child: std::process::Child,
    mut stdout: File,
    current: &dyn Fn() -> bool,
) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(io_error)? {
            break status;
        }
        if !current() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(json!({"code":"Cancelled"}));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if !status.success() {
        return Err(json!({"code":"VoxelProviderRejected"}));
    }
    stdout.seek(SeekFrom::Start(0)).map_err(io_error)?;
    serde_json::from_slice(&limit(stdout, 64 * 1024)?).map_err(|_| corrupt())
}

fn provider_fingerprint(path: &Path, lattice: bool) -> Result<String> {
    Ok(build_fingerprint(&read(path, MAX_PROVIDER)?, lattice))
}

fn build_fingerprint(bytes: &[u8], lattice: bool) -> String {
    let mut hash = Sha256::new();
    // The queue claims by provider fingerprint. Keep the two sealed payload types isolated.
    let domain: &[u8] = if lattice {
        b"luma.voxel.gyroid.provider/1\0"
    } else {
        b"luma.voxel.provider/1\0"
    };
    hash.update(domain);
    hash.update(bytes);
    format!("sha256:{:x}", hash.finalize())
}

fn read(path: &Path, max: u64) -> Result<Vec<u8>> {
    limit(File::open(path).map_err(io_error)?, max)
}

fn limit(reader: impl Read, max: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > max {
        return Err(json!({"code":"ResourceLimit","message":"Input exceeds byte limit"}));
    }
    Ok(bytes)
}

fn number(value: &str) -> Result<i64> {
    value
        .parse()
        .ok()
        .filter(|number| *number >= 0)
        .ok_or_else(usage)
}

fn encode(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| json!({"code":"Encoding"}))
}

fn core_error(error: living_cad_viewer::Error) -> Value {
    serde_json::to_value(error).unwrap_or_else(|_| json!({"code":"CoreError"}))
}

fn io_error(error: impl std::fmt::Display) -> Value {
    json!({"code":"IoFailure","message":error.to_string()})
}

fn corrupt() -> Value {
    json!({"code":"CorruptArtifact","message":"Invalid LVX or provenance"})
}

fn usage() -> Value {
    json!({"code":"Usage","message":"DB PROJECT source PART | adaptive-sphere VOXEL_CLI PART REVISION HASH PROFILE.json OUTPUT.lvxadap | adaptive-submit VOXEL_CLI REQUEST PART REVISION HASH PROFILE.json | adaptive-work VOXEL_CLI | adaptive-receipt REQUEST | adaptive-status JOB | adaptive-cancel JOB | adaptive-retry JOB ATTEMPT | adaptive-export JOB OUTPUT.lvxadap | lattice-source PART | lattice-submit[-luma] PROVIDER REQUEST PART REVISION HASH GRID.json [BUNDLE.json] | lattice-work PROVIDER | lattice-receipt REQUEST | lattice-status JOB | lattice-cancel JOB | lattice-retry JOB ATTEMPT | lattice-export JOB FILE.lvx | submit PROVIDER REQUEST PART REVISION HASH GRID.json | submit-luma PROVIDER REQUEST PART REVISION HASH GRID.json BUNDLE.json | work PROVIDER | receipt REQUEST | status JOB | cancel JOB | retry JOB ATTEMPT | export JOB FILE.lvx | field-csg VOXEL_CLI OP LEFT_JOB RIGHT_JOB implicit | mask PUBLICATIONS VOXEL_CLI COMMAND ... | paged PUBLICATIONS VOXEL_CLI DIRECTORY COMMAND ..."})
}
