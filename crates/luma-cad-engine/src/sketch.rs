//! Lowering for the registered Living IR 2D document.
use crate::{Kind, ParamValue, ParameterHost, ParameterSnapshot};
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

const SKETCH: &str = "SketchRef";
const POINT: &str = "SketchPointRef";

pub struct SketchHost {
    mode: Mode,
    handles: ParameterHost,
    registry: Registry,
    plan: Mutex<Plan>,
}

#[derive(Default)]
struct Plan {
    ids: BTreeSet<String>,
    commands: Vec<Json>,
    extrusion: Option<ProfileExtrusion>,
    output: bool,
}

#[derive(Debug, Serialize)]
pub struct ProfileExtrusion {
    pub feature_id: String,
    pub profile_id: String,
    pub signed_height_m: f64,
}

#[derive(Debug)]
pub struct SketchPlan {
    pub commands: Vec<Json>,
    pub extrusion: Option<ProfileExtrusion>,
    pub parameters: ParameterSnapshot,
}

fn opaque(name: &str) -> Type {
    Type::Opaque(name.into())
}

impl SketchHost {
    pub fn new() -> Result<Self, Error> {
        Self::for_mode(Mode::Command)
    }

    pub fn for_mode(mode: Mode) -> Result<Self, Error> {
        Self::with_parameters(mode, ParameterSnapshot::default())
    }

    pub fn for_recipe(inherited: ParameterSnapshot) -> Result<Self, Error> {
        Self::with_parameters(Mode::Recipe, inherited)
    }

    fn with_parameters(mode: Mode, inherited: ParameterSnapshot) -> Result<Self, Error> {
        let handles = ParameterHost::new(inherited)?;
        let length = Kind::Length.value_type();
        let specs = [
            ("sketch", "create", vec![Type::Text; 3], opaque(SKETCH)),
            (
                "sketch",
                "point",
                vec![opaque(SKETCH), Type::Text, length.clone(), length.clone()],
                opaque(POINT),
            ),
            (
                "sketch",
                "fix",
                vec![
                    opaque(SKETCH),
                    Type::Text,
                    opaque(POINT),
                    length.clone(),
                    length.clone(),
                ],
                Type::Unit,
            ),
            (
                "sketch",
                "line",
                vec![opaque(SKETCH), Type::Text, opaque(POINT), opaque(POINT)],
                opaque("SketchCurveRef"),
            ),
            (
                "sketch",
                "circle",
                vec![opaque(SKETCH), Type::Text, opaque(POINT), opaque(POINT)],
                opaque("SketchCurveRef"),
            ),
            (
                "sketch",
                "profile",
                vec![opaque(SKETCH)],
                opaque("ProfileRef"),
            ),
            ("draft", "create", vec![Type::Text], opaque("DraftRef")),
            (
                "draft",
                "layer",
                vec![opaque("DraftRef"), Type::Text],
                opaque("DraftLayerRef"),
            ),
            (
                "draft",
                "point",
                vec![
                    opaque("DraftLayerRef"),
                    Type::Text,
                    length.clone(),
                    length.clone(),
                ],
                opaque("DraftPointRef"),
            ),
            (
                "draft",
                "add",
                vec![
                    opaque("DraftLayerRef"),
                    Type::Text,
                    opaque("DraftPointRef"),
                    opaque("DraftPointRef"),
                ],
                opaque("DraftEntityRef"),
            ),
            (
                "draft",
                "transform",
                vec![opaque("DraftEntityRef"), length.clone(), length.clone()],
                Type::Unit,
            ),
            (
                "brep",
                "extrude",
                vec![Type::Text, opaque("ProfileRef"), length],
                opaque("BodyRef"),
            ),
            (
                "cad",
                "output",
                vec![Type::Text, opaque("BodyRef")],
                Type::Unit,
            ),
        ];
        let descriptors = specs
            .into_iter()
            .filter(|(module, _, _, _)| match mode {
                Mode::Recipe => *module != "draft",
                Mode::Command => *module != "cad",
            })
            .map(|(module, name, parameters, result)| Descriptor {
                module: module.into(),
                name: name.into(),
                operation: format!("{module}.{name}"),
                major: 1,
                parameters: parameters.into_iter().map(Parameter::Exact).collect(),
                result: ResultType::Exact(result),
                effect: Effect::BuildPlan,
                modes: [mode].into(),
            });
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            handles
                .registry()
                .descriptors()
                .filter(|item| {
                    item.module == "units"
                        || (mode == Mode::Recipe
                            && matches!(item.operation.as_str(), "param.length" | "param.value"))
                })
                .cloned()
                .chain(descriptors),
        )?;
        Ok(Self {
            mode,
            handles,
            registry,
            plan: Mutex::default(),
        })
    }

    pub fn finish(self) -> Result<SketchPlan, Error> {
        let plan = self.plan.into_inner().map_err(|_| poisoned())?;
        if plan.commands.is_empty() {
            return Err(Error::new(
                "EmptySketchPlan",
                "Expected at least one 2D command",
            ));
        }
        if self.mode == Mode::Recipe && !plan.output {
            return Err(Error::new(
                "MissingRecipeOutput",
                "Publish the recipe extrusion with cad.output(\"solid\", body)",
            ));
        }
        Ok(SketchPlan {
            commands: plan.commands,
            extrusion: plan.extrusion,
            parameters: self.handles.finish(&BTreeMap::new())?.parameters,
        })
    }

    fn add(&self, ids: &[&str], commands: impl IntoIterator<Item = Json>) -> Result<(), Error> {
        let mut plan = self.plan.lock().map_err(|_| poisoned())?;
        for id in ids {
            if !valid_id(id) {
                return Err(Error::new("InvalidSketchId", "Expected a lowercase UUID"));
            }
            if !plan.ids.insert((*id).into()) {
                return Err(Error::new("DuplicateSketchId", *id));
            }
        }
        for command in commands {
            if plan.commands.len() == 256 {
                return Err(Error::new("HostPlanLimit", "2D plan exceeds 256 commands"));
            }
            plan.commands.push(command);
        }
        Ok(())
    }

    fn point(&self, value: &Value, sketch: &str) -> Result<String, Error> {
        let (owner, id): (String, String) = self.handles.unwrap_handle(value, POINT)?;
        if owner != sketch {
            return Err(Error::new(
                "ForeignSketchPoint",
                "Point belongs to another sketch",
            ));
        }
        Ok(id)
    }
}

