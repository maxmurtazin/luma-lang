//! CAD-4 stationary Box studies, served through the native Core project CLI.
use super::{
    core_error, encode, io_error, number, publish, read, read_bundle, read_response, usage, Result,
};
use living_cad_viewer::checkpoint::project::{luma_cad::thermal_jobs::BoxSource, ProjectStore};
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
struct Profile {
    cad: &'static str,
    native: &'static str,
    cell: (&'static str, &'static str),
    boundary: (&'static str, &'static str),
}

const HEAT: Profile = Profile {
    cad: "cad4-box-heat-v1",
    native: "bootstrap-volume-heat-3d-fv-preview-v1",
    cell: ("absolute_temperature", "K"),
    boundary: ("heat_flow", "W"),
};
const DIFFUSION: Profile = Profile {
    cad: "cad4-box-diffusion-v1",
    native: "bootstrap-volume-diffusion-3d-fv-preview-v1",
    cell: ("amount_concentration", "mol/m3"),
    boundary: ("molar_flow", "mol/s"),
};
const DC: Profile = Profile {
    cad: "cad4-box-dc-v1",
    native: "bootstrap-volume-dc-3d-fv-preview-v1",
    cell: ("electric_potential", "V"),
    boundary: ("electric_current", "A"),
};

fn valid_grid(cells: [u16; 3]) -> bool {
    cells.iter().all(|cell| (1..=32).contains(cell))
        && cells.into_iter().map(u32::from).product::<u32>() <= 4096
}

