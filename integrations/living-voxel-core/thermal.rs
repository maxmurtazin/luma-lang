//! CAD-4 Box heat studies, served through the native Core project CLI.
use super::{core_error, encode, io_error, number, publish, read, read_response, usage, Result};
use living_cad_viewer::checkpoint::project::{luma_cad::thermal_jobs::HeatInput, ProjectStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_PROVIDER: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024;
const PROFILE: &str = "bootstrap-volume-heat-3d-fv-preview-v1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Study {
    cells: [u16; 3],
    material: Material,
    min_x_temperature_k: f64,
    max_x_temperature_k: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Material {
    id: String,
    generation: u64,
    conductivity_w_per_m_k: f64,
}

impl Study {
    fn parse(raw: &[u8]) -> Result<Self> {
        let invalid = || json!({"code":"InvalidHeatStudy"});
        let study: Self = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if study.cells.iter().any(|cell| !(1..=32).contains(cell))
            || study
                .cells
                .iter()
                .map(|cell| u32::from(*cell))
                .product::<u32>()
                > 4096
            || study.material.id.trim().is_empty()
            || study.material.id.len() > 128
            || study.material.generation == 0
            || !study.material.conductivity_w_per_m_k.is_finite()
            || study.material.conductivity_w_per_m_k <= 0.0
            || [study.min_x_temperature_k, study.max_x_temperature_k]
                .iter()
                .any(|t| !t.is_finite() || *t <= 0.0)
        {
            return Err(invalid());
        }
        Ok(study)
    }

    fn problem(&self, [width, height, depth]: [f64; 3]) -> Value {
        let [x, y, z] = self.cells;
        json!({
            "schema":"living.simulation.bootstrap.volume-heat/v1",
            "width_m":width,"height_m":height,"depth_m":depth,
            "cells_x":x,"cells_y":y,"cells_z":z,
            "conductivity_w_per_m_k":self.material.conductivity_w_per_m_k,
            "volumetric_heat_source_w_per_m3":0.0,
            "boundaries":[
              {"side":"min_x","condition":{"kind":"temperature","temperature_k":self.min_x_temperature_k}},
              {"side":"max_x","condition":{"kind":"temperature","temperature_k":self.max_x_temperature_k}},
              {"side":"min_y","condition":{"kind":"insulated"}},
              {"side":"max_y","condition":{"kind":"insulated"}},
              {"side":"min_z","condition":{"kind":"insulated"}},
              {"side":"max_z","condition":{"kind":"insulated"}}
            ]
        })
    }
}

pub(super) fn execute(store: &ProjectStore, action: &str, args: &[String]) -> Result<Value> {
    match (action, args) {
        ("heat-source", [part]) => encode(store.heat_box_source(part, None).map_err(core_error)?),
        ("heat-submit", [provider, request, part, revision, hash, study]) => {
            let revision = revision.parse().map_err(|_| usage())?;
            // Reject unsupported geometry before opening the solver executable.
            store
                .heat_box_source(part, Some((revision, hash)))
                .map_err(core_error)?;
            let study = Study::parse(&read(Path::new(study), 8192)?)?;
            let digest = sha(&read(Path::new(provider), MAX_PROVIDER)?);
            encode(
                store
                    .admit_heat(
                        request,
                        &fingerprint(&digest),
                        part,
                        (revision, hash),
                        encode(study)?,
                    )
                    .map_err(core_error)?,
            )
        }
        ("heat-work", [provider]) => {
            let solver = Solver::open(Path::new(provider))?;
            encode(
                store
                    .run_heat(&fingerprint(&solver.digest), |input, current| {
                        solver.compute(input, current).map_err(|fault| {
                            match fault["provider_status"]
                                .as_str()
                                .or_else(|| fault["code"].as_str())
                            {
                                Some("NotConverged") => "NotConverged",
                                Some("Unresolved") => "Unresolved",
                                Some("LimitExceeded") => "LimitExceeded",
                                Some("Cancelled") => "Cancelled",
                                _ => "SimulationProviderRejected",
                            }
                        })
                    })
                    .map_err(core_error)?,
            )
        }
        ("heat-status", [job]) => encode(store.heat_status(number(job)?).map_err(core_error)?),
        ("heat-receipt", [request]) => encode(store.heat_receipt(request).map_err(core_error)?),
        ("heat-cancel", [job]) => {
            let job = number(job)?;
            store.heat_status(job).map_err(core_error)?;
            encode(store.cancel_luma_job(job).map_err(core_error)?)
        }
        ("heat-retry", [job, attempt]) => {
            let job = number(job)?;
            store.heat_status(job).map_err(core_error)?;
            encode(
                store
                    .retry_luma_job(job, number(attempt)?)
                    .map_err(core_error)?,
            )
        }
        ("heat-export", [job, path]) => {
            let (report, bytes) = recover(store, job)?;
            publish(Path::new(path), &bytes)?;
            Ok(report)
        }
        ("heat-cell", [provider, job, x, y, z]) => {
            let (report, bytes) = recover(store, job)?;
            let report = &report["report"];
            let solver = Solver::open(Path::new(provider))?;
            if report["provider_binary_sha256"] != solver.digest {
                return Err(json!({"code":"InputChanged"}));
            }
            fs::write(solver.problem(), report["problem"].to_string()).map_err(io_error)?;
            fs::write(solver.archive(), bytes).map_err(io_error)?;
            let answer = solver.call(
                "query",
                &[
                    solver.problem().as_os_str(),
                    solver.archive().as_os_str(),
                    "--cell".as_ref(),
                    x.as_ref(),
                    y.as_ref(),
                    z.as_ref(),
                ],
                &|| true,
            )?;
            if answer["status"] != "Ready"
                || answer["problem_digest"] != report["summary"]["problem_digest"]
                || answer["quantity"] != "absolute_temperature"
                || answer["unit"] != "K"
            {
                return Err(corrupt());
            }
            Ok(answer)
        }
        _ => Err(usage()),
    }
}

struct Solver {
    directory: tempfile::TempDir,
    executable: PathBuf,
    digest: String,
}

impl Solver {
    fn open(provider: &Path) -> Result<Self> {
        let directory = tempfile::tempdir().map_err(io_error)?;
        let executable = directory
            .path()
            .join(if cfg!(windows) { "lsim.exe" } else { "lsim" });
        let bytes = read(provider, MAX_PROVIDER)?;
        let digest = sha(&bytes);
        fs::write(&executable, bytes).map_err(io_error)?;
        fs::set_permissions(
            &executable,
            fs::metadata(provider).map_err(io_error)?.permissions(),
        )
        .map_err(io_error)?;
        Ok(Self {
            directory,
            executable,
            digest,
        })
    }

    fn problem(&self) -> PathBuf {
        self.directory.path().join("problem.json")
    }
    fn archive(&self) -> PathBuf {
        self.directory.path().join("result.lsim")
    }

    fn call(
        &self,
        action: &str,
        args: &[&std::ffi::OsStr],
        current: &dyn Fn() -> bool,
    ) -> Result<Value> {
        if !current() {
            return Err(json!({"code":"Cancelled"}));
        }
        let stdout = tempfile::tempfile().map_err(io_error)?;
        let child = Command::new(&self.executable)
            .arg(action)
            .args(args)
            .env_remove("LUMA_CAD_PROJECT_KEY")
            .env_remove("LUMA_CAD_TEST_CRASH_AT")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(stdout.try_clone().map_err(io_error)?)
            .spawn()
            .map_err(io_error)?;
        read_response(child, stdout, current)
    }

    fn compute(&self, input: &HeatInput, current: &dyn Fn() -> bool) -> Result<Value> {
        let study = Study::parse(input.study.to_string().as_bytes())?;
        let problem = study.problem(input.source.extent_m);
        let request = self.problem();
        let archive = self.archive();
        fs::write(&request, problem.to_string()).map_err(io_error)?;
        let accepted = self.call(
            "run",
            &[
                "--stationary".as_ref(),
                "--profile".as_ref(),
                PROFILE.as_ref(),
                request.as_os_str(),
                archive.as_os_str(),
            ],
            current,
        )?;
        let checked = self.call(
            "inspect",
            &[request.as_os_str(), archive.as_os_str()],
            current,
        )?;
        let summary = &checked["summary"];
        if accepted["status"] != "Accepted"
            || accepted["profile"] != PROFILE
            || checked["status"] != "Ready"
            || summary["profile"] != PROFILE
            || summary["qualification"] != "Experimental"
            || !summary["problem_digest"].is_string()
            || summary["problem_digest"] != accepted["problem_digest"]
        {
            return Err(corrupt());
        }
        let bytes = read(&archive, MAX_ARCHIVE)?;
        if !bytes.starts_with(b"PK\x03\x04") {
            return Err(corrupt());
        }
        let report = json!({
            "profile":"cad4-box-heat-v1","source":input.source,"study":input.study,
            "problem":problem,"provider_binary_sha256":self.digest,
            "simulation_revision":include_str!("simulation-pin.txt").trim(),
            "outcome":"Accepted","summary":summary,
            "archive_sha256":sha(&bytes)
        });
        Ok(json!({"report":report,"archive_hex":hex::encode(bytes)}))
    }
}

fn recover(store: &ProjectStore, job: &str) -> Result<(Value, Vec<u8>)> {
    let result = store.heat_result(number(job)?).map_err(core_error)?;
    let artifact = &result["result"];
    let bytes = hex::decode(artifact["archive_hex"].as_str().ok_or_else(corrupt)?)
        .map_err(|_| corrupt())?;
    let report = &artifact["report"];
    if bytes.len() as u64 > MAX_ARCHIVE
        || !bytes.starts_with(b"PK\x03\x04")
        || report["archive_sha256"] != sha(&bytes)
    {
        return Err(corrupt());
    }
    Ok((
        json!({"report":report,"manifest":result["manifest"]}),
        bytes,
    ))
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn fingerprint(digest: &str) -> String {
    format!(
        "sha256:{}",
        sha(format!("luma.simulation.heat.provider/1\0{digest}").as_bytes())
    )
}
fn corrupt() -> Value {
    json!({"code":"CorruptThermalArtifact"})
}
