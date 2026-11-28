//! Accepted Core geometry and its durable Voxel job inputs.
use super::{artifact_jobs::JobInput, jobs, *};
use luma_cad_jobs as queue;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SphereSource {
    pub project: String,
    pub document: String,
    pub entity: String,
    pub revision: u64,
    pub definition_hash: String,
    pub center_m: [f64; 3],
    pub radius_m: f64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VoxelJobInput {
    pub source: SphereSource,
    pub grid: Value,
    pub command_digest: Option<String>,
}

impl JobInput for VoxelJobInput {
    const PREFIX: &str = "voxel";
    const TASK: &str = "voxel-sphere";
    const OPERATION: &str = "voxel.sample.sphere";
    const SCHEMA: &str = "luma.voxel.lvx/1";
    const INPUT_DOMAIN: &[u8] = b"luma-voxel-job-input/1";
    const RESULT_DOMAIN: &[u8] = b"luma-voxel-job-result/1";

    fn pin(&self) -> queue::SourcePin {
        queue::SourcePin {
            document: self.source.document.clone(),
            revision: self.source.revision,
            definition_digest: jobs::normalized_hash(&self.source.definition_hash),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatticeSource {
    pub project: String,
    pub document: String,
    pub entity: String,
    pub revision: u64,
    pub definition_hash: String,
    pub field_json: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatticeJobInput {
    pub source: LatticeSource,
    pub grid: Value,
    pub command_digest: Option<String>,
}

impl JobInput for LatticeJobInput {
    const PREFIX: &str = "voxel-lattice";
    const TASK: &str = "voxel-gyroid";
    const OPERATION: &str = "voxel.sample.gyroid";
    const SCHEMA: &str = "luma.voxel.lvx/1";
    const INPUT_DOMAIN: &[u8] = b"luma-voxel-gyroid-input/1";
    const RESULT_DOMAIN: &[u8] = b"luma-voxel-gyroid-result/1";

    fn pin(&self) -> queue::SourcePin {
        queue::SourcePin {
            document: self.source.document.clone(),
            revision: self.source.revision,
            definition_digest: jobs::normalized_hash(&self.source.definition_hash),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveJobInput {
    pub source: SphereSource,
    pub profile: Value,
}

impl JobInput for AdaptiveJobInput {
    const PREFIX: &str = "adaptive-sphere";
    const TASK: &str = "adaptive-sphere";
    const OPERATION: &str = "voxel.adapt.sphere";
    const SCHEMA: &str = "luma.voxel.lvxadap/1";
    const INPUT_DOMAIN: &[u8] = b"luma-voxel-adaptive-input/1";
    const RESULT_DOMAIN: &[u8] = b"luma-voxel-adaptive-result/1";

    fn pin(&self) -> queue::SourcePin {
        queue::SourcePin {
            document: self.source.document.clone(),
            revision: self.source.revision,
            definition_digest: jobs::normalized_hash(&self.source.definition_hash),
        }
    }
}

impl ProjectStore {
    pub fn voxel_sphere_source(
        &self,
        document: &str,
        pin: Option<(u64, &str)>,
    ) -> Result<SphereSource, Error> {
        let (revision, hash, authored) = self.voxel_part(document, pin)?;
        let editor = crate::editor::inspect(&authored)?;
        let unsupported = || {
            error(
                "UnsupportedGeometry",
                "Expected an active Core editor sphere",
            )
        };
        let entity = editor.output.ok_or_else(unsupported)?.to_string();
        let node = editor
            .nodes
            .iter()
            .find(|node| node["id"] == entity && node["kind"] == "Sphere")
            .ok_or_else(unsupported)?;
        let length = |name: &str| {
            node["properties"][name]
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value / 1000.0)
                .ok_or_else(unsupported)
        };
        Ok(SphereSource {
            project: self.project_id.clone(),
            document: document.into(),
            entity,
            revision,
            definition_hash: hash,
            center_m: [
                length("position_x")?,
                length("position_y")?,
                length("position_z")?,
            ],
            radius_m: length("radius")?,
        })
    }

    fn voxel_part(
        &self,
        document: &str,
        pin: Option<(u64, &str)>,
    ) -> Result<(u64, String, Value), Error> {
        let registry = self.registry()?;
        let (head, _, current) = part_head(&registry, document)?;
        let (revision, hash, native) = match pin {
            None => (head.native_revision, head.definition_hash, current),
            Some((revision, hash)) => {
                let native = jobs::definition_at(
                    &registry,
                    &queue::SourcePin {
                        document: document.into(),
                        revision,
                        definition_digest: jobs::normalized_hash(hash),
                    },
                )?;
                (revision, hash.trim_start_matches("sha256:").into(), native)
            }
        };
        Ok((
            revision,
            hash,
            serde_json::from_str(native).map_err(json_error)?,
        ))
    }

    pub fn voxel_lattice_source(
        &self,
        document: &str,
        pin: Option<(u64, &str)>,
    ) -> Result<LatticeSource, Error> {
        let (revision, definition_hash, authored) = self.voxel_part(document, pin)?;
        let editor = crate::editor::inspect(&authored)?;
        let unsupported = || {
            error(
                "UnsupportedGeometry",
                "Expected an active Core Gyroid lattice",
            )
        };
        let entity = editor.output.ok_or_else(unsupported)?.to_string();
        let lattice = authored["geometry"]["nodes"]
            .as_array()
            .is_some_and(|nodes| {
                nodes
                    .iter()
                    .any(|node| node["id"] == entity && node["kind"] == "cad.lattice_fill.v1")
            });
        if !lattice {
            return Err(unsupported());
        }
        let field = crate::editor::lattice_projection(&authored, &entity)?.field;
        if !matches!(
            field.intent().cell,
            living_frep::lattice::Cell::Gyroid { .. }
        ) {
            return Err(unsupported());
        }
        Ok(LatticeSource {
            project: self.project_id.clone(),
            document: document.into(),
            entity,
            revision,
            definition_hash,
            field_json: String::from_utf8(field.encode()).map_err(|_| Error::internal())?,
        })
    }

    pub fn admit_lattice_voxel_job(
        &self,
        request: &str,
        provider: &str,
        document: &str,
        pin: (u64, &str),
        grid: Value,
        command_digest: Option<String>,
    ) -> Result<queue::Delivery, Error> {
        let source = self.voxel_lattice_source(document, Some(pin))?;
        self.admit_artifact_job(
            request,
            provider,
            LatticeJobInput {
                source,
                grid,
                command_digest,
            },
        )
    }

    pub fn lattice_voxel_job_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        self.artifact_job_receipt::<LatticeJobInput>(request)
    }

    pub fn run_lattice_voxel_job(
        &self,
        provider: &str,
        compute: impl FnOnce(&LatticeJobInput, &dyn Fn() -> bool) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_voxel_artifact_job(provider, compute)
    }

    pub fn lattice_voxel_job_status(&self, id: i64) -> Result<queue::JobReceipt, Error> {
        self.artifact_job_status::<LatticeJobInput>(id)
    }

    pub fn lattice_voxel_job_result(&self, id: i64) -> Result<Value, Error> {
        self.artifact_job_result::<LatticeJobInput>(id)
    }

    pub fn admit_adaptive_voxel_job(
        &self,
        request: &str,
        provider: &str,
        part: &str,
        pin: (u64, &str),
        profile: Value,
    ) -> Result<queue::Delivery, Error> {
        let source = self.voxel_sphere_source(part, Some(pin))?;
        self.admit_artifact_job(request, provider, AdaptiveJobInput { source, profile })
    }

    pub fn adaptive_voxel_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        self.artifact_job_receipt::<AdaptiveJobInput>(request)
    }

    pub fn run_adaptive_voxel_job(
        &self,
        provider: &str,
        compute: impl FnOnce(&AdaptiveJobInput, &dyn Fn() -> bool) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_voxel_artifact_job(provider, compute)
    }

    pub fn adaptive_voxel_status(&self, id: i64) -> Result<queue::JobReceipt, Error> {
        self.artifact_job_status::<AdaptiveJobInput>(id)
    }

    pub fn adaptive_voxel_result(&self, id: i64) -> Result<Value, Error> {
        self.artifact_job_result::<AdaptiveJobInput>(id)
    }

    pub fn admit_voxel_job(
        &self,
        request: &str,
        provider: &str,
        document: &str,
        pin: (u64, &str),
        grid: Value,
        command_digest: Option<String>,
    ) -> Result<queue::Delivery, Error> {
        let source = self.voxel_sphere_source(document, Some(pin))?;
        self.admit_artifact_job(
            request,
            provider,
            VoxelJobInput {
                source,
                grid,
                command_digest,
            },
        )
    }

    pub fn voxel_job_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        self.artifact_job_receipt::<VoxelJobInput>(request)
    }

    pub fn run_voxel_job(
        &self,
        provider: &str,
        compute: impl FnOnce(&VoxelJobInput, &dyn Fn() -> bool) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_voxel_artifact_job(provider, compute)
    }

    fn run_voxel_artifact_job<T: JobInput>(
        &self,
        provider: &str,
        compute: impl FnOnce(&T, &dyn Fn() -> bool) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_artifact_job(provider, |input, attempt| {
            let current = || {
                queue::receipt(self.db.luma_connection(), attempt.job).is_ok_and(|receipt| {
                    receipt.state == queue::State::Running && receipt.attempt == attempt.number
                })
            };
            compute(input, &current)
        })
    }

    pub fn voxel_job_status(&self, id: i64) -> Result<queue::JobReceipt, Error> {
        self.artifact_job_status::<VoxelJobInput>(id)
    }

    pub fn voxel_job_result(&self, id: i64) -> Result<Value, Error> {
        self.artifact_job_result::<VoxelJobInput>(id)
    }
}
