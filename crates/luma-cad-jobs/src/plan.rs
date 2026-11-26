use crate::{digest, scope_id, stable_key, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_TASKS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePin {
    pub document: String,
    pub revision: u64,
    pub definition_digest: String,
}

impl SourcePin {
    pub(crate) fn validate(&self) -> Result<()> {
        if !scope_id(&self.document) || !digest(&self.definition_digest) {
            return Err(Error::Invalid("invalid source pin"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Policy {
    HistoricalPinned,
    CurrentIfUnchanged {
        slot: String,
        expected_generation: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FutureOutput {
    pub task: String,
}

/// A selected version of a provider operation from the admitted registry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    pub major: u32,
    pub provider_fingerprint: String,
    pub output_schema: String,
    pub inputs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub operation: String,
    pub major: u32,
    pub provider_fingerprint: String,
    pub input_digest: String,
    pub source: SourcePin,
    pub policy: Policy,
    pub inputs: BTreeMap<String, FutureOutput>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub key: String,
    pub task: String,
}

#[derive(Debug)]
pub struct Plan {
    pub(crate) tasks: Vec<(String, TaskSpec, String)>,
    pub(crate) intents: Vec<Intent>,
}

impl Plan {
    /// Check submitted closures and order dependency tasks before consumers.
    pub fn prepare(
        contracts: &BTreeMap<String, TaskContract>,
        tasks: &BTreeMap<String, TaskSpec>,
        intents: Vec<Intent>,
    ) -> Result<Self> {
        if tasks.len() > MAX_TASKS || intents.len() > MAX_TASKS {
            return Err(Error::Invalid("task or intent limit exceeded"));
        }
        let mut intent_keys = BTreeSet::new();
        let mut pending = BTreeSet::new();
        for intent in &intents {
            if !stable_key(&intent.key) || !intent_keys.insert(&intent.key) {
                return Err(Error::Invalid("invalid or duplicate intent key"));
            }
            pending.insert(intent.task.as_str());
        }
        let mut selected = BTreeMap::new();
        while let Some(key) = pending.pop_first() {
            if selected.contains_key(key) {
                continue;
            }
            let task = lookup(tasks, key)?;
            let contract = contract(contracts, task)?;
            task.source.validate()?;
            if !stable_key(key) || !digest(&task.input_digest) {
                return Err(Error::Invalid("invalid task key or input digest"));
            }
            if let Policy::CurrentIfUnchanged { slot, .. } = &task.policy {
                if !stable_key(slot) {
                    return Err(Error::Invalid("invalid publication slot"));
                }
            }
            if task.inputs.len() > MAX_TASKS || !task.inputs.keys().eq(contract.inputs.keys()) {
                return Err(Error::Invalid(
                    "task inputs do not match the provider contract",
                ));
            }
            for (slot, dependency) in &task.inputs {
                if !stable_key(slot) {
                    return Err(Error::Invalid("invalid provider input slot"));
                }
                let upstream = lookup(tasks, &dependency.task)?;
                let output = self::contract(contracts, upstream)?;
                if output.output_schema != contract.inputs[slot] {
                    return Err(Error::Invalid("future output schema does not match input"));
                }
                pending.insert(dependency.task.as_str());
            }
            selected.insert(key, (task, &contract.output_schema));
        }
        let mut finished = BTreeSet::new();
        let mut ordered = Vec::new();
        while !selected.is_empty() {
            let ready = selected
                .iter()
                .find(|(_, (task, _))| {
                    task.inputs
                        .values()
                        .all(|input| finished.contains(input.task.as_str()))
                })
                .map(|(key, _)| *key);
            let key = ready.ok_or(Error::Invalid("task dependency cycle"))?;
            let (task, output) = selected.remove(key).ok_or(Error::Corrupt)?;
            ordered.push((key.to_owned(), task.clone(), output.clone()));
            finished.insert(key);
        }
        Ok(Self {
            tasks: ordered,
            intents,
        })
    }
}

fn lookup<'a>(tasks: &'a BTreeMap<String, TaskSpec>, key: &str) -> Result<&'a TaskSpec> {
    tasks
        .get(key)
        .ok_or(Error::Invalid("task dependency is absent"))
}

fn contract<'a>(
    registry: &'a BTreeMap<String, TaskContract>,
    task: &TaskSpec,
) -> Result<&'a TaskContract> {
    let contract = registry
        .get(&task.operation)
        .ok_or(Error::Invalid("unsupported task operation"))?;
    if !stable_key(&task.operation)
        || task.major == 0
        || task.major != contract.major
        || task.provider_fingerprint != contract.provider_fingerprint
        || !digest(&task.provider_fingerprint)
        || !stable_key(&contract.output_schema)
    {
        return Err(Error::Invalid(
            "unknown operation version, provider or output schema",
        ));
    }
    Ok(contract)
}
