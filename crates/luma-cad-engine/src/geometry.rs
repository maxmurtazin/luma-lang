use crate::{Kind, ParamExpr, ParamValue, ParameterHost, ParameterPlan, ParameterSnapshot};
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

pub const GEOMETRY_PROFILE: &str = "recipe-geometry-v1";

const BODY: &str = "BodyRef";
const VIEW: &str = "ViewSpecRef";
// The recipe profile uses the D1 plate clearance policy. The provider
// validates the eventual cut and its datum independently.
const MAX_HOLES: usize = 64;
const MIN_CLEARANCE_M: f64 = 0.001;
const CLEARANCE_TOLERANCE_M: f64 = 1e-14;

#[derive(Clone, Copy)]
enum Operation {
    Plate,
    ThroughHole,
    Output,
    ViewBody,
    Show,
}

const BINDINGS: &[(&str, &str, Operation)] = &[
    ("cad", "plate", Operation::Plate),
    ("cad", "through_hole", Operation::ThroughHole),
    ("cad", "output", Operation::Output),
    ("view", "from_body", Operation::ViewBody),
    ("view", "show", Operation::Show),
];

impl Operation {
    fn descriptor(self, module: &str, name: &str) -> Descriptor {
        let text = Parameter::Exact(Type::Text);
        let body = Parameter::Exact(opaque(BODY));
        let (parameters, result, effect) = match self {
            Self::Plate => (
                vec![text, length(), length(), length()],
                opaque(BODY),
                Effect::BuildPlan,
            ),
            Self::ThroughHole => (
                vec![text, body, length(), length(), length()],
                opaque(BODY),
                Effect::BuildPlan,
            ),
            Self::Output => (vec![text, body], Type::Unit, Effect::BuildPlan),
            Self::ViewBody => (vec![body], opaque(VIEW), Effect::Pure),
            Self::Show => (
                vec![Parameter::Exact(opaque(VIEW))],
                Type::Unit,
                Effect::ViewIntent,
            ),
        };
        Descriptor {
            module: module.into(),
            name: name.into(),
            operation: format!("{module}.{name}"),
            major: 1,
            parameters,
            result: ResultType::Exact(result),
            effect,
            modes: [Mode::Recipe].into(),
        }
    }
}

fn opaque(name: &str) -> Type {
    Type::Opaque(name.into())
}

fn length() -> Parameter {
    let kind = Kind::Length;
    let physical = kind.value_type();
    Parameter::Mapped {
        slot: 0,
        types: [
            (physical.clone(), physical.clone()),
            (kind.opaque_type("ParamRef"), physical.clone()),
            (kind.opaque_type("ParamExpr"), physical),
        ]
        .into(),
    }
}

/// Feature positions and dimensions are SI and local to an axis-aligned,
/// centered plate. Body names refer to the `solid` port of earlier features.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Geometry {
    Plate {
        size_m: [f64; 3],
    },
    ThroughHole {
        base: String,
        diameter_m: f64,
        centre_m: [f64; 2],
        entry_z_m: f64,
    },
}

#[derive(Debug, Serialize)]
pub struct Feature {
    pub key: String,
    pub site: CallSite,
    pub geometry: Geometry,
    pub inputs: BTreeMap<&'static str, ParamExpr>,
}

#[derive(Debug, Serialize)]
pub struct ViewIntent {
    pub body: String,
    pub site: CallSite,
}

/// Provider-independent output. The Core adapter must resolve the logical keys,
/// acquire an authority snapshot and validate again before any commit.
#[derive(Debug, Serialize)]
pub struct RecipeGeometryPlan {
    pub profile: &'static str,
    pub source_digest: String,
    pub parameters: ParameterPlan,
    pub features: Vec<Feature>,
    pub outputs: BTreeMap<String, String>,
    pub view_intents: Vec<ViewIntent>,
}

enum FeatureDraft {
    Plate([ParamExpr; 3]),
    ThroughHole {
        base: String,
        inputs: [ParamExpr; 3],
    },
}

#[derive(Default)]
struct Draft {
    keys: BTreeSet<String>,
    features: Vec<(String, CallSite, FeatureDraft)>,
    outputs: BTreeMap<String, String>,
    views: Vec<ViewIntent>,
}

pub struct GeometryHost {
    parameters: ParameterHost,
    registry: Registry,
    draft: Mutex<Draft>,
}

