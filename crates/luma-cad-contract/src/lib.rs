//! CAD descriptors and provider-independent quantity operations.
mod source;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
pub use source::source_digest;
use std::collections::BTreeSet;

pub const SEALED_MODULES: &[&str] = &[
    "cad", "units", "param", "project", "sketch", "draft", "drawing", "implicit", "brep",
    "lattice", "assembly", "elements", "voxel", "sim", "view", "studio", "render", "jobs",
    "artifact",
];

#[derive(Clone, Copy)]
enum Dimension {
    Length,
    Angle,
    Time,
}

impl Dimension {
    fn name(self) -> &'static str {
        match self {
            Self::Length => "Quantity<Length>",
            Self::Angle => "Quantity<Angle>",
            Self::Time => "Quantity<Time>",
        }
    }

    fn ty(self) -> Type {
        Type::Opaque(self.name().into())
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Convert(Dimension, f64),
    Add,
    Subtract,
}

const DEFINITIONS: &[(&str, Operation)] = &[
    ("m", Operation::Convert(Dimension::Length, 1.0)),
    ("cm", Operation::Convert(Dimension::Length, 0.01)),
    ("mm", Operation::Convert(Dimension::Length, 0.001)),
    ("inch", Operation::Convert(Dimension::Length, 0.0254)),
    ("rad", Operation::Convert(Dimension::Angle, 1.0)),
    (
        "deg",
        Operation::Convert(Dimension::Angle, std::f64::consts::PI / 180.0),
    ),
    ("seconds", Operation::Convert(Dimension::Time, 1.0)),
    ("add", Operation::Add),
    ("subtract", Operation::Subtract),
];

impl Operation {
    fn descriptor(self, name: &str) -> Descriptor {
        let (parameters, result) = match self {
            Self::Convert(dimension, _) => (
                vec![Parameter::Exact(Type::Number)],
                ResultType::Exact(dimension.ty()),
            ),
            Self::Add | Self::Subtract => {
                let accepts: BTreeSet<_> = [Dimension::Length, Dimension::Angle, Dimension::Time]
                    .into_iter()
                    .map(Dimension::ty)
                    .collect();
                let quantity = Parameter::Variable { slot: 0, accepts };
                (vec![quantity.clone(), quantity], ResultType::Variable(0))
            }
        };
        Descriptor {
            module: "units".into(),
            name: name.into(),
            operation: format!("units.{name}"),
            major: 1,
            parameters,
            result,
            effect: Effect::Pure,
            modes: [Mode::Recipe, Mode::Command].into(),
        }
    }

    fn evaluate(self, values: &[Value]) -> Result<Value, Error> {
        let (name, number) = match (self, values) {
            (Self::Convert(dimension, scale), [Value::Number(value)]) => {
                (dimension.name(), value * scale)
            }
            (
                Self::Add | Self::Subtract,
                [Value::Opaque {
                    type_name,
                    payload: a,
                }, Value::Opaque { payload: b, .. }],
            ) => {
                let a = quantity_value(a)?;
                let b = quantity_value(b)?;
                let result = match self {
                    Self::Add => a + b,
                    _ => a - b,
                };
                (type_name.as_str(), result)
            }
            _ => return Err(Error::new("HostTypeMismatch", "invalid quantity arguments")),
        };
        if !number.is_finite() {
            return Err(Error::new("NonFiniteQuantity", "quantity must be finite"));
        }
        let si_value = if number == 0.0 { 0.0 } else { number };
        Ok(Value::Opaque {
            type_name: name.into(),
            payload: si_value.to_string().into(),
        })
    }
}

fn quantity_value(payload: &serde_json::Value) -> Result<f64, Error> {
    payload
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .ok_or_else(|| Error::new("InvalidQuantity", "expected a finite SI decimal"))
}

/// Embedded quantity profile shared by CAD check and evaluation.
pub struct UnitsHost {
    registry: Registry,
}

impl UnitsHost {
    pub fn new() -> Self {
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            DEFINITIONS
                .iter()
                .map(|(name, operation)| operation.descriptor(name)),
        )
        .expect("static CAD quantity profile");
        Self { registry }
    }
}

impl Default for UnitsHost {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatch for UnitsHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _site: &CallSite,
        arguments: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new(
                "HostRegistryMismatch",
                "quantity operation is unknown to this profile",
            ));
        }
        let operation = DEFINITIONS
            .iter()
            .find_map(|(name, operation)| (*name == descriptor.name).then_some(*operation))
            .expect("descriptor and dispatch use the same table");
        descriptor.check_mode(mode)?;
        let types = arguments
            .iter()
            .map(|value| {
                value.value_type().ok_or_else(|| {
                    Error::new("NonFiniteNumber", "quantity argument must be finite")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        descriptor.specialize(&types)?;
        operation.evaluate(arguments)
    }
}
