//! Typed commands for a study or an archived observation captured by the native caller.
use crate::ParameterHost;
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde_json::Value as Observation;
use std::sync::atomic::{AtomicBool, Ordering};

const STUDY: &str = "StudyRef";
const OPTIONS: &str = "SolveOptionsRef";
const TASK: &str = "TaskSpecRef<SimulationResult>";
const RESULT: &str = "SimulationResultRef";
const QUANTITY: &str = "SimulationQuantity";
const SELECTOR: &str = "SimulationSelector";
const OBSERVATION: &str = "SimulationObservation";

pub struct SimulationHost<'a> {
    handles: ParameterHost,
    registry: Registry,
    observation: Option<&'a Observation>,
    used: AtomicBool,
}

impl<'a> SimulationHost<'a> {
    pub fn for_study() -> Result<Self, Error> {
        Self::new(
            None,
            [
                ("pinned_study", &[], STUDY, Effect::ReadPinned),
                ("solve_options", &[], OPTIONS, Effect::Pure),
                ("solve", &[STUDY, OPTIONS], TASK, Effect::DeclareTask),
            ],
        )
    }

    pub fn for_result(observation: &'a Observation) -> Result<Self, Error> {
        Self::new(
            Some(observation),
            [
                ("pinned_result", &[], RESULT, Effect::ReadPinned),
                ("quantity", &[], QUANTITY, Effect::Pure),
                ("selector", &[], SELECTOR, Effect::Pure),
                (
                    "query",
                    &[RESULT, QUANTITY, SELECTOR],
                    OBSERVATION,
                    Effect::ReadPinned,
                ),
            ],
        )
    }

    fn new<const N: usize>(
        observation: Option<&'a Observation>,
        specs: [(&str, &[&str], &str, Effect); N],
    ) -> Result<Self, Error> {
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            specs
                .into_iter()
                .map(|(name, args, result, effect)| Descriptor {
                    module: "sim".into(),
                    name: name.into(),
                    operation: format!("sim.{name}"),
                    major: 1,
                    parameters: args
                        .iter()
                        .map(|arg| Parameter::Exact(Type::Opaque((*arg).into())))
                        .collect(),
                    result: ResultType::Exact(Type::Opaque(result.into())),
                    effect,
                    modes: [Mode::Command].into(),
                }),
        )?;
        Ok(Self {
            handles: ParameterHost::new(Default::default())?,
            registry,
            observation,
            used: AtomicBool::new(false),
        })
    }

    pub fn finish(self) -> Result<(), Error> {
        if !self.used.into_inner() {
            return Err(match self.observation {
                Some(_) => Error::new("MissingSimulationQuery", "Expected one sim.query call"),
                None => Error::new(
                    "MissingSimulationTask",
                    "Expected one sim.solve declaration",
                ),
            });
        }
        Ok(())
    }

    fn use_once(&self, operation: &str) -> Result<(), Error> {
        if self.used.swap(true, Ordering::Relaxed) {
            return Err(Error::new(
                "SimulationCommandLimit",
                format!("Expected one {operation} call"),
            ));
        }
        Ok(())
    }
}

impl Dispatch for SimulationHost<'_> {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        match (descriptor.name.as_str(), args) {
            ("pinned_study", []) => self.handles.wrap_handle(STUDY, ()),
            ("solve_options", []) => self.handles.wrap_handle(OPTIONS, ()),
            ("solve", [study, options]) => {
                self.handles.unwrap_handle::<()>(study, STUDY)?;
                self.handles.unwrap_handle::<()>(options, OPTIONS)?;
                self.use_once(&descriptor.operation)?;
                self.handles.wrap_handle(TASK, ())
            }
            ("pinned_result", []) => self.handles.wrap_handle(RESULT, ()),
            ("quantity", []) => self.handles.wrap_handle(QUANTITY, ()),
            ("selector", []) => self.handles.wrap_handle(SELECTOR, ()),
            ("query", [result, quantity, selector]) => {
                self.handles.unwrap_handle::<()>(result, RESULT)?;
                self.handles.unwrap_handle::<()>(quantity, QUANTITY)?;
                self.handles.unwrap_handle::<()>(selector, SELECTOR)?;
                let observation = self.observation.ok_or_else(|| {
                    Error::new(
                        "MissingSimulationResult",
                        "Expected an archived observation",
                    )
                })?;
                self.use_once(&descriptor.operation)?;
                self.handles.wrap_handle(OBSERVATION, observation)
            }
            _ => Err(Error::new("SimulationInputType", &descriptor.operation)),
        }
    }
}
