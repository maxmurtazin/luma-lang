//! Core-owned admission and durable outcomes for a pinned B-rep task batch.
use super::{jobs, *};
use luma::cad_engine::BrepTask;
use luma_cad_jobs as queue;

const TASK: &str = "brep-batch";
const OPERATION: &str = "brep.engineering_batch";
const SCHEMA: &str = "luma.brep.engineering_results/1";
const INPUT_DOMAIN: &[u8] = b"luma-brep-job-input/1";
const RESULT_DOMAIN: &[u8] = b"luma-brep-job-result/1";
const MAX_RESULT: usize = 8 * 1024 * 1024;

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

#[derive(Deserialize, Serialize)]
struct Admission {
    project: String,
    provider: String,
    request: String,
    input: BrepJobInput,
}

#[derive(Deserialize, Serialize)]
struct Output {
    project: String,
    job: queue::JobId,
    attempt: i64,
    body: String,
}

impl ProjectStore {
    pub fn admit_brep_job(
        &self,
        request: &str,
        provider: &str,
        input: BrepJobInput,
    ) -> Result<queue::Delivery, Error> {
        uuid(request)?;
        let request = request.to_ascii_lowercase();
        let registry = self.registry()?;
        let source = input.source.pin();
        jobs::definition_at(&registry, &source)?;
        let admission = Admission {
            project: self.project_id.clone(),
            provider: provider.into(),
            request,
            input,
        };
        let payload = text(seal(&self.key, INPUT_DOMAIN, &admission)?)?;
        let spec = task(source, provider, &payload);
        let contract = queue::TaskContract {
            major: spec.major,
            provider_fingerprint: provider.into(),
            output_schema: SCHEMA.into(),
            inputs: Default::default(),
        };
        let plan = queue::Plan::prepare(
            &[(OPERATION.into(), contract)].into(),
            &[(TASK.into(), spec)].into(),
            vec![queue::Intent {
                key: TASK.into(),
                task: TASK.into(),
            }],
        )
        .and_then(|plan| plan.with_payload(TASK, payload.clone()))
        .map_err(jobs::queue_error)?;

        let scope = self.brep_scope(&admission.request);
        let mut tx = jobs::job_write(self.db.luma_connection())?;
        queue::install_in(&tx).map_err(jobs::queue_error)?;
        let delivery = match queue::intent(&tx, &scope, TASK).map_err(jobs::queue_error)? {
            Some(delivery) => {
                if queue::task_payload(&tx, delivery.job)
                    .map_err(jobs::queue_error)?
                    .as_deref()
                    != Some(payload.as_str())
                {
                    return Err(error(
                        "RequestIdReuse",
                        "B-rep request already has a different plan",
                    ));
                }
                delivery
            }
            None => queue::admit(&mut tx, &scope, &plan)
                .map_err(jobs::queue_error)?
                .pop()
                .ok_or_else(Error::internal)?,
        };
        fault("brep-job-before-admission");
        tx.commit()
            .map_err(|problem| jobs::queue_error(problem.into()))?;
        fault("brep-job-after-admission");
        Ok(delivery)
    }

    pub fn brep_job_receipt(&self, request: &str) -> Result<queue::Delivery, Error> {
        uuid(request)?;
        let connection = self.db.luma_connection();
        if !queue::schema_present(connection).map_err(jobs::queue_error)? {
            return Err(unknown_job());
        }
        let request = request.to_ascii_lowercase();
        let scope = self.brep_scope(&request);
        let delivery = queue::intent(connection, &scope, TASK)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        let payload = queue::task_payload(connection, delivery.job)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        if self.brep_admission(&payload)?.request != request {
            return Err(error("CheckpointCorrupt", "B-rep job request differs"));
        }
        Ok(delivery)
    }

