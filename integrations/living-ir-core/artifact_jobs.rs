//! Core-owned transport for durable historical artifacts.
use super::{jobs, *};
use luma_cad_jobs as queue;
use serde::de::DeserializeOwned;

const MAX_RESULT: usize = 8 * 1024 * 1024;

pub(super) trait JobInput: Serialize + DeserializeOwned {
    const PREFIX: &str;
    const TASK: &str;
    const OPERATION: &str;
    const SCHEMA: &str;
    const INPUT_DOMAIN: &[u8];
    const RESULT_DOMAIN: &[u8];

    fn pin(&self) -> queue::SourcePin;
}

#[derive(Deserialize, Serialize)]
struct Admission<T> {
    project: String,
    provider: String,
    request: String,
    input: T,
}

#[derive(Deserialize, Serialize)]
struct Output {
    project: String,
    job: queue::JobId,
    attempt: i64,
    body: String,
}

impl ProjectStore {
    pub(super) fn admit_artifact_job<T: JobInput>(
        &self,
        request: &str,
        provider: &str,
        input: T,
    ) -> Result<queue::Delivery, Error> {
        uuid(request)?;
        let request = request.to_ascii_lowercase();
        let registry = self.registry()?;
        let source = input.pin();
        jobs::definition_at(&registry, &source)?;
        let admission = Admission {
            project: self.project_id.clone(),
            provider: provider.into(),
            request,
            input,
        };
        let payload = text(seal(&self.key, T::INPUT_DOMAIN, &admission)?)?;
        let spec = task::<T>(source, provider, &payload);
        let contract = queue::TaskContract {
            major: spec.major,
            provider_fingerprint: provider.into(),
            output_schema: T::SCHEMA.into(),
            inputs: Default::default(),
        };
        let plan = queue::Plan::prepare(
            &[(T::OPERATION.into(), contract)].into(),
            &[(T::TASK.into(), spec)].into(),
            vec![queue::Intent {
                key: T::TASK.into(),
                task: T::TASK.into(),
            }],
        )
        .and_then(|plan| plan.with_payload(T::TASK, payload.clone()))
        .map_err(jobs::queue_error)?;

        let scope = self.artifact_scope::<T>(&admission.request);
        let mut tx = jobs::job_write(self.db.luma_connection())?;
        queue::install_in(&tx).map_err(jobs::queue_error)?;
        let delivery = match queue::intent(&tx, &scope, T::TASK).map_err(jobs::queue_error)? {
            Some(delivery) => {
                if queue::task_payload(&tx, delivery.job)
                    .map_err(jobs::queue_error)?
                    .as_deref()
                    != Some(payload.as_str())
                {
                    return Err(error(
                        "RequestIdReuse",
                        "Artifact request already has a different plan",
                    ));
                }
                delivery
            }
            None => queue::admit(&mut tx, &scope, &plan)
                .map_err(jobs::queue_error)?
                .pop()
                .ok_or_else(Error::internal)?,
        };
        fault(&format!("{}-job-before-admission", T::PREFIX));
        tx.commit()
            .map_err(|problem| jobs::queue_error(problem.into()))?;
        fault(&format!("{}-job-after-admission", T::PREFIX));
        Ok(delivery)
    }

    pub(super) fn artifact_job_receipt<T: JobInput>(
        &self,
        request: &str,
    ) -> Result<queue::Delivery, Error> {
        uuid(request)?;
        let connection = self.db.luma_connection();
        if !queue::schema_present(connection).map_err(jobs::queue_error)? {
            return Err(unknown_job());
        }
        let request = request.to_ascii_lowercase();
        let scope = self.artifact_scope::<T>(&request);
        let delivery = queue::intent(connection, &scope, T::TASK)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        let payload = queue::task_payload(connection, delivery.job)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        if self.artifact_admission::<T>(&payload)?.request != request {
            return Err(error("CheckpointCorrupt", "Artifact job request differs"));
        }
        Ok(delivery)
    }