impl Dispatch for SketchHost {
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
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        if matches!(descriptor.module.as_str(), "units" | "param") {
            return self.handles.call(descriptor, mode, site, arguments);
        }
        match (descriptor.operation.as_str(), arguments) {
            ("sketch.create", [Value::Text(sketch), Value::Text(plane), Value::Text(profile)]) => {
                if !valid_id(profile) {
                    return Err(Error::new(
                        "InvalidSketchId",
                        "Expected a lowercase Profile UUID",
                    ));
                }
                self.add(
                    &[plane, sketch],
                    [
                        json!({"type":"workplane.create","workplane":{"id":plane,"basis":"xy"}}),
                        json!({"type":"sketch.create","sketch_id":sketch,"workplane_id":plane,
                        "name":"Sketch","solver_policy":"preserve_branch"}),
                    ],
                )?;
                self.handles.wrap_handle(SKETCH, (sketch, profile))
            }
            ("sketch.point", [sketch, Value::Text(id), x, y]) => {
                let (sketch, _): (String, String) = self.handles.unwrap_handle(sketch, SKETCH)?;
                self.add(
                    &[id],
                    [json!({"type":"sketch.variables.create","sketch_id":sketch,
                    "variables":[{"id":id,"initial_m":[length_m(x)?,length_m(y)?]}]})],
                )?;
                self.handles.wrap_handle(POINT, (sketch, id))
            }
            ("sketch.fix", [sketch, Value::Text(id), point, x, y]) => {
                let (sketch, _): (String, String) = self.handles.unwrap_handle(sketch, SKETCH)?;
                let point = self.point(point, &sketch)?;
                self.add(
                    &[id],
                    [json!({"type":"constraint.create","sketch_id":sketch,
                    "constraint":{"id":id,"enabled":true,"relation":{"kind":"fix_point",
                        "point_id":point,"position_m":[length_m(x)?,length_m(y)?]}}})],
                )?;
                Ok(Value::Unit)
            }
            (
                operation @ ("sketch.line" | "sketch.circle"),
                [sketch, Value::Text(id), first, second],
            ) => {
                let (sketch, _): (String, String) = self.handles.unwrap_handle(sketch, SKETCH)?;
                let first = self.point(first, &sketch)?;
                let second = self.point(second, &sketch)?;
                let command = if operation == "sketch.line" {
                    json!({"type":"sketch.segment.create","sketch_id":sketch,
                        "segment":{"id":id,"start_id":first,"end_id":second}})
                } else {
                    json!({"type":"sketch.curve.create","sketch_id":sketch,
                        "curve":{"id":id,"geometry":{"kind":"circle","center_id":first,"rim_id":second}}})
                };
                self.add(&[id], [command])?;
                self.handles.wrap_handle("SketchCurveRef", (sketch, id))
            }
            ("sketch.profile", [sketch]) => {
                let (sketch, id): (String, String) = self.handles.unwrap_handle(sketch, SKETCH)?;
                self.add(&[&id], [json!({"type":"profile.create","profile":{
                    "id":id,"source_ref":{"kind":"sketch","sketch_id":sketch},
                    "selection":{"kind":"all_non_construction"},"workplane":{"kind":"inherit_source"},
                    "purpose":"solid_profile","multi_region_policy":"reject",
                    "acceptance_policy":"require_fully_constrained","numeric_profile_id":"mechanical-si-v1"
                }})])?;
                self.handles.wrap_handle("ProfileRef", id)
            }
            ("draft.create", [Value::Text(id)]) => {
                self.add(
                    &[id],
                    [json!({"type":"draft.create","draft_id":id,"name":"Draft"})],
                )?;
                self.handles.wrap_handle("DraftRef", id)
            }
            ("draft.layer", [draft, Value::Text(id)]) => {
                let draft: String = self.handles.unwrap_handle(draft, "DraftRef")?;
                self.add(
                    &[id],
                    [json!({"type":"layer.create","owner_id":draft,"layer":{
                        "id":id,"name":"Geometry","visible":true,"locked":false,
                        "default_style_ref":null,"export_policy":"include","order":0
                    }})],
                )?;
                self.handles.wrap_handle("DraftLayerRef", (draft, id))
            }
            ("draft.point", [layer, Value::Text(id), x, y]) => {
                let (draft, layer): (String, String) =
                    self.handles.unwrap_handle(layer, "DraftLayerRef")?;
                let geometry = json!({"type":"point2","x_m":length_m(x)?,"y_m":length_m(y)?});
                self.add(&[id], [draft_entity(&draft, &layer, id, geometry)])?;
                self.handles.wrap_handle("DraftPointRef", (draft, id))
            }
            ("draft.add", [layer, Value::Text(id), start, end]) => {
                let (draft, layer): (String, String) =
                    self.handles.unwrap_handle(layer, "DraftLayerRef")?;
                let (start_draft, start): (String, String) =
                    self.handles.unwrap_handle(start, "DraftPointRef")?;
                let (end_draft, end): (String, String) =
                    self.handles.unwrap_handle(end, "DraftPointRef")?;
                if start_draft != draft || end_draft != draft {
                    return Err(Error::new(
                        "ForeignDraftPoint",
                        "Segment points must belong to the Draft",
                    ));
                }
                let geometry = json!({"type":"segment2","start_id":start,"end_id":end});
                self.add(&[id], [draft_entity(&draft, &layer, id, geometry)])?;
                self.handles.wrap_handle("DraftEntityRef", id)
            }
            ("draft.transform", [entity, dx, dy]) => {
                let id: String = self.handles.unwrap_handle(entity, "DraftEntityRef")?;
                self.add(
                    &[],
                    [json!({"type":"entity.transform","entity_ids":[id],
                        "transform":{"kind":"translate","delta_m":[length_m(dx)?,length_m(dy)?]},
                        "copy_policy":{"kind":"in_place"}
                    })],
                )?;
                Ok(Value::Unit)
            }
            ("brep.extrude", [Value::Text(feature_id), profile, height]) => {
                let profile_id = self.handles.unwrap_handle(profile, "ProfileRef")?;
                let signed_height_m = length_m(height)?;
                if signed_height_m <= 0.0 {
                    return Err(Error::new(
                        "UnsupportedExtrusionHeight",
                        "Core normal extrusion requires positive height",
                    ));
                }
                self.add(&[feature_id], [])?;
                let mut plan = self.plan.lock().map_err(|_| poisoned())?;
                if plan.extrusion.is_some() {
                    return Err(Error::new(
                        "ExtrusionPlanLimit",
                        "Expected one normal extrusion per Part",
                    ));
                }
                plan.extrusion = Some(ProfileExtrusion {
                    feature_id: feature_id.clone(),
                    profile_id,
                    signed_height_m,
                });
                self.handles.wrap_handle("BodyRef", feature_id)
            }
            ("cad.output", [Value::Text(key), body]) => {
                let feature: String = self.handles.unwrap_handle(body, "BodyRef")?;
                let mut plan = self.plan.lock().map_err(|_| poisoned())?;
                if key != "solid"
                    || plan.output
                    || !plan
                        .extrusion
                        .as_ref()
                        .is_some_and(|intent| intent.feature_id == feature)
                {
                    return Err(Error::new(
                        "InvalidRecipeOutput",
                        "Expected one solid output from the recipe extrusion",
                    ));
                }
                plan.output = true;
                Ok(Value::Unit)
            }
            _ => Err(Error::new("SketchInputType", &descriptor.operation)),
        }
    }
}

fn draft_entity(draft: &str, layer: &str, id: &str, geometry: Json) -> Json {
    json!({"type":"entity.create","owner_id":draft,"entity":{
        "id":id,"layer_id":layer,"kind":geometry,"entity_role":"geometry",
        "construction":false,"style_ref":null
    }})
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, value)| {
            if [8, 13, 18, 23].contains(&index) {
                value == b'-'
            } else {
                value.is_ascii_digit() || (b'a'..=b'f').contains(&value)
            }
        })
}

pub(crate) fn length_m(value: &Value) -> Result<f64, Error> {
    let ParamValue::Length(metres) = Kind::Length.parse(value)? else {
        return Err(Error::new("SketchInputType", "Expected Length"));
    };
    Ok(metres)
}

fn poisoned() -> Error {
    Error::new("SketchPlanUnavailable", "Cannot access 2D plan")
}