fn valid_material(id: &str, generation: u64, coefficient: f64) -> bool {
    !id.trim().is_empty()
        && id.len() <= 128
        && generation > 0
        && coefficient.is_finite()
        && coefficient > 0.0
}

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
        if !valid_grid(study.cells)
            || !valid_material(
                &study.material.id,
                study.material.generation,
                study.material.conductivity_w_per_m_k,
            )
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TransportStudy {
    cells: [u16; 3],
    material: TransportMaterial,
    boundaries: EndConditions,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TransportMaterial {
    id: String,
    generation: u64,
    coefficient: f64,
    unit: TransportUnit,
}

#[derive(Deserialize, Serialize)]
enum TransportUnit {
    #[serde(rename = "m2/s")]
    Diffusivity,
    #[serde(rename = "S/m")]
    Conductivity,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum EndConditions {
    Diffusion {
        min_x_concentration_mol_per_m3: f64,
        max_x_concentration_mol_per_m3: f64,
    },
    Dc {
        min_x_potential_v: f64,
        max_x_potential_v: f64,
    },
}

impl TransportStudy {
    fn parse(raw: &[u8]) -> Result<Self> {
        let invalid = || json!({"code":"InvalidTransportStudy"});
        let study: Self = serde_json::from_slice(raw)
            .map_err(|error| json!({"code":"InvalidTransportStudy","message":error.to_string()}))?;
        let valid_ends = match (study.boundaries, &study.material.unit) {
            (
                EndConditions::Diffusion {
                    min_x_concentration_mol_per_m3,
                    max_x_concentration_mol_per_m3,
                },
                TransportUnit::Diffusivity,
            ) => [
                min_x_concentration_mol_per_m3,
                max_x_concentration_mol_per_m3,
            ]
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0),
            (
                EndConditions::Dc {
                    min_x_potential_v,
                    max_x_potential_v,
                },
                TransportUnit::Conductivity,
            ) => min_x_potential_v.is_finite() && max_x_potential_v.is_finite(),
            _ => false,
        };
        if !valid_grid(study.cells)
            || !valid_material(
                &study.material.id,
                study.material.generation,
                study.material.coefficient,
            )
            || !valid_ends
        {
            return Err(invalid());
        }
        Ok(study)
    }

    fn problem(&self, [width, height, depth]: [f64; 3]) -> (&'static Profile, Value) {
        let (profile, schema, coefficient, min, max, wall) = match self.boundaries {
            EndConditions::Diffusion {
                min_x_concentration_mol_per_m3,
                max_x_concentration_mol_per_m3,
            } => (
                &DIFFUSION,
                "living.simulation.bootstrap.volume-diffusion/v1",
                "diffusivity_m2_per_s",
                json!({"kind":"concentration","concentration_mol_per_m3":min_x_concentration_mol_per_m3}),
                json!({"kind":"concentration","concentration_mol_per_m3":max_x_concentration_mol_per_m3}),
                "impermeable",
            ),
            EndConditions::Dc {
                min_x_potential_v,
                max_x_potential_v,
            } => (
                &DC,
                "living.simulation.bootstrap.volume-dc/v1",
                "conductivity_s_per_m",
                json!({"kind":"potential","potential_v":min_x_potential_v}),
                json!({"kind":"potential","potential_v":max_x_potential_v}),
                "insulated",
            ),
        };
        let [x, y, z] = self.cells;
        let mut problem = json!({
            "schema":schema,
            "width_m":width,"height_m":height,"depth_m":depth,
            "cells_x":x,"cells_y":y,"cells_z":z,
            "boundaries":[
              {"side":"min_x","condition":min},
              {"side":"max_x","condition":max},
              {"side":"min_y","condition":{"kind":wall}},
              {"side":"max_y","condition":{"kind":wall}},
              {"side":"min_z","condition":{"kind":wall}},
              {"side":"max_z","condition":{"kind":wall}}
            ]
        });
        problem[coefficient] = json!(self.material.coefficient);
        if matches!(self.boundaries, EndConditions::Diffusion { .. }) {
            problem["volumetric_source_mol_per_m3_s"] = json!(0.0);
        }
        (profile, problem)
    }
}

#[derive(Clone, Copy)]
enum Family {
    Heat,
    Transport,
}

impl Family {
    fn fingerprint(self, digest: &str) -> String {
        let name = match self {
            Self::Heat => "heat",
            Self::Transport => "transport",
        };
        format!(
            "sha256:{}",
            sha(format!("luma.simulation.{name}.provider/1\0{digest}").as_bytes())
        )
    }

    fn status(self, store: &ProjectStore, job: i64) -> Result<Value> {
        encode(
            match self {
                Self::Heat => store.heat_status(job),
                Self::Transport => store.transport_status(job),
            }
            .map_err(core_error)?,
        )
    }

    fn profile(self, report: &Value) -> Result<&'static Profile> {
        match (self, report["profile"].as_str()) {
            (Self::Heat, Some(profile)) if profile == HEAT.cad => Ok(&HEAT),
            (Self::Transport, Some(profile)) if profile == DIFFUSION.cad => Ok(&DIFFUSION),
            (Self::Transport, Some(profile)) if profile == DC.cad => Ok(&DC),
            _ => Err(corrupt()),
        }
    }
}

