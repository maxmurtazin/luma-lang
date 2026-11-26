use crate::{stable_key, Intent, JobReceipt, Plan, State, TaskContract, TaskSpec};
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex};

const TASK: &str = "TaskSpecRef";
const JOB: &str = "JobRef";
const INTENT: &str = "JobIntentRef";
const OUTCOME: &str = "OutcomeRef";

/// One preparation session. The authority supplies captured job receipts and
/// the provider's admitted task graph. The session never opens the outbox.
pub struct JobsHost<'a> {
    fallback: &'a dyn Dispatch,
    registry: Registry,
    tasks: &'a BTreeMap<String, TaskSpec>,
    captured: &'a BTreeMap<String, JobReceipt>,
    intents: Mutex<BTreeMap<String, String>>,
}

impl<'a> JobsHost<'a> {
    pub fn new(
        fallback: &'a dyn Dispatch,
        tasks: &'a BTreeMap<String, TaskSpec>,
        captured: &'a BTreeMap<String, JobReceipt>,
    ) -> Result<Self, Error> {
        let registry = fallback.registry().with_descriptors([
            descriptor(
                "submit",
                vec![Parameter::Exact(Type::Text), exact(TASK)],
                INTENT,
                Effect::DeclareTask,
            ),
            descriptor("status", vec![exact(JOB)], OUTCOME, Effect::ReadPinned),
            descriptor("result", vec![exact(JOB)], OUTCOME, Effect::ReadPinned),
        ])?;
        Ok(Self {
            fallback,
            registry,
            tasks,
            captured,
            intents: Mutex::new(BTreeMap::new()),
        })
    }

    /// Call after successful Luma execution. Only the model authority may
    /// pass the returned plan to the SQLite transaction.
    pub fn finish(self, contracts: &BTreeMap<String, TaskContract>) -> crate::Result<Plan> {
        let intents = self
            .intents
            .into_inner()
            .map_err(|_| crate::Error::Corrupt)?
            .into_iter()
            .map(|(key, task)| Intent { key, task })
            .collect();
        Plan::prepare(contracts, self.tasks, intents)
    }
}

impl Dispatch for JobsHost<'_> {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        site: &CallSite,
        arguments: &[Value],
    ) -> Result<Value, Error> {
        match (descriptor.operation.as_str(), arguments) {
            ("jobs.submit", [Value::Text(key), task]) => {
                let task = handle(task, TASK)?;
                if !self.tasks.contains_key(task) {
                    return Err(Error::new("UnadmittedTask", task));
                }
                let mut intents = self
                    .intents
                    .lock()
                    .map_err(|_| Error::new("InvalidJobSession", "job plan lock failed"))?;
                if !stable_key(key) || intents.contains_key(key) {
                    return Err(Error::new("InvalidJobIntentKey", key));
                }
                if intents.len() >= crate::plan::MAX_TASKS {
                    return Err(Error::new("JobIntentLimit", "too many job intents"));
                }
                intents.insert(key.clone(), task.into());
                Ok(opaque(INTENT, json!(key)))
            }
            (operation @ ("jobs.status" | "jobs.result"), [job]) => {
                let reference = handle(job, JOB)?;
                let receipt = self
                    .captured
                    .get(reference)
                    .ok_or_else(|| Error::new("UnadmittedJob", reference))?;
                let payload = if operation == "jobs.status" {
                    json!({"kind": "status", "state": receipt.state, "attempt": receipt.attempt})
                } else {
                    match receipt.state {
                        State::Published => {
                            json!({"kind": "artifact", "artifact": receipt.artifact.as_ref()
                            .ok_or_else(|| Error::new("InvalidJobSnapshot", "published artifact is absent"))?})
                        }
                        State::Failed => json!({"kind": "failed", "reason": receipt.failure}),
                        State::Cancelled => json!({"kind": "cancelled"}),
                        State::Superseded => json!({"kind": "superseded"}),
                        _ => json!({"kind": "pending", "state": receipt.state}),
                    }
                };
                Ok(opaque(OUTCOME, payload))
            }
            _ => self.fallback.call(descriptor, mode, site, arguments),
        }
    }
}

fn descriptor(name: &str, parameters: Vec<Parameter>, result: &str, effect: Effect) -> Descriptor {
    Descriptor {
        module: "jobs".into(),
        name: name.into(),
        operation: format!("jobs.{name}"),
        major: 1,
        parameters,
        result: ResultType::Exact(Type::Opaque(result.into())),
        effect,
        modes: [Mode::Recipe, Mode::Command].into(),
    }
}

fn exact(type_name: &str) -> Parameter {
    Parameter::Exact(Type::Opaque(type_name.into()))
}

fn opaque(type_name: &str, payload: serde_json::Value) -> Value {
    Value::Opaque {
        type_name: type_name.into(),
        payload,
    }
}

fn handle<'a>(value: &'a Value, expected: &str) -> Result<&'a str, Error> {
    if let Value::Opaque { type_name, payload } = value {
        if type_name == expected {
            if let Some(reference) = payload.as_str() {
                return Ok(reference);
            }
        }
    }
    Err(Error::new(
        "InvalidJobHandle",
        format!("expected {expected}"),
    ))
}
