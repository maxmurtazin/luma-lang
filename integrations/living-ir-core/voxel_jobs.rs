//! Accepted Core editor sphere and its durable Voxel job inputs.
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

impl ProjectStore {
    pub fn voxel_sphere_source(
        &self,
        document: &str,
        pin: Option<(u64, &str)>,
    ) -> Result<SphereSource, Error> {
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
        let authored: Value = serde_json::from_str(native).map_err(json_error)?;
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
