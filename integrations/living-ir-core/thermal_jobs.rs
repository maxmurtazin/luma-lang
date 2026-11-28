//! Historical CAD-4 studies on accepted, axis-aligned editor Boxes.
use super::{artifact_jobs::JobInput, jobs, *};
use luma_cad_jobs as queue;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoxSource {
    pub project: String,
    pub document: String,
    pub entity: String,
    pub revision: u64,
    pub definition_hash: String,
    pub origin_m: [f64; 3],
    pub extent_m: [f64; 3],
}

#[derive(Deserialize, Serialize)]
pub struct HeatInput {
    pub source: BoxSource,
    pub study: Value,
}

impl JobInput for HeatInput {
    const PREFIX: &str = "heat";
    const TASK: &str = "stationary-box-heat";
    const OPERATION: &str = "simulation.stationary.box_heat";
    const SCHEMA: &str = "luma.simulation.thermal-archive/1";
    const INPUT_DOMAIN: &[u8] = b"luma-thermal-job-input/1";
    const RESULT_DOMAIN: &[u8] = b"luma-thermal-job-result/1";

    fn pin(&self) -> queue::SourcePin {
        queue::SourcePin {
            document: self.source.document.clone(),
            revision: self.source.revision,
            definition_digest: jobs::normalized_hash(&self.source.definition_hash),
        }
    }
}

impl ProjectStore {
    pub fn heat_box_source(
        &self,
        document: &str,
        pin: Option<(u64, &str)>,
    ) -> Result<BoxSource, Error> {
        let (revision, definition_hash, authored) = self.voxel_part(document, pin)?;
        let editor = crate::editor::inspect(&authored)?;
        let unsupported = || {
            error(
                "UnsupportedGeometry",
                "Expected an active axis-aligned Core Box",
            )
        };
        let entity = editor.output.ok_or_else(unsupported)?.to_string();
        let node = editor
            .nodes
            .iter()
            .find(|node| node["id"] == entity && node["kind"] == "Box")
            .ok_or_else(unsupported)?;
        let properties = &node["properties"];
        if ["rotation_x", "rotation_y", "rotation_z"]
            .iter()
            .any(|key| properties[key].as_f64() != Some(0.0))
        {
            return Err(unsupported());
        }
        let millimetres = |name: &str| {
            properties[name]
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value / 1000.0)
                .ok_or_else(unsupported)
        };
        let extent_m = [
            millimetres("width")?,
            millimetres("height")?,
            millimetres("depth")?,
        ];
        if extent_m.iter().any(|size| *size <= 0.0) {
            return Err(unsupported());
        }
        let centre = [
            millimetres("position_x")?,
            millimetres("position_y")?,
            millimetres("position_z")?,
        ];
        let origin_m = std::array::from_fn(|axis| centre[axis] - extent_m[axis] / 2.0);
        Ok(BoxSource {
            project: self.project_id.clone(),
            document: document.into(),
            entity,
            revision,
            definition_hash,
            origin_m,
            extent_m,
        })
    }

    pub fn admit_heat(
        &self,
        request: &str,
        provider: &str,
        part: &str,
        pin: (u64, &str),
        study: Value,
    ) -> Result<queue::Delivery, Error> {
        let source = self.heat_box_source(part, Some(pin))?;
        self.admit_artifact_job(request, provider, HeatInput { source, study })
    }

    pub fn run_heat(
        &self,
        provider: &str,
        compute: impl FnOnce(&HeatInput, &dyn Fn() -> bool) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_voxel_artifact_job(provider, compute)
    }

    pub fn heat_status(&self, job: i64) -> Result<queue::JobReceipt, Error> {
        self.artifact_job_status::<HeatInput>(job)
    }

    pub fn heat_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        self.artifact_job_receipt::<HeatInput>(request)
    }

    pub fn heat_result(&self, job: i64) -> Result<Value, Error> {
        self.artifact_job_result::<HeatInput>(job)
    }
}
