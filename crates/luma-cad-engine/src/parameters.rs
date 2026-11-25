use luma_host_api::{Error, Type, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_COUNT: u64 = (1u64 << 53) - 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Kind {
    Length,
    Angle,
    Scalar,
    Count,
    Boolean,
}

impl Kind {
    pub(crate) const ALL: [Self; 5] = [
        Self::Length,
        Self::Angle,
        Self::Scalar,
        Self::Count,
        Self::Boolean,
    ];
    pub(crate) const EXPR: [Self; 3] = [Self::Length, Self::Angle, Self::Scalar];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Length => "Length",
            Self::Angle => "Angle",
            Self::Scalar => "Scalar",
            Self::Count => "Count",
            Self::Boolean => "Boolean",
        }
    }

    pub(crate) fn value_type(self) -> Type {
        match self {
            Self::Length | Self::Angle => self.opaque_type("Quantity"),
            Self::Scalar | Self::Count => Type::Number,
            Self::Boolean => Type::Bool,
        }
    }

    pub(crate) fn opaque_type(self, family: &str) -> Type {
        Type::Opaque(format!("{family}<{}>", self.name()))
    }

    pub(crate) fn physical_number(self, number: f64) -> Result<ParamValue, Error> {
        let value = match self {
            Self::Length => ParamValue::Length(number),
            Self::Angle => ParamValue::Angle(number),
            Self::Scalar => ParamValue::Scalar(number),
            _ => {
                return Err(Error::new(
                    "ParameterTypeMismatch",
                    "expected an arithmetic parameter kind",
                ))
            }
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn parse(self, value: &Value) -> Result<ParamValue, Error> {
        let value = match (self, value) {
            (Self::Length | Self::Angle, Value::Opaque { type_name, payload })
                if self.value_type() == Type::Opaque(type_name.clone()) =>
            {
                let number = payload
                    .as_str()
                    .and_then(|text| text.parse().ok())
                    .ok_or_else(|| Error::new("InvalidQuantity", "expected a finite SI decimal"))?;
                self.physical_number(number)?
            }
            (Self::Scalar, Value::Number(number)) => ParamValue::Scalar(*number),
            (Self::Count, Value::Number(number)) => {
                if !(0.0..=MAX_COUNT as f64).contains(number) || number.fract() != 0.0 {
                    return Err(Error::new(
                        "ParameterCountRange",
                        "Count must be an integer in 0…2^53−1",
                    ));
                }
                ParamValue::Count(*number as u64)
            }
            (Self::Boolean, Value::Bool(value)) => ParamValue::Boolean(*value),
            _ => {
                return Err(Error::new(
                    "ParameterTypeMismatch",
                    "value does not match parameter kind",
                ))
            }
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn wrap<T: Serialize>(self, family: &str, value: T) -> Result<Value, Error> {
        let Type::Opaque(type_name) = self.opaque_type(family) else {
            unreachable!()
        };
        let payload = serde_json::to_value(value)
            .map_err(|error| Error::new("ParameterEncoding", error.to_string()))?;
        Ok(Value::Opaque { type_name, payload })
    }
}

/// Values live in SI in the engine. Count has a lossless integer representation.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum ParamValue {
    Length(f64),
    Angle(f64),
    Scalar(f64),
    Count(u64),
    Boolean(bool),
}

impl ParamValue {
    pub fn kind(self) -> Kind {
        match self {
            Self::Length(_) => Kind::Length,
            Self::Angle(_) => Kind::Angle,
            Self::Scalar(_) => Kind::Scalar,
            Self::Count(_) => Kind::Count,
            Self::Boolean(_) => Kind::Boolean,
        }
    }

    pub(crate) fn number(self) -> Result<f64, Error> {
        match self {
            Self::Length(value) | Self::Angle(value) | Self::Scalar(value) => Ok(value),
            Self::Count(value) => Ok(value as f64),
            Self::Boolean(_) => Err(Error::new(
                "ParameterTypeMismatch",
                "Boolean has no order or arithmetic",
            )),
        }
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        match self {
            Self::Boolean(_) => Ok(()),
            Self::Count(value) if value > MAX_COUNT => {
                Err(Error::new("ParameterCountRange", "Count exceeds 2^53−1"))
            }
            _ if !self.number()?.is_finite() => Err(Error::new(
                "NonFiniteParameter",
                "parameter value must be finite",
            )),
            _ => Ok(()),
        }
    }

    pub(crate) fn host_value(self) -> Result<Value, Error> {
        match self {
            Self::Length(number) | Self::Angle(number) => {
                self.kind().wrap("Quantity", number.to_string())
            }
            Self::Scalar(number) => Ok(Value::Number(number)),
            Self::Count(number) => Ok(Value::Number(number as f64)),
            Self::Boolean(value) => Ok(Value::Bool(value)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub default: ParamValue,
    pub bounds: Option<(ParamValue, ParamValue)>,
    pub exposed: bool,
}

impl Declaration {
    pub(crate) fn check(&self, value: ParamValue) -> Result<(), Error> {
        if value.kind() != self.default.kind() {
            return Err(Error::new(
                "ParameterTypeMismatch",
                "value does not match declared parameter kind",
            ));
        }
        value.validate()?;
        if let Some((min, max)) = self.bounds {
            if min.kind() != value.kind() || max.kind() != value.kind() {
                return Err(Error::new(
                    "ParameterTypeMismatch",
                    "parameter bounds must match the declared kind",
                ));
            }
            min.validate()?;
            max.validate()?;
            let (min, max) = (min.number()?, max.number()?);
            if min > max || !(min..=max).contains(&value.number()?) {
                return Err(Error::new(
                    "ParameterBounds",
                    "parameter value is outside its inclusive bounds",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub declaration: Declaration,
    pub override_value: Option<ParamValue>,
}

impl Parameter {
    pub fn effective(&self) -> ParamValue {
        self.override_value.unwrap_or(self.declaration.default)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.declaration.check(self.declaration.default)?;
        self.declaration.check(self.effective())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ParameterSnapshot(BTreeMap<String, Parameter>);

impl ParameterSnapshot {
    pub fn new(parameters: BTreeMap<String, Parameter>) -> Result<Self, Error> {
        for (key, value) in &parameters {
            validate_key(key)?;
            value.validate()?;
        }
        Ok(Self(parameters))
    }

    pub fn entries(&self) -> &BTreeMap<String, Parameter> {
        &self.0
    }

    /// Prepare an override edit against one admitted snapshot. The original is immutable.
    pub fn with_override(&self, key: &str, value: Option<ParamValue>) -> Result<Self, Error> {
        self.with_overrides([(key, value)])
    }

    pub fn with_overrides<'a>(
        &self,
        edits: impl IntoIterator<Item = (&'a str, Option<ParamValue>)>,
    ) -> Result<Self, Error> {
        let mut entries = self.0.clone();
        for (key, value) in edits {
            let parameter = entries
                .get_mut(key)
                .ok_or_else(|| Error::new("UnknownParameter", key))?;
            if !parameter.declaration.exposed {
                return Err(Error::new("ParameterNotExposed", key));
            }
            if let Some(value) = value {
                parameter.declaration.check(value)?;
            }
            parameter.override_value = value;
        }
        Ok(Self(entries))
    }
}

pub(crate) fn validate_key(key: &str) -> Result<(), Error> {
    let mut bytes = key.bytes();
    if key.len() <= 128
        && matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.:/-".contains(&c))
    {
        Ok(())
    } else {
        Err(Error::new(
            "InvalidStableKey",
            "parameter key must be a canonical StableKey",
        ))
    }
}
