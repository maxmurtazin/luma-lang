//! M7 planning over the two publications captured by the native caller.
use crate::{parameters::validate_key, ParameterHost};
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Serialize)]
pub enum PlanarOperation {
    Union,
    Intersection,
    Difference,
}

impl PlanarOperation {
    pub fn provider_id(self) -> &'static str {
        match self {
            Self::Union => "living.brep.boolean.planar.union",
            Self::Intersection => "living.brep.boolean.planar.intersection",
            Self::Difference => "living.brep.boolean.planar.difference",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Operand {
    Left,
    Right,
}

#[derive(Debug)]
pub struct PlanarPlan {
    pub key: String,
    pub operation: PlanarOperation,
    pub operands: [Operand; 2],
}

pub struct PlanarHost {
    handles: ParameterHost,
    registry: Registry,
    plan: Mutex<Option<PlanarPlan>>,
}

const BODY: &str = "BodyRef";

impl PlanarHost {
    pub fn new() -> Result<Self, Error> {
        let body = Type::Opaque(BODY.into());
        let descriptors = [
            ("left_operand", Effect::ReadPinned),
            ("right_operand", Effect::ReadPinned),
            ("planar_union", Effect::BuildPlan),
            ("planar_intersection", Effect::BuildPlan),
            ("planar_difference", Effect::BuildPlan),
        ]
        .map(|(name, effect)| Descriptor {
            module: "brep".into(),
            name: name.into(),
            operation: format!("brep.{name}"),
            major: 1,
            parameters: if effect == Effect::ReadPinned {
                Vec::new()
            } else {
                vec![
                    Parameter::Exact(Type::Text),
                    Parameter::Exact(body.clone()),
                    Parameter::Exact(body.clone()),
                ]
            },
            result: ResultType::Exact(body.clone()),
            effect,
            modes: [Mode::Command].into(),
        });
        Ok(Self {
            handles: ParameterHost::new(Default::default())?,
            registry: Registry::new(
                SEALED_MODULES.iter().map(|name| (*name).into()),
                descriptors,
            )?,
            plan: Mutex::default(),
        })
    }

    pub fn finish(self) -> Result<PlanarPlan, Error> {
        self.plan
            .into_inner()
            .map_err(|_| unavailable())?
            .ok_or_else(|| Error::new("EmptyM7Plan", "Expected a planar Boolean operation"))
    }
}

impl Dispatch for PlanarHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        arguments: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        match (descriptor.name.as_str(), arguments) {
            ("left_operand", []) => self.handles.wrap_handle(BODY, Operand::Left),
            ("right_operand", []) => self.handles.wrap_handle(BODY, Operand::Right),
            (name, [Value::Text(key), a, b]) => {
                validate_key(key)?;
                let operation = match name {
                    "planar_union" => PlanarOperation::Union,
                    "planar_intersection" => PlanarOperation::Intersection,
                    "planar_difference" => PlanarOperation::Difference,
                    _ => return Err(Error::new("InvalidM7Operation", &descriptor.operation)),
                };
                let a: Operand = self.handles.unwrap_handle(a, BODY)?;
                let b: Operand = self.handles.unwrap_handle(b, BODY)?;
                if a == b {
                    return Err(Error::new(
                        "InvalidM7Operands",
                        "Expected both captured operands",
                    ));
                }
                let mut plan = self.plan.lock().map_err(|_| unavailable())?;
                if plan.is_some() {
                    return Err(Error::new(
                        "M7PlanLimit",
                        "Expected one Boolean over the captured pair",
                    ));
                }
                *plan = Some(PlanarPlan {
                    key: key.clone(),
                    operation,
                    operands: [a, b],
                });
                self.handles.wrap_handle(BODY, key)
            }
            _ => Err(Error::new("InvalidM7Operation", &descriptor.operation)),
        }
    }
}

fn unavailable() -> Error {
    Error::new("M7PlanUnavailable", "Cannot access the Boolean plan")
}