    pub fn run_brep_job(
        &self,
        provider: &str,
        compute: impl FnOnce(&BrepJobInput) -> Result<Value, &'static str>,
    ) -> Result<Option<queue::JobReceipt>, Error> {
        let connection = self.db.luma_connection();
        let mut tx = jobs::job_write(connection)?;
        queue::install_in(&tx).map_err(jobs::queue_error)?;
        let claimed = queue::claim(&mut tx, provider).map_err(jobs::queue_error)?;
        tx.commit()
            .map_err(|problem| jobs::queue_error(problem.into()))?;
        let Some(claimed) = claimed else {
            return Ok(None);
        };
        fault("brep-job-after-claim");

        let result = (|| {
            let payload = claimed.payload.as_deref().ok_or("InvalidJobPlan")?;
            let admission = self
                .brep_admission(payload)
                .map_err(|_| "CheckpointCorrupt")?;
            let source = admission.input.source.pin();
            if admission.provider != provider
                || claimed.task != task(source.clone(), provider, payload)
                || !claimed.inputs.is_empty()
            {
                return Err("InvalidJobPlan");
            }
            let registry = self.registry().map_err(|_| "CheckpointCorrupt")?;
            jobs::definition_at(&registry, &source).map_err(|_| "MissingPinnedSource")?;
            let result = compute(&admission.input)?;
            let body = serde_json::to_string(&result).map_err(|_| "Encoding")?;
            if body.len() > MAX_RESULT {
                return Err("ResourceLimit");
            }
            let manifest = queue::Artifact {
                schema: SCHEMA.into(),
                manifest_digest: digest(&body),
            };
            let output = Output {
                project: self.project_id.clone(),
                job: claimed.attempt.job,
                attempt: claimed.attempt.number,
                body,
            };
            let bytes = seal(&self.key, RESULT_DOMAIN, &output).map_err(|_| "ResourceLimit")?;
            let content = text(bytes).map_err(|_| "Encoding")?;
            Ok((manifest, content))
        })();

        let mut tx = jobs::job_write(connection)?;
        match result {
            Ok((manifest, content)) => {
                queue::complete_with_content(&mut tx, claimed.attempt, &manifest, Some(content))
                    .map_err(jobs::queue_error)?;
                queue::publish(&mut tx, claimed.attempt).map_err(jobs::queue_error)?;
            }
            Err(code) => queue::fail(&mut tx, claimed.attempt, code).map_err(jobs::queue_error)?,
        }
        let receipt = queue::receipt(&tx, claimed.attempt.job).map_err(jobs::queue_error)?;
        fault("brep-job-before-publication");
        tx.commit()
            .map_err(|problem| jobs::queue_error(problem.into()))?;
        Ok(Some(receipt))
    }

    pub fn brep_job_status(&self, id: i64) -> Result<queue::JobReceipt, Error> {
        let connection = self.db.luma_connection();
        let job = queue::JobId(id);
        if !queue::schema_present(connection).map_err(jobs::queue_error)? {
            return Err(unknown_job());
        }
        let input = queue::task_payload(connection, job)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        let admission = self.brep_admission(&input)?;
        let receipt = queue::receipt(connection, job).map_err(jobs::queue_error)?;
        if receipt.source != admission.input.source.pin() {
            return Err(error("CheckpointCorrupt", "B-rep job source differs"));
        }
        Ok(receipt)
    }

    pub fn brep_job_result(&self, id: i64) -> Result<Value, Error> {
        let receipt = self.brep_job_status(id)?;
        let manifest = receipt
            .artifact
            .filter(|manifest| {
                receipt.state == queue::State::Published && manifest.schema == SCHEMA
            })
            .ok_or_else(|| error("JobPending", "No published B-rep result is available"))?;
        let attempt = queue::Attempt {
            job: receipt.job,
            number: receipt.attempt,
        };
        let content = queue::artifact_content(self.db.luma_connection(), attempt)
            .map_err(jobs::queue_error)?
            .ok_or_else(|| error("CheckpointCorrupt", "Missing B-rep result"))?;
        let output: Output = unseal(&self.key, RESULT_DOMAIN, content.as_bytes())?;
        if output.project != self.project_id
            || output.job != receipt.job
            || output.attempt != receipt.attempt
            || digest(&output.body) != manifest.manifest_digest
        {
            return Err(error("CheckpointCorrupt", "B-rep job result differs"));
        }
        let value: Value = serde_json::from_str(&output.body).map_err(json_error)?;
        Ok(json!({"manifest":manifest,"result":value}))
    }

    fn brep_admission(&self, content: &str) -> Result<Admission, Error> {
        let admission: Admission = unseal(&self.key, INPUT_DOMAIN, content.as_bytes())?;
        if admission.project != self.project_id {
            return Err(error(
                "CheckpointCorrupt",
                "B-rep job belongs to a different project",
            ));
        }
        Ok(admission)
    }

    fn brep_scope(&self, request: &str) -> queue::Scope {
        jobs::scope(&self.project_id, &format!("brep/{request}"))
    }
}

fn task(source: queue::SourcePin, provider: &str, payload: &str) -> queue::TaskSpec {
    queue::TaskSpec {
        operation: OPERATION.into(),
        major: 1,
        provider_fingerprint: provider.into(),
        input_digest: digest(payload),
        source,
        policy: queue::Policy::HistoricalPinned,
        inputs: Default::default(),
    }
}

fn unknown_job() -> Error {
    error("UnknownJob", "B-rep job is absent")
}

fn text(bytes: Vec<u8>) -> Result<String, Error> {
    String::from_utf8(bytes).map_err(|_| Error::internal())
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn fault(stage: &str) {
    #[cfg(feature = "checkpoint-fault-injection")]
    if std::env::var("LUMA_CAD_TEST_CRASH_AT").as_deref() == Ok(stage) {
        std::process::exit(73);
    }
    let _ = stage;
}