pub(super) fn execute(store: &ProjectStore, action: &str, args: &[String]) -> Result<Value> {
    let (name, action) = action.split_once('-').ok_or_else(usage)?;
    let family = match name {
        "heat" => Family::Heat,
        "transport" => Family::Transport,
        _ => return Err(usage()),
    };
    match (action, args) {
        ("source", [part]) => encode(store.heat_box_source(part, None).map_err(core_error)?),
        (
            "submit" | "submit-luma",
            [provider, request, part, revision, hash, study, bundle @ ..],
        ) => {
            let revision = revision.parse().map_err(|_| usage())?;
            let pin = (revision, hash.as_str());
            // Reject unsupported geometry before opening the solver executable.
            store.heat_box_source(part, Some(pin)).map_err(core_error)?;
            let raw = read(Path::new(study), 8192)?;
            let study = match family {
                Family::Heat => encode(Study::parse(&raw)?)?,
                Family::Transport => encode(TransportStudy::parse(&raw)?)?,
            };
            let command_digest = match (action, bundle) {
                ("submit", []) => None,
                ("submit-luma", [bundle]) => {
                    let bundle = read_bundle(Path::new(bundle))?;
                    luma::cad::prepare_simulation_task(&bundle, 100_000).map_err(|error| {
                        json!({"code":"SimulationTaskPlanRejected","message":format!("{error:?}")})
                    })?;
                    Some(bundle.source_digest())
                }
                _ => return Err(usage()),
            };
            let fingerprint = family.fingerprint(&sha(&read(Path::new(provider), MAX_PROVIDER)?));
            encode(
                match family {
                    Family::Heat => {
                        store.admit_heat(request, &fingerprint, part, pin, study, command_digest)
                    }
                    Family::Transport => store.admit_transport(
                        request,
                        &fingerprint,
                        part,
                        pin,
                        study,
                        command_digest,
                    ),
                }
                .map_err(core_error)?,
            )
        }
        ("work", [provider]) => {
            let solver = Solver::open(Path::new(provider))?;
            let fingerprint = family.fingerprint(&solver.digest);
            encode(
                match family {
                    Family::Heat => store.run_heat(&fingerprint, |input, current| {
                        let study = Study::parse(input.study.to_string().as_bytes())
                            .map_err(worker_fault)?;
                        let problem = study.problem(input.source.extent_m);
                        solver
                            .compute(
                                &input.source,
                                &input.study,
                                input.command_digest.as_deref(),
                                &HEAT,
                                problem,
                                current,
                            )
                            .map_err(worker_fault)
                    }),
                    Family::Transport => store.run_transport(&fingerprint, |input, current| {
                        let study = TransportStudy::parse(input.study.to_string().as_bytes())
                            .map_err(worker_fault)?;
                        let (profile, problem) = study.problem(input.source.extent_m);
                        solver
                            .compute(
                                &input.source,
                                &input.study,
                                input.command_digest.as_deref(),
                                profile,
                                problem,
                                current,
                            )
                            .map_err(worker_fault)
                    }),
                }
                .map_err(core_error)?,
            )
        }
        ("status", [job]) => family.status(store, number(job)?),
        ("receipt", [request]) => encode(
            match family {
                Family::Heat => store.heat_receipt(request),
                Family::Transport => store.transport_receipt(request),
            }
            .map_err(core_error)?,
        ),
        ("cancel", [job]) => {
            let job = number(job)?;
            family.status(store, job)?;
            encode(store.cancel_luma_job(job).map_err(core_error)?)
        }
        ("retry", [job, attempt]) => {
            let job = number(job)?;
            family.status(store, job)?;
            encode(
                store
                    .retry_luma_job(job, number(attempt)?)
                    .map_err(core_error)?,
            )
        }
        ("export", [job, path]) => {
            let (report, bytes) = recover(store, family, job)?;
            publish(Path::new(path), &bytes)?;
            Ok(report)
        }
        ("cell" | "cell-luma", [provider, job, x, y, z, bundle @ ..]) => {
            let index = [
                x.parse().map_err(|_| usage())?,
                y.parse().map_err(|_| usage())?,
                z.parse().map_err(|_| usage())?,
            ];
            let bundle = query_bundle(action, bundle)?;
            query(
                store,
                family,
                provider,
                job,
                Selector::Cell(index),
                bundle.as_ref(),
            )
        }
        ("boundary" | "boundary-luma", [provider, job, side, bundle @ ..]) => {
            if !["min_x", "max_x", "min_y", "max_y", "min_z", "max_z"].contains(&side.as_str()) {
                return Err(usage());
            }
            let bundle = query_bundle(action, bundle)?;
            query(
                store,
                family,
                provider,
                job,
                Selector::Boundary(side),
                bundle.as_ref(),
            )
        }
        ("view", [_job]) => {
            Err(json!({"code":"UnsupportedCapability","capability":"simulation-to-view"}))
        }
        _ => Err(usage()),
    }
}

fn worker_fault(fault: Value) -> &'static str {
    match fault["provider_status"]
        .as_str()
        .or_else(|| fault["code"].as_str())
    {
        Some("NotConverged") => "NotConverged",
        Some("Unresolved") => "Unresolved",
        Some("LimitExceeded") => "LimitExceeded",
        Some("Cancelled") => "Cancelled",
        Some("InternalFault") => "InternalFault",
        Some("Rejected") => "Rejected",
        _ => "SimulationProviderRejected",
    }
}

