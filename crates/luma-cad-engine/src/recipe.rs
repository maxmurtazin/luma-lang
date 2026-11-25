use crate::{Declaration, ExprOp, Kind, ParamExpr, Parameter, ParameterSnapshot};
use luma_cad_contract::{UnitsHost, SEALED_MODULES};
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter as Arg, Registry, ResultType,
    Type, Value,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Mutex};

static NEXT_SCOPE: Mutex<u64> = Mutex::new(0);
const MAX_PARAMETERS: usize = 2048;

#[derive(Clone, Copy)]
enum Operation {
    Declare(Kind),
    Bounds,
    Expose,
    Value,
    Add,
    Subtract,
    Scale,
    DivideScalar,
    Negate,
}

const BINDINGS: &[(&str, Operation)] = &[
    ("length", Operation::Declare(Kind::Length)),
    ("angle", Operation::Declare(Kind::Angle)),
    ("scalar", Operation::Declare(Kind::Scalar)),
    ("count", Operation::Declare(Kind::Count)),
    ("boolean", Operation::Declare(Kind::Boolean)),
    ("bounds", Operation::Bounds),
    ("expose", Operation::Expose),
    ("value", Operation::Value),
    ("add", Operation::Add),
    ("subtract", Operation::Subtract),
    ("scale", Operation::Scale),
    ("divide_scalar", Operation::DivideScalar),
    ("negate", Operation::Negate),
];

fn refs(kinds: impl IntoIterator<Item = Kind>) -> Arg {
    Arg::Variable {
        slot: 0,
        accepts: kinds
            .into_iter()
            .map(|kind| kind.opaque_type("ParamRef"))
            .collect(),
    }
}

fn mapping(types: impl IntoIterator<Item = (Type, Type)>) -> Arg {
    Arg::Mapped {
        slot: 0,
        types: types.into_iter().collect(),
    }
}

fn expr_input() -> Arg {
    mapping(Kind::EXPR.into_iter().flat_map(|kind| {
        let expr = kind.opaque_type("ParamExpr");
        [
            (kind.value_type(), expr.clone()),
            (kind.opaque_type("ParamRef"), expr.clone()),
            (expr.clone(), expr),
        ]
    }))
}

impl Operation {
    fn descriptor(self, name: &str) -> Descriptor {
        let exact = Arg::Exact;
        let variable = ResultType::Variable(0);
        let (parameters, result, effect) = match self {
            Self::Declare(kind) => (
                vec![exact(Type::Text), exact(kind.value_type())],
                ResultType::Exact(kind.opaque_type("ParamRef")),
                Effect::BuildPlan,
            ),
            Self::Bounds => {
                // Scalar and Count share Number as their materialized type.
                // Resolve the bound's kind from the reference at execution.
                let compatible = [Kind::Length, Kind::Angle, Kind::Scalar, Kind::Count];
                (
                    vec![refs(compatible), bound_arg(), bound_arg()],
                    variable,
                    Effect::BuildPlan,
                )
            }
            Self::Expose => (
                vec![refs(Kind::ALL), exact(Type::Bool)],
                variable,
                Effect::BuildPlan,
            ),
            Self::Value => (
                vec![mapping(Kind::ALL.map(|kind| {
                    (kind.opaque_type("ParamRef"), kind.value_type())
                }))],
                variable,
                Effect::ReadPinned,
            ),
            Self::Add | Self::Subtract => {
                (vec![expr_input(), expr_input()], variable, Effect::Pure)
            }
            Self::Scale | Self::DivideScalar => (
                vec![expr_input(), exact(Type::Number)],
                variable,
                Effect::Pure,
            ),
            Self::Negate => (vec![expr_input()], variable, Effect::Pure),
        };
        Descriptor {
            module: "param".into(),
            name: name.into(),
            operation: format!("param.{name}"),
            major: 1,
            parameters,
            result,
            effect,
            modes: [Mode::Recipe].into(),
        }
    }
}

