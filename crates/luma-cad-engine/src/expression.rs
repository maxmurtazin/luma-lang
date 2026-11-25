use crate::parameters::{Kind, ParamValue, ParameterSnapshot};
use luma_host_api::{CallSite, Error};
use serde::{Deserialize, Serialize};

const MAX_NODES: usize = 2048;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExprOp {
    Literal { value: ParamValue },
    Parameter { key: String },
    Add { left: usize, right: usize },
    Subtract { left: usize, right: usize },
    Scale { input: usize, scalar: f64 },
    DivideScalar { input: usize, scalar: f64 },
    Negate { input: usize },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExprNode {
    pub operation: ExprOp,
    pub site: CallSite,
}

/// A topologically ordered expression arena. The last node is its result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParamExpr {
    pub version: u32,
    pub kind: Kind,
    pub nodes: Vec<ExprNode>,
}

impl ParamExpr {
    pub(crate) fn leaf(kind: Kind, operation: ExprOp, site: &CallSite) -> Self {
        Self {
            version: 1,
            kind,
            nodes: vec![ExprNode {
                operation,
                site: site.clone(),
            }],
        }
    }

    pub(crate) fn append(mut self, operation: ExprOp, site: &CallSite) -> Result<Self, Error> {
        self.push(ExprNode {
            operation,
            site: site.clone(),
        })?;
        Ok(self)
    }

    fn push(&mut self, node: ExprNode) -> Result<usize, Error> {
        if self.nodes.len() >= MAX_NODES {
            return Err(Error::new(
                "ParameterExpressionLimit",
                "expression exceeds 2048 nodes",
            ));
        }
        let index = self.nodes.len();
        self.nodes.push(node);
        Ok(index)
    }

    pub(crate) fn root(&self) -> Result<usize, Error> {
        if self.version != 1
            || self.nodes.is_empty()
            || self.nodes.len() > MAX_NODES
            || !Kind::EXPR.contains(&self.kind)
        {
            return Err(Error::new(
                "InvalidParameterExpression",
                "unsupported or empty expression arena",
            ));
        }
        Ok(self.nodes.len() - 1)
    }

    fn import(&mut self, other: Self) -> Result<usize, Error> {
        other.root()?;
        if self.kind != other.kind {
            return Err(Error::new(
                "ParameterTypeMismatch",
                "expression dimensions differ",
            ));
        }
        let mut slots = Vec::with_capacity(other.nodes.len());
        for node in other.nodes {
            let operation = node
                .operation
                .remap(|arg| slots.get(arg).copied().ok_or_else(bad_edge))?;
            let node = ExprNode {
                operation,
                site: node.site,
            };
            let index = match self.nodes.iter().position(|prior| prior == &node) {
                Some(index) => index,
                None => self.push(node)?,
            };
            slots.push(index);
        }
        slots.last().copied().ok_or_else(bad_edge)
    }

    pub(crate) fn combine(
        mut self,
        other: Self,
        operation: fn(usize, usize) -> ExprOp,
        site: &CallSite,
    ) -> Result<Self, Error> {
        let left = self.root()?;
        let right = self.import(other)?;
        self.append(operation(left, right), site)
    }

    pub fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().filter_map(|node| match &node.operation {
            ExprOp::Parameter { key } => Some(key.as_str()),
            _ => None,
        })
    }

    pub fn evaluate(&self, context: &ParameterSnapshot) -> Result<ParamValue, Error> {
        let root = self.root()?;
        let mut values: Vec<f64> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let read = |arg| values.get(arg).copied().ok_or_else(bad_edge);
            let value = match &node.operation {
                ExprOp::Literal { value } => *value,
                ExprOp::Parameter { key } => context
                    .entries()
                    .get(key)
                    .ok_or_else(|| Error::new("UnknownParameter", key))?
                    .effective(),
                ExprOp::Add { left, right } => {
                    self.kind.physical_number(read(*left)? + read(*right)?)?
                }
                ExprOp::Subtract { left, right } => {
                    self.kind.physical_number(read(*left)? - read(*right)?)?
                }
                ExprOp::Scale { scalar, .. } | ExprOp::DivideScalar { scalar, .. }
                    if !scalar.is_finite() =>
                {
                    return Err(Error::new(
                        "NonFiniteParameter",
                        "expression scalar must be finite",
                    ));
                }
                ExprOp::Scale { input, scalar } => {
                    self.kind.physical_number(read(*input)? * scalar)?
                }
                ExprOp::DivideScalar { scalar: 0.0, .. } => {
                    return Err(Error::new(
                        "ParameterDivisionByZero",
                        "expression divisor must be nonzero",
                    ));
                }
                ExprOp::DivideScalar { input, scalar } => {
                    self.kind.physical_number(read(*input)? / scalar)?
                }
                ExprOp::Negate { input } => self.kind.physical_number(-read(*input)?)?,
            };
            value.validate()?;
            if value.kind() != self.kind {
                return Err(Error::new(
                    "ParameterTypeMismatch",
                    "expression node has a different dimension",
                ));
            }
            values.push(value.number()?);
        }
        self.kind.physical_number(values[root])
    }
}

impl ExprOp {
    fn remap(self, map: impl Fn(usize) -> Result<usize, Error>) -> Result<Self, Error> {
        Ok(match self {
            Self::Add { left, right } => Self::Add {
                left: map(left)?,
                right: map(right)?,
            },
            Self::Subtract { left, right } => Self::Subtract {
                left: map(left)?,
                right: map(right)?,
            },
            Self::Scale { input, scalar } => Self::Scale {
                input: map(input)?,
                scalar,
            },
            Self::DivideScalar { input, scalar } => Self::DivideScalar {
                input: map(input)?,
                scalar,
            },
            Self::Negate { input } => Self::Negate { input: map(input)? },
            leaf => leaf,
        })
    }
}

fn bad_edge() -> Error {
    Error::new(
        "InvalidParameterExpression",
        "expression dependency must refer to an earlier node",
    )
}
