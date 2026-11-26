//! CAD-1 native job provider for an immutable Part-definition artifact.
use super::*;
use luma_cad_jobs as queue;

const TASK: &str = "part_definition";
const OPERATION: &str = "core.part.definition";
const SCHEMA: &str = "core.part.definition/1";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueueRequest {
    source: queue::SourcePin,
    intents: Vec<queue::Intent>,
}

impl QueueRequest {
    pub(super) fn prepare(
        pin: &SourcePin,
        intents: Vec<queue::Intent>,
    ) -> Result<Option<Self>, Error> {
        if intents.is_empty() {
            return Ok(None);
        }
        let request = Self {
            source: queue::SourcePin {
                document: pin.document_id.clone(),
                revision: pin.native_revision,
                definition_digest: normalized_hash(&pin.definition_hash),
            },
            intents,
        };
        request.plan()?;
        Ok(Some(request))
    }

    fn plan(&self) -> Result<queue::Plan, Error> {
        queue::Plan::prepare(&contracts(), &catalog(&self.source), self.intents.clone())
            .map_err(queue_error)
    }

    pub(super) fn admit(
        &self,
        tx: &mut rusqlite::Transaction<'_>,
        scope: &queue::Scope,
    ) -> Result<Vec<queue::Delivery>, Error> {
        queue::install_in(tx).map_err(queue_error)?;
        queue::admit(tx, scope, &self.plan()?).map_err(queue_error)
    }
}

pub(super) fn scope(project: &str, recipe: &str) -> queue::Scope {
    queue::Scope {
        project: project.into(),
        recipe: recipe.into(),
    }
}

fn contracts() -> BTreeMap<String, queue::TaskContract> {
    [(
        OPERATION.into(),
        queue::TaskContract {
            major: 1,
            provider_fingerprint: PROFILE.into(),
            output_schema: SCHEMA.into(),
            inputs: BTreeMap::new(),
        },
    )]
    .into()
}

pub(super) fn catalog(pin: &queue::SourcePin) -> BTreeMap<String, queue::TaskSpec> {
    [(
        TASK.into(),
        queue::TaskSpec {
            operation: OPERATION.into(),
            major: 1,
            provider_fingerprint: PROFILE.into(),
            input_digest: pin.definition_digest.clone(),
            source: pin.clone(),
            policy: queue::Policy::HistoricalPinned,
            inputs: BTreeMap::new(),
        },
    )]
    .into()
}

pub(super) fn normalized_hash(hash: &str) -> String {
    if hash.starts_with("sha256:") {
        hash.into()
    } else {
        format!("sha256:{hash}")
    }
}

pub(super) fn queue_error(problem: queue::Error) -> Error {
    let code = match &problem {
        queue::Error::Missing => "UnknownJob",
        queue::Error::Conflict => "Conflict",
        queue::Error::Corrupt => "CheckpointCorrupt",
        queue::Error::Invalid(_) => "InvalidJobPlan",
        queue::Error::Database(_) | queue::Error::Json(_) => "JobStore",
    };
    // Job SQL errors are implementation details and can contain paths.
    error(code, "The CAD job operation could not be completed")
}

fn transaction_error(err: rusqlite::Error) -> Error {
    queue_error(err.into())
}

impl ProjectStore {
    pub(super) fn capture_luma_jobs(
        &self,
        scope: &queue::Scope,
        keys: &[String],
    ) -> Result<BTreeMap<String, queue::JobReceipt>, Error> {
        if keys.len() > 128 {
            return Err(error("ResourceLimit", "Too many admitted job inputs"));
        }
        if keys.is_empty() {
            return Ok(BTreeMap::new());
        }
        let tx = self
            .db
            .luma_connection()
            .unchecked_transaction()
            .map_err(transaction_error)?;
        let mut captured = BTreeMap::new();
        for key in keys {
            let receipt = queue::capture(&tx, scope, key)
                .map_err(queue_error)?
                .ok_or_else(|| error("UnknownJob", "Selected job intent is absent"))?;
            captured.insert(key.clone(), receipt);
        }
        tx.commit().map_err(transaction_error)?;
        Ok(captured)
    }

    pub fn luma_job(&self, document: &str, key: &str) -> Result<queue::JobReceipt, Error> {
        uuid(document)?;
        let registry = self.registry()?;
        let recipe = registry
            .luma_recipes
            .get(document)
            .ok_or_else(|| error("MissingAcceptedSource", "Part has no accepted recipe"))?;
        self.capture_luma_jobs(
            &scope(&registry.project_id, &recipe.instance),
            &[key.into()],
        )?
        .remove(key)
        .ok_or_else(|| error("UnknownJob", "Selected job intent is absent"))
    }