impl GeometryHost {
    pub fn new(inherited: ParameterSnapshot) -> Result<Self, Error> {
        let parameters = ParameterHost::new(inherited)?;
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            parameters.registry().descriptors().cloned().chain(
                BINDINGS
                    .iter()
                    .map(|(module, name, operation)| operation.descriptor(module, name)),
            ),
        )?;
        Ok(Self {
            parameters,
            registry,
            draft: Mutex::default(),
        })
    }

    fn reference(&self, value: &Value, name: &str) -> Result<String, Error> {
        self.parameters
            .unwrap_handle(value, name)
            .map_err(|problem| {
                Error::new(
                    if problem.code == "ForeignParameterHandle" {
                        "ForeignGeometryHandle"
                    } else {
                        "InvalidGeometryHandle"
                    },
                    problem.message,
                )
            })
    }

    fn inputs(&self, values: [&Value; 3], site: &CallSite) -> Result<[ParamExpr; 3], Error> {
        let [first, second, third] = values;
        Ok([
            self.parameters.expression(first, Kind::Length, site)?,
            self.parameters.expression(second, Kind::Length, site)?,
            self.parameters.expression(third, Kind::Length, site)?,
        ])
    }

    fn feature(&self, key: &str, draft: FeatureDraft, site: &CallSite) -> Result<Value, Error> {
        crate::parameters::validate_key(key)?;
        let handle = self.parameters.wrap_handle(BODY, key)?;
        let mut state = self.draft.lock().map_err(|_| unavailable())?;
        if let FeatureDraft::ThroughHole { base, .. } = &draft {
            if !state.keys.contains(base) {
                return Err(Error::new("UnknownBody", base));
            }
        }
        if !state.keys.insert(key.into()) {
            return Err(Error::new("DuplicateStableKey", key));
        }
        state.features.push((key.into(), site.clone(), draft));
        Ok(handle)
    }

    pub fn finish(
        self,
        variables: &BTreeMap<String, Value>,
        source_digest: String,
    ) -> Result<RecipeGeometryPlan, Error> {
        let parameters = self.parameters.finish(variables)?;
        let draft = self.draft.into_inner().map_err(|_| unavailable())?;
        if let Some((key, _, _)) = draft
            .features
            .iter()
            .find(|(key, _, _)| parameters.parameters.entries().contains_key(key))
        {
            return Err(Error::new("DuplicateStableKey", key));
        }
        let features = resolve(draft.features, &parameters.parameters)?;
        Ok(RecipeGeometryPlan {
            profile: GEOMETRY_PROFILE,
            source_digest,
            parameters,
            features,
            outputs: draft.outputs,
            view_intents: draft.views,
        })
    }
}