    pub(super) fn run_artifact_job<T: JobInput>(
        &self,
        provider: &str,
        compute: impl FnOnce(&T, queue::Attempt) -> Result<Value, &'static str>,
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
        fault(&format!("{}-job-after-claim", T::PREFIX));

        let result = (|| {
            let payload = claimed.payload.as_deref().ok_or("InvalidJobPlan")?;
            let admission = self
                .artifact_admission::<T>(payload)
                .map_err(|_| "CheckpointCorrupt")?;
            let source = admission.input.pin();
            if admission.provider != provider
                || claimed.task != task::<T>(source.clone(), provider, payload)
                || !claimed.inputs.is_empty()
            {
                return Err("InvalidJobPlan");
            }
            let registry = self.registry().map_err(|_| "CheckpointCorrupt")?;
            jobs::definition_at(&registry, &source).map_err(|_| "MissingPinnedSource")?;
            let result = compute(&admission.input, claimed.attempt)?;
            let body = serde_json::to_string(&result).map_err(|_| "Encoding")?;
            if body.len() > MAX_RESULT {
                return Err("ResourceLimit");
            }
            let manifest = queue::Artifact {
                schema: T::SCHEMA.into(),
                manifest_digest: digest(&body),
            };
            let output = Output {
                project: self.project_id.clone(),
                job: claimed.attempt.job,
                attempt: claimed.attempt.number,
                body,
            };
            let bytes = seal(&self.key, T::RESULT_DOMAIN, &output).map_err(|_| "ResourceLimit")?;
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
        fault(&format!("{}-job-before-publication", T::PREFIX));
        tx.commit()
            .map_err(|problem| jobs::queue_error(problem.into()))?;
        Ok(Some(receipt))
    }

    pub(super) fn artifact_job_status<T: JobInput>(
        &self,
        id: i64,
    ) -> Result<queue::JobReceipt, Error> {
        let connection = self.db.luma_connection();
        let job = queue::JobId(id);
        if !queue::schema_present(connection).map_err(jobs::queue_error)? {
            return Err(unknown_job());
        }
        let input = queue::task_payload(connection, job)
            .map_err(jobs::queue_error)?
            .ok_or_else(unknown_job)?;
        let admission = self.artifact_admission::<T>(&input)?;
        let receipt = queue::receipt(connection, job).map_err(jobs::queue_error)?;
        if receipt.source != admission.input.pin() {
            return Err(error("CheckpointCorrupt", "Artifact job source differs"));
        }
        Ok(receipt)
    }

    pub(super) fn artifact_job_result<T: JobInput>(&self, id: i64) -> Result<Value, Error> {
        let receipt = self.artifact_job_status::<T>(id)?;
        let manifest = receipt
            .artifact
            .filter(|manifest| {
                receipt.state == queue::State::Published && manifest.schema == T::SCHEMA
            })
            .ok_or_else(|| error("JobPending", "No published artifact result is available"))?;
        let attempt = queue::Attempt {
            job: receipt.job,
            number: receipt.attempt,
        };
        let content = queue::artifact_content(self.db.luma_connection(), attempt)
            .map_err(jobs::queue_error)?
            .ok_or_else(|| error("CheckpointCorrupt", "Missing artifact result"))?;
        let output: Output = unseal(&self.key, T::RESULT_DOMAIN, content.as_bytes())?;
        if output.project != self.project_id
            || output.job != receipt.job
            || output.attempt != receipt.attempt
            || digest(&output.body) != manifest.manifest_digest
        {
            return Err(error("CheckpointCorrupt", "Artifact job result differs"));
        }
        let value: Value = serde_json::from_str(&output.body).map_err(json_error)?;
        Ok(json!({"manifest":manifest,"result":value}))
    }

    fn artifact_admission<T: JobInput>(&self, content: &str) -> Result<Admission<T>, Error> {
        let admission: Admission<T> = unseal(&self.key, T::INPUT_DOMAIN, content.as_bytes())?;
        if admission.project != self.project_id {
            return Err(error(
                "CheckpointCorrupt",
                "Artifact job belongs to a different project",
            ));
        }
        Ok(admission)
    }

    fn artifact_scope<T: JobInput>(&self, request: &str) -> queue::Scope {
        jobs::scope(&self.project_id, &format!("{}/{request}", T::PREFIX))
    }
}

fn task<T: JobInput>(source: queue::SourcePin, provider: &str, payload: &str) -> queue::TaskSpec {
    queue::TaskSpec {
        operation: T::OPERATION.into(),
        major: 1,
        provider_fingerprint: provider.into(),
        input_digest: digest(payload),
        source,
        policy: queue::Policy::HistoricalPinned,
        inputs: Default::default(),
    }
}

fn unknown_job() -> Error {
    error("UnknownJob", "Artifact job is absent")
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
