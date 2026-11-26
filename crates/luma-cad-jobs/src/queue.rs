use crate::{digest, scope_id, Error, Plan, Policy, Result, SourcePin, TaskSpec};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub struct Scope {
    pub project: String,
    pub recipe: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JobId(pub i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attempt {
    pub job: JobId,
    pub number: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delivery {
    pub key: String,
    pub generation: i64,
    pub job: JobId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Admitted,
    Running,
    Computed,
    Published,
    Superseded,
    Failed,
    Cancelled,
}

impl State {
    fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "admitted" => Self::Admitted,
            "running" => Self::Running,
            "computed" => Self::Computed,
            "published" => Self::Published,
            "superseded" => Self::Superseded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return Err(Error::Corrupt),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Artifact {
    pub schema: String,
    pub manifest_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claimed {
    pub attempt: Attempt,
    pub task: TaskSpec,
    pub inputs: BTreeMap<String, Artifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Current {
    pub job: JobId,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Publication {
    Historical,
    Current(Current),
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    Cancelled,
    AlreadyPublished,
    AlreadyTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobReceipt {
    pub job: JobId,
    pub generation: i64,
    pub attempt: i64,
    pub state: State,
    pub source: SourcePin,
    pub artifact: Option<Artifact>,
    pub publication: Option<Publication>,
    pub failure: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TaskRecord {
    task: TaskSpec,
    output_schema: String,
    inputs: BTreeMap<String, JobId>,
}

struct StoredJob {
    project: String,
    attempt: i64,
    state: State,
    record: TaskRecord,
}

/// Install the outbox tables in the same database as the caller's model authority.
pub fn install(connection: &mut Connection) -> Result<()> {
    let tx = connection.transaction()?;
    tx.execute_batch(include_str!("queue.sql"))?;
    let version: i64 = tx.query_row("SELECT version FROM lcad_schema WHERE id=1", [], |row| {
        row.get(0)
    })?;
    if version != 1 {
        return Err(Error::Invalid("unsupported outbox schema version"));
    }
    tx.commit()?;
    Ok(())
}

/// Write jobs and intent mappings to the caller's model transaction.
/// A failed admission rolls back its own savepoint.
pub fn admit(tx: &mut Transaction<'_>, scope: &Scope, plan: &Plan) -> Result<Vec<Delivery>> {
    if !scope_id(&scope.project) || !scope_id(&scope.recipe) {
        return Err(Error::Invalid("invalid project or recipe scope"));
    }
    let name = serde_json::to_string(scope)?;
    let checkpoint = tx.savepoint()?;
    let mut resolved = BTreeMap::new();
    for (key, task, output_schema) in &plan.tasks {
        let inputs = task
            .inputs
            .iter()
            .map(|(slot, dep)| {
                Ok((
                    slot.clone(),
                    *resolved.get(&dep.task).ok_or(Error::Corrupt)?,
                ))
            })
            .collect::<Result<_>>()?;
        let record = TaskRecord {
            task: task.clone(),
            output_schema: output_schema.clone(),
            inputs,
        };
        let payload = serde_json::to_string(&record)?;
        let latest = checkpoint
            .query_row(
                "SELECT id, generation, spec FROM lcad_jobs WHERE scope=?1 AND task_key=?2
             ORDER BY generation DESC LIMIT 1",
                params![name, key],
                |row| {
                    Ok((
                        JobId(row.get(0)?),
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let job = match latest {
            Some((id, _, prior)) if prior == payload => id,
            latest => {
                let generation = next(latest.map_or(0, |(_, generation, _)| generation))?;
                checkpoint.execute(
                    "INSERT INTO lcad_jobs(scope, project, task_key, generation, provider, spec)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        name,
                        scope.project,
                        key,
                        generation,
                        task.provider_fingerprint,
                        payload
                    ],
                )?;
                let job = JobId(checkpoint.last_insert_rowid());
                for (slot, dependency) in &record.inputs {
                    checkpoint.execute(
                        "INSERT INTO lcad_dependencies(job, slot, dependency) VALUES (?1, ?2, ?3)",
                        params![job.0, slot, dependency.0],
                    )?;
                }
                job
            }
        };
        resolved.insert(key, job);
    }
    let mut deliveries = Vec::with_capacity(plan.intents.len());
    for entry in &plan.intents {
        let job = *resolved.get(&entry.task).ok_or(Error::Corrupt)?;
        let latest = intent(&checkpoint, scope, &entry.key)?;
        let generation = match latest {
            Some(ref prior) if prior.job == job => prior.generation,
            latest => {
                let generation = next(latest.map_or(0, |prior| prior.generation))?;
                checkpoint.execute(
                    "INSERT INTO lcad_intents(scope, intent_key, generation, job) VALUES (?1, ?2, ?3, ?4)",
                    params![name, entry.key, generation, job.0],
                )?;
                generation
            }
        };
        deliveries.push(Delivery {
            key: entry.key.clone(),
            generation,
            job,
        });
    }
    checkpoint.commit()?;
    Ok(deliveries)
}

pub fn intent(connection: &Connection, scope: &Scope, key: &str) -> Result<Option<Delivery>> {
    Ok(connection.query_row(
        "SELECT generation, job FROM lcad_intents WHERE scope=?1 AND intent_key=?2 ORDER BY generation DESC LIMIT 1",
        params![serde_json::to_string(scope)?, key],
        |row| Ok(Delivery { key: key.into(), generation: row.get(0)?, job: JobId(row.get(1)?) }),
    ).optional()?)
}

/// Select an accepted task only after every typed future output has been published.
pub fn claim(tx: &mut Transaction<'_>, provider: &str) -> Result<Option<Claimed>> {
    if !digest(provider) {
        return Err(Error::Invalid("invalid provider fingerprint"));
    }
    let checkpoint = tx.savepoint()?;
    let picked = checkpoint
        .query_row(
            "UPDATE lcad_jobs SET state='running', attempt=attempt+1
         WHERE id=(SELECT job.id FROM lcad_jobs job
           WHERE job.state='admitted' AND job.provider=?1
             AND NOT EXISTS (SELECT 1 FROM lcad_dependencies edge
                 JOIN lcad_jobs input ON input.id=edge.dependency
                 WHERE edge.job=job.id AND input.state!='published')
           ORDER BY job.id LIMIT 1)
         RETURNING id, attempt, spec",
            [provider],
            |row| {
                Ok((
                    JobId(row.get(0)?),
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((job, number, spec)) = picked else {
        return Ok(None);
    };
    let record: TaskRecord = serde_json::from_str(&spec)?;
    let mut inputs = BTreeMap::new();
    for (slot, upstream) in &record.inputs {
        let input = receipt(&checkpoint, *upstream)?;
        if input.state != State::Published {
            return Err(Error::Corrupt);
        }
        inputs.insert(slot.clone(), input.artifact.ok_or(Error::Corrupt)?);
    }
    checkpoint.execute(
        "INSERT INTO lcad_attempts(job, number, state) VALUES (?1, ?2, 'running')",
        params![job.0, number],
    )?;
    checkpoint.commit()?;
    Ok(Some(Claimed {
        attempt: Attempt { job, number },
        task: record.task,
        inputs,
    }))
}

/// Record a provider-verified artifact manifest. Publication is a separate step.
pub fn complete(tx: &mut Transaction<'_>, attempt: Attempt, artifact: &Artifact) -> Result<()> {
    let checkpoint = tx.savepoint()?;
    let job = check(&checkpoint, attempt, State::Running)?;
    if artifact.schema != job.record.output_schema || !digest(&artifact.manifest_digest) {
        return Err(Error::Invalid(
            "artifact does not match the selected output schema",
        ));
    }
    checkpoint.execute(
        "UPDATE lcad_attempts SET artifact=?3 WHERE job=?1 AND number=?2",
        params![
            attempt.job.0,
            attempt.number,
            serde_json::to_string(artifact)?
        ],
    )?;
    transition(&checkpoint, attempt, "computed")?;
    checkpoint.commit()?;
    Ok(())
}

pub fn fail(tx: &mut Transaction<'_>, attempt: Attempt, outcome: &str) -> Result<()> {
    if outcome.trim().is_empty() || outcome.len() > 128 || outcome.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid failure outcome code"));
    }
    let checkpoint = tx.savepoint()?;
    check(&checkpoint, attempt, State::Running)?;
    checkpoint.execute(
        "UPDATE lcad_attempts SET failure=?3 WHERE job=?1 AND number=?2",
        params![attempt.job.0, attempt.number, outcome],
    )?;
    transition(&checkpoint, attempt, "failed")?;
    checkpoint.commit()?;
    Ok(())
}

/// Request an explicit fresh attempt after failure or cancellation.
pub fn retry(tx: &Transaction<'_>, last: Attempt) -> Result<()> {
    let rows = tx.execute(
        "UPDATE lcad_jobs SET state='admitted' WHERE id=?1 AND attempt=?2 AND state IN ('failed','cancelled')",
        params![last.job.0, last.number],
    )?;
    if rows != 1 {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub fn cancel(tx: &mut Transaction<'_>, job: JobId) -> Result<CancelOutcome> {
    let checkpoint = tx.savepoint()?;
    let prior = stored(&checkpoint, job)?;
    match prior.state {
        State::Published => return Ok(CancelOutcome::AlreadyPublished),
        State::Superseded | State::Failed => return Ok(CancelOutcome::AlreadyTerminal),
        State::Cancelled => return Ok(CancelOutcome::Cancelled),
        State::Admitted => {
            checkpoint.execute(
                "UPDATE lcad_jobs SET state='cancelled' WHERE id=?1",
                [job.0],
            )?;
        }
        State::Running | State::Computed => {
            transition(
                &checkpoint,
                Attempt {
                    job,
                    number: prior.attempt,
                },
                "cancelled",
            )?;
        }
    }
    checkpoint.commit()?;
    Ok(CancelOutcome::Cancelled)
}

/// Update the authority's visible source guard in the model transaction.
pub fn set_head(tx: &Transaction<'_>, project: &str, source: &SourcePin) -> Result<()> {
    source.validate()?;
    if !scope_id(project) {
        return Err(Error::Invalid("invalid project scope"));
    }
    if let Some(head) = head(tx, project, &source.document)? {
        if head == *source {
            return Ok(());
        }
        if head.revision >= source.revision {
            return Err(Error::Conflict);
        }
    }
    tx.execute(
        "INSERT INTO lcad_source_heads(project, document, pin) VALUES (?1, ?2, ?3)
         ON CONFLICT(project, document) DO UPDATE SET pin=excluded.pin",
        params![project, source.document, serde_json::to_string(source)?],
    )?;
    Ok(())
}

pub fn publish(tx: &mut Transaction<'_>, attempt: Attempt) -> Result<Publication> {
    let checkpoint = tx.savepoint()?;
    let job = stored(&checkpoint, attempt.job)?;
    if job.attempt != attempt.number {
        return Err(Error::Conflict);
    }
    if matches!(job.state, State::Published | State::Superseded) {
        return receipt(&checkpoint, attempt.job)?
            .publication
            .ok_or(Error::Corrupt);
    }
    if job.state != State::Computed || receipt(&checkpoint, attempt.job)?.artifact.is_none() {
        return Err(Error::Conflict);
    }

    let source = &job.record.task.source;
    let result = match &job.record.task.policy {
        Policy::HistoricalPinned => Publication::Historical,
        Policy::CurrentIfUnchanged {
            slot,
            expected_generation,
        } => {
            let previous = slot_generation(&checkpoint, &job.project, slot)?;
            if previous != *expected_generation
                || head(&checkpoint, &job.project, &source.document)?.as_ref() != Some(source)
            {
                Publication::Superseded
            } else {
                let generation = previous.checked_add(1).ok_or(Error::Conflict)?;
                checkpoint.execute(
                    "INSERT INTO lcad_slots(project, slot, generation, job) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(project, slot) DO UPDATE SET generation=excluded.generation, job=excluded.job",
                    params![job.project, slot, generation.to_string(), attempt.job.0],
                )?;
                Publication::Current(Current {
                    job: attempt.job,
                    generation,
                })
            }
        }
    };
    checkpoint.execute(
        "UPDATE lcad_attempts SET publication=?3 WHERE job=?1 AND number=?2",
        params![
            attempt.job.0,
            attempt.number,
            serde_json::to_string(&result)?
        ],
    )?;
    transition(
        &checkpoint,
        attempt,
        if result == Publication::Superseded {
            "superseded"
        } else {
            "published"
        },
    )?;
    checkpoint.commit()?;
    Ok(result)
}

pub fn slot_generation(connection: &Connection, project: &str, slot: &str) -> Result<u64> {
    Ok(slot_binding(connection, project, slot)?.map_or(0, |current| current.generation))
}

/// A slot pointing to an older source is historical until another publish.
pub fn current(connection: &Connection, project: &str, slot: &str) -> Result<Option<Current>> {
    let row = connection
        .query_row(
            "SELECT slot.job, slot.generation, job.spec, head.pin
         FROM lcad_slots slot JOIN lcad_jobs job ON job.id=slot.job
         LEFT JOIN lcad_source_heads head ON head.project=slot.project
           AND head.document=json_extract(job.spec, '$.task.source.document')
         WHERE slot.project=?1 AND slot.slot=?2",
            params![project, slot],
            |row| {
                Ok((
                    JobId(row.get(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((job, generation, spec, Some(pin))) = row else {
        return Ok(None);
    };
    let record: TaskRecord = serde_json::from_str(&spec)?;
    let current: SourcePin = serde_json::from_str(&pin)?;
    if current != record.task.source {
        return Ok(None);
    }
    Ok(Some(Current {
        job,
        generation: generation.parse().map_err(|_| Error::Corrupt)?,
    }))
}

pub fn receipt(connection: &Connection, id: JobId) -> Result<JobReceipt> {
    read_receipt(connection, id, None)
}

pub fn attempt_receipt(connection: &Connection, attempt: Attempt) -> Result<JobReceipt> {
    read_receipt(connection, attempt.job, Some(attempt.number))
}

type ReceiptRow = (
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn read_receipt(connection: &Connection, id: JobId, number: Option<i64>) -> Result<JobReceipt> {
    let row: ReceiptRow = connection
        .query_row(
            "SELECT job.generation, coalesce(?2, job.attempt), job.state, job.spec,
                attempt.state, attempt.artifact, attempt.publication, attempt.failure
         FROM lcad_jobs job LEFT JOIN lcad_attempts attempt
           ON attempt.job=job.id AND attempt.number=coalesce(?2, job.attempt)
         WHERE job.id=?1",
            params![id.0, number],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::Missing)?;
    let (generation, attempt, job_state, spec, attempt_state, artifact, publication, failure) = row;
    let record: TaskRecord = serde_json::from_str(&spec)?;
    let (state, details) = match (number, attempt_state.as_deref()) {
        (Some(_), Some(state)) => (State::parse(state)?, true),
        (Some(_), None) => return Err(Error::Missing),
        (None, recorded) => (State::parse(&job_state)?, recorded == Some(&job_state)),
    };
    Ok(JobReceipt {
        job: id,
        generation,
        attempt,
        state,
        source: record.task.source,
        artifact: if details {
            artifact.as_deref().map(serde_json::from_str).transpose()?
        } else {
            None
        },
        publication: if details {
            publication
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
        } else {
            None
        },
        failure: failure.filter(|_| details),
    })
}

fn next(number: i64) -> Result<i64> {
    number.checked_add(1).ok_or(Error::Conflict)
}

fn stored(connection: &Connection, id: JobId) -> Result<StoredJob> {
    let (project, attempt, state, payload) = connection
        .query_row(
            "SELECT project, attempt, state, spec FROM lcad_jobs WHERE id=?1",
            [id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(Error::Missing)?;
    Ok(StoredJob {
        project,
        attempt,
        state: State::parse(&state)?,
        record: serde_json::from_str(&payload)?,
    })
}

fn check(connection: &Connection, attempt: Attempt, state: State) -> Result<StoredJob> {
    let job = stored(connection, attempt.job)?;
    if job.attempt != attempt.number || job.state != state {
        return Err(Error::Conflict);
    }
    Ok(job)
}

fn transition(connection: &Connection, attempt: Attempt, state: &str) -> Result<()> {
    let parameters = params![attempt.job.0, attempt.number, state];
    if connection.execute(
        "UPDATE lcad_jobs SET state=?3 WHERE id=?1 AND attempt=?2",
        parameters,
    )? != 1
        || connection.execute(
            "UPDATE lcad_attempts SET state=?3 WHERE job=?1 AND number=?2",
            parameters,
        )? != 1
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn head(connection: &Connection, project: &str, document: &str) -> Result<Option<SourcePin>> {
    let pin: Option<String> = connection
        .query_row(
            "SELECT pin FROM lcad_source_heads WHERE project=?1 AND document=?2",
            params![project, document],
            |row| row.get(0),
        )
        .optional()?;
    Ok(pin.as_deref().map(serde_json::from_str).transpose()?)
}

fn slot_binding(connection: &Connection, project: &str, slot: &str) -> Result<Option<Current>> {
    let row = connection
        .query_row(
            "SELECT job, generation FROM lcad_slots WHERE project=?1 AND slot=?2",
            params![project, slot],
            |row| Ok((JobId(row.get(0)?), row.get::<_, String>(1)?)),
        )
        .optional()?;
    row.map(|(job, revision)| {
        Ok(Current {
            job,
            generation: revision.parse().map_err(|_| Error::Corrupt)?,
        })
    })
    .transpose()
}