impl Dispatch for GeometryHost {
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
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new(
                "HostRegistryMismatch",
                "descriptor is unknown to the geometry profile",
            ));
        }
        if matches!(descriptor.module.as_str(), "units" | "param") {
            return self.parameters.call(descriptor, mode, site, arguments);
        }
        descriptor.check_mode(mode)?;
        let types = arguments
            .iter()
            .map(|value| {
                value.value_type().ok_or_else(|| {
                    Error::new("GeometryInputType", "expected a finite geometry input")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        descriptor.specialize(&types)?;
        let operation = BINDINGS
            .iter()
            .find_map(|(module, name, op)| {
                (*module == descriptor.module && *name == descriptor.name).then_some(*op)
            })
            .ok_or_else(|| Error::new("UnsupportedCapability", &descriptor.operation))?;
        match (operation, arguments) {
            (Operation::Plate, [Value::Text(key), width, height, thickness]) => self.feature(
                key,
                FeatureDraft::Plate(self.inputs([width, height, thickness], site)?),
                site,
            ),
            (Operation::ThroughHole, [Value::Text(key), base, diameter, x, y]) => self.feature(
                key,
                FeatureDraft::ThroughHole {
                    base: self.reference(base, BODY)?,
                    inputs: self.inputs([diameter, x, y], site)?,
                },
                site,
            ),
            (Operation::Output, [Value::Text(key), body]) => {
                crate::parameters::validate_key(key)?;
                let body = self.reference(body, BODY)?;
                let mut draft = self.draft.lock().map_err(|_| unavailable())?;
                if !draft.keys.contains(&body) {
                    return Err(Error::new("UnknownBody", body));
                }
                if draft.outputs.contains_key(key) {
                    return Err(Error::new("DuplicateOutput", key));
                }
                draft.outputs.insert(key.into(), body);
                Ok(Value::Unit)
            }
            (Operation::ViewBody, [body]) => self
                .parameters
                .wrap_handle(VIEW, self.reference(body, BODY)?),
            (Operation::Show, [view]) => {
                let body = self.reference(view, VIEW)?;
                let mut draft = self.draft.lock().map_err(|_| unavailable())?;
                if !draft.keys.contains(&body) {
                    return Err(Error::new("UnknownBody", body));
                }
                draft.views.push(ViewIntent {
                    body,
                    site: site.clone(),
                });
                Ok(Value::Unit)
            }
            _ => Err(Error::new(
                "GeometryInputType",
                "unexpected geometry arguments",
            )),
        }
    }
}

#[derive(Clone)]
struct PlateProfile {
    size: [f64; 3],
    holes: Vec<([f64; 2], f64)>,
}

fn resolve(
    drafts: Vec<(String, CallSite, FeatureDraft)>,
    parameters: &ParameterSnapshot,
) -> Result<Vec<Feature>, Error> {
    let mut profiles = BTreeMap::<String, PlateProfile>::new();
    let mut features = Vec::with_capacity(drafts.len());
    for (key, site, draft) in drafts {
        let (geometry, inputs, profile) = match draft {
            FeatureDraft::Plate(inputs) => {
                let size = resolve_inputs(&inputs, parameters)?;
                if size.iter().any(|value| *value <= 0.0) {
                    return Err(Error::new("InvalidPlateSize", &key));
                }
                (
                    Geometry::Plate { size_m: size },
                    ["width", "height", "thickness"]
                        .into_iter()
                        .zip(inputs)
                        .collect(),
                    PlateProfile {
                        size,
                        holes: Vec::new(),
                    },
                )
            }
            FeatureDraft::ThroughHole { base, inputs } => {
                let [diameter, x, y] = resolve_inputs(&inputs, parameters)?;
                if diameter > 1e6 {
                    return Err(Error::new("InvalidHoleDiameter", &key));
                }
                let centre = [x, y];
                let mut profile = profiles
                    .get(&base)
                    .cloned()
                    .ok_or_else(|| Error::new("UnknownBody", &base))?;
                profile.add_hole(&key, centre, diameter / 2.0)?;
                (
                    Geometry::ThroughHole {
                        base,
                        diameter_m: diameter,
                        centre_m: centre,
                        entry_z_m: profile.size[2] / 2.0,
                    },
                    ["diameter", "x", "y"].into_iter().zip(inputs).collect(),
                    profile,
                )
            }
        };
        profiles.insert(key.clone(), profile);
        features.push(Feature {
            key,
            site,
            geometry,
            inputs,
        });
    }
    Ok(features)
}

fn resolve_inputs(
    inputs: &[ParamExpr; 3],
    parameters: &ParameterSnapshot,
) -> Result<[f64; 3], Error> {
    let [first, second, third] = inputs;
    let length = |input: &ParamExpr| match input.evaluate(parameters)? {
        ParamValue::Length(value) => Ok(value),
        _ => Err(Error::new("GeometryInputType", "expected a length in SI")),
    };
    Ok([length(first)?, length(second)?, length(third)?])
}

impl PlateProfile {
    fn add_hole(&mut self, key: &str, [x, y]: [f64; 2], radius: f64) -> Result<(), Error> {
        if radius <= 0.0 {
            return Err(Error::new("InvalidHoleDiameter", key));
        }
        if self.holes.len() >= MAX_HOLES {
            return Err(Error::new(
                "GeometryHoleLimit",
                "a plate output supports up to 64 holes",
            ));
        }
        let edge =
            (self.size[0] / 2.0 - x.abs() - radius).min(self.size[1] / 2.0 - y.abs() - radius);
        if edge + CLEARANCE_TOLERANCE_M < MIN_CLEARANCE_M {
            return Err(Error::new("HoleEdgeClearance", key));
        }
        if self.holes.iter().any(|([other_x, other_y], other_radius)| {
            (x - other_x).hypot(y - other_y) - radius - other_radius + CLEARANCE_TOLERANCE_M
                < MIN_CLEARANCE_M
        }) {
            return Err(Error::new("HolePairClearance", key));
        }
        self.holes.push(([x, y], radius));
        Ok(())
    }
}

fn unavailable() -> Error {
    Error::new(
        "GeometrySessionError",
        "local geometry state is unavailable",
    )
}