    /// Claim and publish one definition artifact. The saved canonical bytes
    /// come from an authenticated checkpoint and are pinned by native hash.
    pub fn run_luma_job(&self) -> Result<Option<queue::JobReceipt>, Error> {
        let connection = self.db.luma_connection();
        let mut tx = job_write(connection)?;
        queue::install_in(&tx).map_err(queue_error)?;
        let claimed = queue::claim(&mut tx, PROFILE).map_err(queue_error)?;
        tx.commit().map_err(transaction_error)?;
        let Some(claimed) = claimed else {
            return Ok(None);
        };

        let artifact = (|| {
            let source = &claimed.task.source;
            if claimed.task != catalog(source)[TASK] || !claimed.inputs.is_empty() {
                return Err(error("InvalidJobPlan", "Unknown Core job contract"));
            }
            let registry = self.registry()?;
            let canonical = definition_at(&registry, source)?;
            Ok(queue::Artifact {
                schema: SCHEMA.into(),
                manifest_digest: format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())),
            })
        })();

        let mut tx = job_write(connection)?;
        match artifact {
            Ok(artifact) => {
                queue::complete(&mut tx, claimed.attempt, &artifact).map_err(queue_error)?;
                queue::publish(&mut tx, claimed.attempt).map_err(queue_error)?;
            }
            Err(problem) => {
                queue::fail(&mut tx, claimed.attempt, problem.code.as_ref()).map_err(queue_error)?
            }
        }
        let receipt = queue::receipt(&tx, claimed.attempt.job).map_err(queue_error)?;
        tx.commit().map_err(transaction_error)?;
        Ok(Some(receipt))
    }

    pub fn cancel_luma_job(&self, job: i64) -> Result<queue::JobReceipt, Error> {
        let mut tx = job_write(self.db.luma_connection())?;
        let job = queue::JobId(job);
        queue::cancel(&mut tx, job).map_err(queue_error)?;
        let receipt = queue::receipt(&tx, job).map_err(queue_error)?;
        tx.commit().map_err(transaction_error)?;
        Ok(receipt)
    }

    pub fn retry_luma_job(&self, job: i64, attempt: i64) -> Result<queue::JobReceipt, Error> {
        let tx = job_write(self.db.luma_connection())?;
        let job = queue::JobId(job);
        queue::retry(
            &tx,
            queue::Attempt {
                job,
                number: attempt,
            },
        )
        .map_err(queue_error)?;
        let receipt = queue::receipt(&tx, job).map_err(queue_error)?;
        tx.commit().map_err(transaction_error)?;
        Ok(receipt)
    }

    /// Recover a published artifact using the immutable ID from an accepted receipt.
    pub fn luma_job_definition(&self, job: i64) -> Result<Value, Error> {
        let receipt =
            queue::receipt(self.db.luma_connection(), queue::JobId(job)).map_err(queue_error)?;
        let artifact = receipt
            .artifact
            .filter(|artifact| {
                receipt.state == queue::State::Published && artifact.schema == SCHEMA
            })
            .ok_or_else(|| error("JobPending", "No published definition is available"))?;
        let registry = self.registry()?;
        let bytes = definition_at(&registry, &receipt.source)?;
        if artifact.manifest_digest != format!("sha256:{:x}", Sha256::digest(bytes.as_bytes())) {
            return Err(error(
                "CheckpointCorrupt",
                "Published definition digest differs",
            ));
        }
        // Return the exact text whose UTF-8 bytes the manifest identifies.
        Ok(json!({"manifest": artifact, "canonical_definition": bytes}))
    }
}

fn definition_at<'a>(registry: &'a Registry, source: &queue::SourcePin) -> Result<&'a str, Error> {
    registry
        .parts
        .get(&source.document)
        .and_then(|part| {
            part.snapshots
                .iter()
                .find_map(|(revision, hash, definition)| {
                    (*revision == source.revision
                        && normalized_hash(hash) == source.definition_digest)
                        .then_some(definition.as_str())
                })
        })
        .ok_or_else(|| {
            error(
                "MissingPinnedSource",
                "The accepted definition pin is unavailable",
            )
        })
}

fn job_write(connection: &rusqlite::Connection) -> Result<rusqlite::Transaction<'_>, Error> {
    rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(transaction_error)
}