enum Selector<'a> {
    Cell([u32; 3]),
    Boundary(&'a str),
}

fn query_bundle(action: &str, args: &[String]) -> Result<Option<luma::host::SourceBundle>> {
    match (action.ends_with("-luma"), args) {
        (false, []) => Ok(None),
        (true, [path]) => read_bundle(Path::new(path)).map(Some),
        _ => Err(usage()),
    }
}

fn query(
    store: &ProjectStore,
    family: Family,
    provider: &str,
    job: &str,
    selector: Selector<'_>,
    bundle: Option<&luma::host::SourceBundle>,
) -> Result<Value> {
    let (result, bytes) = recover(store, family, job)?;
    let report = &result["report"];
    let profile = family.profile(report)?;
    let (measure, address, arguments) = match selector {
        Selector::Cell(index) => {
            let [x, y, z] = index.map(|i| i.to_string());
            (
                profile.cell,
                json!({"association":"cell_average","index":index}),
                vec!["--cell".into(), x, y, z],
            )
        }
        Selector::Boundary(side) => (
            profile.boundary,
            json!({"association":"boundary_outflow","side":side}),
            vec!["--boundary".into(), side.into()],
        ),
    };
    let (quantity, unit) = measure;
    let solver = Solver::open(Path::new(provider))?;
    if report["provider_binary_sha256"] != solver.digest {
        return Err(json!({"code":"InputChanged"}));
    }
    let problem = solver.problem();
    let archive = solver.archive();
    fs::write(&problem, report["problem"].to_string()).map_err(io_error)?;
    fs::write(&archive, bytes).map_err(io_error)?;
    let mut args = vec![problem.as_os_str(), archive.as_os_str()];
    args.extend(arguments.iter().map(std::ffi::OsStr::new));
    let answer = solver.call("query", &args, &|| true)?;
    if answer["status"] != "Ready"
        || answer["schema"] != "living.simulation.bootstrap.stationary-query/v1"
        || answer["profile"] != profile.native
        || answer["problem_digest"] != report["summary"]["problem_digest"]
        || answer["quantity"] != quantity
        || answer["unit"] != unit
        || answer["address"] != address
        || !answer["value"].as_f64().is_some_and(f64::is_finite)
    {
        return Err(corrupt());
    }
    if let Some(bundle) = bundle {
        luma::cad::read_simulation_observation(bundle, &answer, 100_000).map_err(
            |error| json!({"code":"SimulationQueryRejected","message":format!("{error:?}")}),
        )?;
    }
    Ok(answer)
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

    fn compute(
        &self,
        source: &BoxSource,
        study: &Value,
        command_digest: Option<&str>,
        profile: &Profile,
        problem: Value,
        current: &dyn Fn() -> bool,
    ) -> Result<Value> {
        let request = self.problem();
        let archive = self.archive();
        fs::write(&request, problem.to_string()).map_err(io_error)?;
        let accepted = self.call(
            "run",
            &[
                "--stationary".as_ref(),
                "--profile".as_ref(),
                profile.native.as_ref(),
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
            || accepted["profile"] != profile.native
            || checked["status"] != "Ready"
            || summary["profile"] != profile.native
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
        let mut report = json!({
            "profile":profile.cad,"source":source,"study":study,
            "problem":problem,"provider_binary_sha256":self.digest,
            "simulation_revision":include_str!("simulation-pin.txt").trim(),
            "outcome":"Accepted","summary":summary,
            "archive_sha256":sha(&bytes)
        });
        if let Some(digest) = command_digest {
            report["command_digest"] = json!(digest);
        }
        Ok(json!({"report":report,"archive_hex":hex::encode(bytes)}))
    }
}

fn recover(store: &ProjectStore, family: Family, job: &str) -> Result<(Value, Vec<u8>)> {
    let job = number(job)?;
    let result = match family {
        Family::Heat => store.heat_result(job),
        Family::Transport => store.transport_result(job),
    }
    .map_err(core_error)?;
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
fn corrupt() -> Value {
    json!({"code":"CorruptThermalArtifact"})
}