fn bound_arg() -> Arg {
    // The bounds share slot 1 independently of the reference type. The evaluator validates
    // numeric range and exact kind against the selected reference.
    Arg::Variable {
        slot: 1,
        accepts: [
            Type::Number,
            Kind::Length.value_type(),
            Kind::Angle.value_type(),
        ]
        .into(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scoped<T> {
    scope: u64,
    value: T,
}

/// A transient, recipe-local host. Construct one per checked execution.
pub struct ParameterHost {
    units: UnitsHost,
    registry: Registry,
    inherited: ParameterSnapshot,
    declarations: Mutex<BTreeMap<String, Parameter>>,
    scope: u64,
}

#[derive(Debug, Serialize)]
pub struct ParameterPlan {
    pub parameters: ParameterSnapshot,
    pub expressions: BTreeMap<String, ParamExpr>,
}

impl ParameterHost {
    pub fn new(inherited: ParameterSnapshot) -> Result<Self, Error> {
        let units = UnitsHost::new();
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            units.registry().descriptors().cloned().chain(
                BINDINGS
                    .iter()
                    .map(|(name, operation)| operation.descriptor(name)),
            ),
        )?;
        let scope = {
            let mut next = NEXT_SCOPE.lock().map_err(|_| poisoned())?;
            let scope = *next;
            *next = scope.checked_add(1).ok_or_else(|| {
                Error::new(
                    "ParameterScopeExhausted",
                    "could not allocate a preparation scope",
                )
            })?;
            scope
        };
        Ok(Self {
            units,
            registry,
            inherited,
            declarations: Mutex::new(BTreeMap::new()),
            scope,
        })
    }

    /// Consume a successful execution's local host to produce its parameter plan.
    pub fn finish(self, outputs: &BTreeMap<String, Value>) -> Result<ParameterPlan, Error> {
        let mut expressions = BTreeMap::new();
        for (name, value) in outputs {
            if let Some(kind) = kind_of(value, "ParamExpr") {
                let expression: ParamExpr = self.unwrap(value, kind, "ParamExpr")?;
                expressions.insert(name.clone(), expression);
            }
        }
        let entries = self.declarations.into_inner().map_err(|_| poisoned())?;
        for (key, previous) in self.inherited.entries() {
            if previous.override_value.is_some() && !entries.contains_key(key) {
                return Err(Error::new("OverrideOrphaned", key));
            }
        }
        let parameters = ParameterSnapshot::new(entries)?;
        for expression in expressions.values() {
            expression.evaluate(&parameters)?;
        }
        Ok(ParameterPlan {
            parameters,
            expressions,
        })
    }

    fn wrap<T: Serialize>(&self, kind: Kind, family: &str, value: T) -> Result<Value, Error> {
        let Type::Opaque(type_name) = kind.opaque_type(family) else {
            unreachable!()
        };
        self.wrap_handle(&type_name, value)
    }

    fn unwrap<T: for<'de> Deserialize<'de>>(
        &self,
        value: &Value,
        kind: Kind,
        family: &str,
    ) -> Result<T, Error> {
        let Type::Opaque(type_name) = kind.opaque_type(family) else {
            unreachable!()
        };
        self.unwrap_handle(value, &type_name)
    }

    pub(crate) fn wrap_handle<T: Serialize>(&self, name: &str, value: T) -> Result<Value, Error> {
        let payload = serde_json::to_value(Scoped {
            scope: self.scope,
            value,
        })
        .map_err(|error| Error::new("ParameterEncoding", error.to_string()))?;
        Ok(Value::Opaque {
            type_name: name.into(),
            payload,
        })
    }

    pub(crate) fn unwrap_handle<T: for<'de> Deserialize<'de>>(
        &self,
        value: &Value,
        name: &str,
    ) -> Result<T, Error> {
        let Value::Opaque { type_name, payload } = value else {
            return Err(bad_handle());
        };
        if name != type_name {
            return Err(bad_handle());
        }
        let handle: Scoped<T> =
            serde_json::from_value(payload.clone()).map_err(|_| bad_handle())?;
        if handle.scope != self.scope {
            return Err(Error::new(
                "ForeignParameterHandle",
                "host handle belongs to another preparation",
            ));
        }
        Ok(handle.value)
    }

    fn declare(&self, kind: Kind, key: &str, default: &Value) -> Result<Value, Error> {
        crate::parameters::validate_key(key)?;
        let default = kind.parse(default)?;
        let previous = self.inherited.entries().get(key);
        if previous.is_some_and(|previous| previous.declaration.default.kind() != kind) {
            return Err(Error::new("ParameterTypeMigrationRequired", key));
        }
        let mut entries = self.declarations.lock().map_err(|_| poisoned())?;
        if entries.contains_key(key) {
            return Err(Error::new("DuplicateStableKey", key));
        }
        if entries.len() >= MAX_PARAMETERS {
            return Err(Error::new(
                "ParameterLimit",
                "recipe exceeds 2048 parameters",
            ));
        }
        entries.insert(
            key.into(),
            Parameter {
                declaration: Declaration {
                    default,
                    bounds: None,
                    exposed: true,
                },
                override_value: previous.and_then(|value| value.override_value),
            },
        );
        self.wrap(kind, "ParamRef", key)
    }

    fn parameter(
        &self,
        value: &Value,
        update: impl FnOnce(&mut Parameter, Kind) -> Result<Value, Error>,
    ) -> Result<Value, Error> {
        let kind = kind_of(value, "ParamRef").ok_or_else(bad_handle)?;
        let key: String = self.unwrap(value, kind, "ParamRef")?;
        let mut entries = self.declarations.lock().map_err(|_| poisoned())?;
        let entry = entries
            .get_mut(&key)
            .ok_or_else(|| Error::new("UnknownParameter", key))?;
        if entry.declaration.default.kind() != kind {
            return Err(bad_handle());
        }
        update(entry, kind)
    }

    pub(crate) fn expression(
        &self,
        argument: &Value,
        kind: Kind,
        site: &CallSite,
    ) -> Result<ParamExpr, Error> {
        if kind_of(argument, "ParamExpr") == Some(kind) {
            return self.unwrap(argument, kind, "ParamExpr");
        }
        let operation = if kind_of(argument, "ParamRef") == Some(kind) {
            ExprOp::Parameter {
                key: self.unwrap(argument, kind, "ParamRef")?,
            }
        } else {
            ExprOp::Literal {
                value: kind.parse(argument)?,
            }
        };
        Ok(ParamExpr::leaf(kind, operation, site))
    }
}

impl Dispatch for ParameterHost {
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
        if descriptor.module == "units" {
            return self.units.call(descriptor, mode, site, arguments);
        }
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new(
                "HostRegistryMismatch",
                "descriptor is unknown to the parameter profile",
            ));
        }
        descriptor.check_mode(mode)?;
        let types = arguments
            .iter()
            .map(|value| {
                value.value_type().ok_or_else(|| {
                    Error::new("NonFiniteParameter", "parameter argument must be finite")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let expected = descriptor.specialize(&types)?;
        let operation = BINDINGS
            .iter()
            .find_map(|(name, op)| (*name == descriptor.name).then_some(*op))
            .ok_or_else(|| Error::new("UnsupportedCapability", &descriptor.operation))?;
        match (operation, arguments) {
            (Operation::Declare(kind), [Value::Text(key), default]) => {
                self.declare(kind, key, default)
            }
            (Operation::Bounds, [reference, min, max]) => {
                self.parameter(reference, |parameter, kind| {
                    let mut declaration = parameter.declaration.clone();
                    declaration.bounds = Some((kind.parse(min)?, kind.parse(max)?));
                    declaration.check(declaration.default)?;
                    declaration.check(parameter.effective())?;
                    parameter.declaration = declaration;
                    Ok(reference.clone())
                })
            }
            (Operation::Expose, [reference, Value::Bool(exposed)]) => {
                self.parameter(reference, |parameter, _| {
                    parameter.declaration.exposed = *exposed;
                    Ok(reference.clone())
                })
            }
            (Operation::Value, [reference]) => {
                self.parameter(reference, |parameter, _| parameter.effective().host_value())
            }
            (operation, [first, rest @ ..]) => {
                let kind = Kind::EXPR
                    .into_iter()
                    .find(|kind| kind.opaque_type("ParamExpr") == expected)
                    .ok_or_else(|| {
                        Error::new("HostTypeMismatch", "expected a typed expression result")
                    })?;
                let expression = self.expression(first, kind, site)?;
                let input = expression.root()?;
                let result = match (operation, rest) {
                    (Operation::Add, [rhs]) => expression.combine(
                        self.expression(rhs, kind, site)?,
                        |left, right| ExprOp::Add { left, right },
                        site,
                    )?,
                    (Operation::Subtract, [rhs]) => expression.combine(
                        self.expression(rhs, kind, site)?,
                        |left, right| ExprOp::Subtract { left, right },
                        site,
                    )?,
                    (Operation::Scale, [Value::Number(scalar)]) => expression.append(
                        ExprOp::Scale {
                            input,
                            scalar: *scalar,
                        },
                        site,
                    )?,
                    (Operation::DivideScalar, [Value::Number(0.0)]) => {
                        return Err(Error::new(
                            "ParameterDivisionByZero",
                            "expression divisor must be nonzero",
                        ));
                    }
                    (Operation::DivideScalar, [Value::Number(scalar)]) => expression.append(
                        ExprOp::DivideScalar {
                            input,
                            scalar: *scalar,
                        },
                        site,
                    )?,
                    (Operation::Negate, []) => expression.append(ExprOp::Negate { input }, site)?,
                    _ => {
                        return Err(Error::new(
                            "HostTypeMismatch",
                            "unexpected parameter arguments",
                        ))
                    }
                };
                self.wrap(kind, "ParamExpr", result)
            }
            _ => Err(Error::new(
                "HostTypeMismatch",
                "unexpected parameter arguments",
            )),
        }
    }
}

fn kind_of(value: &Value, family: &str) -> Option<Kind> {
    let value_type = value.value_type()?;
    Kind::ALL
        .into_iter()
        .find(|kind| kind.opaque_type(family) == value_type)
}

fn bad_handle() -> Error {
    Error::new(
        "InvalidParameterHandle",
        "expected a typed local parameter handle",
    )
}

fn poisoned() -> Error {
    Error::new(
        "ParameterSessionError",
        "local parameter state is unavailable",
    )
}
