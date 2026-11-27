//! Core-owned admission and durable outcomes for a pinned B-rep task batch.
use super::{artifact_jobs::JobInput, jobs, *};
use luma::cad_engine::BrepTask;
use luma_cad_jobs as queue;

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrepSource {
    pub publication: String,
    pub geometry_sha256: String,
    pub provenance_sha256: String,
    pub document: String,
    pub revision: u64,
    pub definition_digest: String,
}

impl BrepSource {
    fn pin(&self) -> queue::SourcePin {
        queue::SourcePin {
            document: self.document.clone(),
            revision: self.revision,
            definition_digest: jobs::normalized_hash(&self.definition_digest),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrepJobInput {
    pub source: BrepSource,
    pub source_digest: String,
    pub tasks: Vec<BrepTask>,
}

impl JobInput for BrepJobInput {
    const PREFIX: &str = "brep";
    const TASK: &str = "brep-batch";
    const OPERATION: &str = "brep.engineering_batch";
    const SCHEMA: &str = "luma.brep.engineering_results/1";
    const INPUT_DOMAIN: &[u8] = b"luma-brep-job-input/1";
    const RESULT_DOMAIN: &[u8] = b"luma-brep-job-result/1";
    fn pin(&self) -> queue::SourcePin {
        self.source.pin()
    }
}

impl ProjectStore {
    pub fn admit_brep_job(
        &self,
        request: &str,
        provider: &str,
        input: BrepJobInput,
    ) -> Result<queue::Delivery, Error> {
        self.admit_artifact_job(request, provider, input)
    }
    pub fn brep_job_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        self.artifact_job_receipt::<BrepJobInput>(request)
    }
    pub fn run_brep_job(
        &self,
        provider: &str,
        compute: impl FnOnce(&BrepJobInput) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        self.run_artifact_job(provider, |input, _| compute(input))
    }
    pub fn brep_job_status(&self, id: i64) -> Result<queue::JobReceipt, Error> {
        self.artifact_job_status::<BrepJobInput>(id)
    }
    pub fn brep_job_result(&self, id: i64) -> Result<Value, Error> {
        self.artifact_job_result::<BrepJobInput>(id)
    }
}
