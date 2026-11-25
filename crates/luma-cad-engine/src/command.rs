use crate::{Kind, ParameterSnapshot};
use luma_cad_contract::{UnitsHost, SEALED_MODULES};
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use std::sync::Mutex;

const PART: &str = "AdmittedPartRef";
const PARAMETER: &str = "AdmittedLengthRef";

#[derive(Clone, Copy)]
enum Operation {
    Part,
    Parameter,
    Value,
    Set,
    Reset,
}
const OPERATIONS: &[(&str, &str, Operation)] = &[
    ("project", "part", Operation::Part),
    ("project", "parameter", Operation::Parameter),
    ("param", "value", Operation::Value),
    ("param", "set", Operation::Set),
    ("param", "reset", Operation::Reset),
];

impl Operation {
    fn descriptor(self, module: &str, name: &str) -> Descriptor {
        let part = Type::Opaque(PART.into());
        let parameter = Type::Opaque(PARAMETER.into());
        let length = Kind::Length.value_type();
        let (inputs, result, effect) = match self {
            Self::Part => (vec![Type::Text], part, Effect::ReadPinned),
            Self::Parameter => (vec![part, Type::Text], parameter, Effect::ReadPinned),
            Self::Value => (vec![parameter], length, Effect::ReadPinned),
            Self::Set => (vec![parameter, length], Type::Unit, Effect::BuildPlan),
            Self::Reset => (vec![parameter], Type::Unit, Effect::BuildPlan),
        };
        Descriptor {
            module: module.into(),
            name: name.into(),
            operation: format!("{module}.{name}"),
            major: 1,
            parameters: inputs.into_iter().map(Parameter::Exact).collect(),
            result: ResultType::Exact(result),
            effect,
            modes: [Mode::Command].into(),
        }
    }
}

/// A command receives one admitted Part and a private override overlay.
pub struct CommandHost {
    part: String,
    scope: u64,
    units: UnitsHost,
    registry: Registry,
    overlay: Mutex<ParameterSnapshot>,
}

impl CommandHost {
    pub fn new(part: String, snapshot: ParameterSnapshot) -> Result<Self, Error> {
        let units = UnitsHost::new();
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            units.registry().descriptors().cloned().chain(
                OPERATIONS
                    .iter()
                    .map(|(module, name, op)| op.descriptor(module, name)),
            ),
        )?;
        Ok(Self {
            part,
            scope: crate::recipe::next_scope()?,
            units,
            registry,
            overlay: Mutex::new(snapshot),
        })
    }

    pub fn finish(self) -> Result<ParameterSnapshot, Error> {
        self.overlay.into_inner().map_err(|_| unavailable())
    }

    fn handle(&self, kind: &str, key: &str) -> Value {
        Value::Opaque {
            type_name: kind.into(),
            payload: serde_json::json!([self.scope, key]),
        }
    }

    fn key<'a>(&self, reference: &'a Value) -> Result<&'a str, Error> {
        if let Value::Opaque { payload, .. } = reference {
            if payload[0].as_u64() == Some(self.scope) {
                if let Some(key) = payload[1].as_str() {
                    return Ok(key);
                }
            }
        }
        Err(Error::new(
            "ForeignCommandReference",
            "reference must belong to the admitted Part",
        ))
    }
}

impl Dispatch for CommandHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        site: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if descriptor.module == "units" {
            return self.units.call(descriptor, mode, site, args);
        }
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new(
                "HostRegistryMismatch",
                "unknown command descriptor",
            ));
        }
        descriptor.check_mode(mode)?;
        let types = args
            .iter()
            .map(|arg| {
                arg.value_type().ok_or_else(|| {
                    Error::new("HostTypeMismatch", "expected a finite command argument")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        descriptor.specialize(&types)?;
        let operation = OPERATIONS
            .iter()
            .find_map(|(module, name, op)| {
                (*module == descriptor.module && *name == descriptor.name).then_some(*op)
            })
            .ok_or_else(|| Error::new("UnsupportedCapability", &descriptor.operation))?;
        let mut overlay = self.overlay.lock().map_err(|_| unavailable())?;
        match (operation, args) {
            (Operation::Part, [Value::Text(part)]) => {
                if part != &self.part {
                    return Err(Error::new(
                        "UnadmittedPart",
                        "command must name the admitted Part",
                    ));
                }
                Ok(self.handle(PART, part))
            }
            (Operation::Parameter, [part, Value::Text(key)]) => {
                self.key(part)?;
                let parameter = overlay
                    .entries()
                    .get(key)
                    .ok_or_else(|| Error::new("UnknownParameter", key))?;
                if parameter.declaration.default.kind() != Kind::Length {
                    return Err(Error::new(
                        "UnsupportedCapability",
                        "command supports Length parameters",
                    ));
                }
                Ok(self.handle(PARAMETER, key))
            }
            (Operation::Value, [parameter]) => {
                let key = self.key(parameter)?;
                overlay
                    .entries()
                    .get(key)
                    .ok_or_else(|| Error::new("UnknownParameter", key))?
                    .effective()
                    .host_value()
            }
            (Operation::Set, [parameter, value]) => {
                *overlay = overlay
                    .with_override(self.key(parameter)?, Some(Kind::Length.parse(value)?))?;
                Ok(Value::Unit)
            }
            (Operation::Reset, [parameter]) => {
                *overlay = overlay.with_override(self.key(parameter)?, None)?;
                Ok(Value::Unit)
            }
            _ => Err(Error::new("HostTypeMismatch", "invalid command arguments")),
        }
    }
}

fn unavailable() -> Error {
    Error::new(
        "CommandContextUnavailable",
        "override overlay is unavailable",
    )
}
