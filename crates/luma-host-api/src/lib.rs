//! Embedded host descriptors and typed values, independent of any CAD backend.
use serde::{Deserialize, Serialize};
use serde_json::Value as Payload;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Recipe,
    Command,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Effect {
    Pure,
    ReadPinned,
    BuildPlan,
    DeclareTask,
    ViewIntent,
    ExternalRead,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Type {
    Number,
    Text,
    Bool,
    Unit,
    Opaque(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Parameter {
    Exact(Type),
    Variable {
        slot: u8,
        accepts: BTreeSet<Type>,
    },
    /// Map distinct admitted argument types into one finite signature variable.
    Mapped {
        slot: u8,
        #[serde(with = "type_pairs")]
        types: BTreeMap<Type, Type>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResultType {
    Exact(Type),
    Variable(u8),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub module: String,
    pub name: String,
    pub operation: String,
    pub major: u32,
    pub parameters: Vec<Parameter>,
    pub result: ResultType,
    pub effect: Effect,
    pub modes: BTreeSet<Mode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registry {
    modules: BTreeMap<String, BTreeMap<String, Descriptor>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    Unit,
    Opaque { type_name: String, payload: Payload },
}

impl Value {
    pub fn value_type(&self) -> Option<Type> {
        Some(match self {
            Self::Number(number) if number.is_finite() => Type::Number,
            Self::Number(_) => return None,
            Self::Text(_) => Type::Text,
            Self::Bool(_) => Type::Bool,
            Self::Unit => Type::Unit,
            Self::Opaque { type_name, .. } => Type::Opaque(type_name.clone()),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl Registry {
    /// Sealed module names are reserved even when the profile provides zero bindings for them.
    pub fn new(
        sealed_modules: impl IntoIterator<Item = String>,
        descriptors: impl IntoIterator<Item = Descriptor>,
    ) -> Result<Self, Error> {
        let mut modules = BTreeMap::new();
        for module in sealed_modules {
            if !identifier(&module) || modules.insert(module, BTreeMap::new()).is_some() {
                return Err(Error::new(
                    "InvalidHostRegistry",
                    "invalid or duplicate sealed module",
                ));
            }
        }
        let mut operations = BTreeSet::new();
        for descriptor in descriptors {
            descriptor.validate()?;
            let members = modules.get_mut(&descriptor.module).ok_or_else(|| {
                Error::new("InvalidHostRegistry", "descriptor module must be sealed")
            })?;
            if !operations.insert(descriptor.operation.clone())
                || members
                    .insert(descriptor.name.clone(), descriptor)
                    .is_some()
            {
                return Err(Error::new(
                    "InvalidHostRegistry",
                    "duplicate host operation or member",
                ));
            }
        }
        Ok(Self { modules })
    }

    /// Add native functions to modules already sealed by this profile.
    pub fn with_descriptors(
        &self,
        descriptors: impl IntoIterator<Item = Descriptor>,
    ) -> Result<Self, Error> {
        Self::new(
            self.modules.keys().cloned(),
            self.descriptors().cloned().chain(descriptors),
        )
    }

    pub fn seals(&self, module: &str) -> bool {
        self.modules.contains_key(module)
    }

    pub fn get(&self, module: &str, name: &str) -> Option<&Descriptor> {
        self.modules.get(module)?.get(name)
    }

    pub fn descriptors(&self) -> impl Iterator<Item = &Descriptor> {
        self.modules.values().flat_map(|members| members.values())
    }
}

fn identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

impl Descriptor {
    fn validate(&self) -> Result<(), Error> {
        let invalid = |message| Err(Error::new("InvalidHostRegistry", message));
        if !identifier(&self.name)
            || self.operation.is_empty()
            || self.major == 0
            || self.modes.is_empty()
        {
            return invalid("invalid operation name, ID, major or mode set");
        }
        let mut slots = BTreeMap::new();
        for parameter in &self.parameters {
            let (slot, domain) = match parameter {
                Parameter::Exact(_) => continue,
                Parameter::Variable { slot, accepts } => (*slot, accepts.clone()),
                Parameter::Mapped { slot, types } => (*slot, types.values().cloned().collect()),
            };
            if domain.is_empty() || slots.get(&slot).is_some_and(|earlier| earlier != &domain) {
                return invalid(
                    "every occurrence of a type variable must have the same finite domain",
                );
            }
            slots.insert(slot, domain);
        }
        if matches!(self.result, ResultType::Variable(slot) if !slots.contains_key(&slot)) {
            return invalid("result refers to an unbound type variable");
        }
        Ok(())
    }

    pub fn check_mode(&self, mode: Mode) -> Result<(), Error> {
        if self.modes.contains(&mode) {
            Ok(())
        } else {
            Err(Error::new(
                "HostModeMismatch",
                format!("{} is unavailable in {mode:?} mode", self.operation),
            ))
        }
    }

    /// Solve the finite scheme afresh for each invocation.
    pub fn specialize(&self, types: &[Type]) -> Result<Type, Error> {
        if self.parameters.len() != types.len() {
            return Err(Error::new(
                "E_ARITY",
                format!(
                    "{} expects {} arguments",
                    self.operation,
                    self.parameters.len()
                ),
            ));
        }
        let mut slots = BTreeMap::new();
        for (index, (parameter, actual)) in self.parameters.iter().zip(types).enumerate() {
            let matches = match parameter {
                Parameter::Exact(expected) => expected == actual,
                Parameter::Variable { slot, accepts } => {
                    accepts.contains(actual) && *slots.entry(*slot).or_insert(actual) == actual
                }
                Parameter::Mapped { slot, types } => types
                    .get(actual)
                    .is_some_and(|mapped| *slots.entry(*slot).or_insert(mapped) == mapped),
            };
            if !matches {
                return Err(Error::new(
                    "HostTypeMismatch",
                    format!(
                        "argument {} of {} has incompatible type {actual:?}",
                        index + 1,
                        self.operation
                    ),
                ));
            }
        }
        match &self.result {
            ResultType::Exact(ty) => Ok(ty.clone()),
            ResultType::Variable(slot) => {
                slots.get(slot).map(|ty| (*ty).clone()).ok_or_else(|| {
                    Error::new(
                        "InvalidHostRegistry",
                        "result refers to an unbound type variable",
                    )
                })
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallSite {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

/// Native dispatch is trusted code. During evaluate it may use captured inputs and return
/// proposals, but it must not mutate a project or perform uncaptured IO.
pub trait Dispatch: Sync {
    fn registry(&self) -> &Registry;

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        site: &CallSite,
        arguments: &[Value],
    ) -> Result<Value, Error>;
}

mod type_pairs {
    use super::Type;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub fn serialize<S: serde::Serializer>(
        types: &BTreeMap<Type, Type>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        types.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Type, Type>, D::Error> {
        use serde::de::Error;
        let pairs = Vec::<(Type, Type)>::deserialize(deserializer)?;
        let length = pairs.len();
        let types: BTreeMap<_, _> = pairs.into_iter().collect();
        if types.len() != length {
            return Err(D::Error::custom("duplicate type mapping"));
        }
        Ok(types)
    }
}
