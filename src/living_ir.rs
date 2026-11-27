//! Lower the bounded, fixed rectangle sketch recipe to a Living IR HTTP proposal.
use crate::{cad, host};
use luma_cad_engine::{CommandHost, Declaration, ParamValue, Parameter, ParameterSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const UNSUPPORTED: &str = "Expected one XY sketch with four fixed corners, four axis-aligned segments, one profile and one positive extrusion";

fn id(value: &Value) -> Result<&str, &'static str> {
    value.as_str().ok_or(UNSUPPORTED)
}

fn position(value: &Value) -> Result<[f64; 2], &'static str> {
    let values = value.as_array().ok_or(UNSUPPORTED)?;
    let [x, y] = values.as_slice() else {
        return Err(UNSUPPORTED);
    };
    let (x, y) = (
        x.as_f64().ok_or(UNSUPPORTED)?,
        y.as_f64().ok_or(UNSUPPORTED)?,
    );
    if x.abs() > 1.0 || y.abs() > 1.0 {
        return Err(UNSUPPORTED);
    }
    Ok([x, y])
}

fn rectangle(plan: &luma_cad_engine::SketchPlan) -> Result<Value, &'static str> {
    let [plane, sketch, rest @ .., profile] = plan.commands.as_slice() else {
        return Err(UNSUPPORTED);
    };
    if rest.len() != 12
        || plane["type"] != "workplane.create"
        || plane["workplane"]["basis"] != "xy"
        || sketch["type"] != "sketch.create"
        || sketch["workplane_id"] != plane["workplane"]["id"]
        || profile["type"] != "profile.create"
        || profile["profile"]["source_ref"]["sketch_id"] != sketch["sketch_id"]
    {
        return Err(UNSUPPORTED);
    }
    let mut points = BTreeMap::new();
    let mut fixed = BTreeSet::new();
    let mut edges = BTreeMap::new();
    for item in rest {
        if item["sketch_id"] != sketch["sketch_id"] {
            return Err(UNSUPPORTED);
        }
        match id(&item["type"])? {
            "sketch.variables.create" => {
                let variable = &item["variables"][0];
                let point = id(&variable["id"])?;
                if points
                    .insert(point, position(&variable["initial_m"])?)
                    .is_some()
                {
                    return Err(UNSUPPORTED);
                }
            }
            "constraint.create" => {
                let relation = &item["constraint"]["relation"];
                let point = id(&relation["point_id"])?;
                if relation["kind"] != "fix_point"
                    || points.get(point) != Some(&position(&relation["position_m"])?)
                    || !fixed.insert(point)
                {
                    return Err(UNSUPPORTED);
                }
            }
            "sketch.segment.create" => {
                let segment = &item["segment"];
                let mut pair = [id(&segment["start_id"])?, id(&segment["end_id"])?];
                pair.sort();
                if !pair.iter().all(|point| points.contains_key(point))
                    || edges.insert(pair, id(&segment["id"])?).is_some()
                {
                    return Err(UNSUPPORTED);
                }
            }
            _ => return Err(UNSUPPORTED),
        }
    }
    if points.len() != 4 || fixed.len() != 4 || edges.len() != 4 {
        return Err(UNSUPPORTED);
    }
    let mut lower = [f64::INFINITY; 2];
    let mut upper = [f64::NEG_INFINITY; 2];
    for point in points.values() {
        for axis in 0..2 {
            lower[axis] = lower[axis].min(point[axis]);
            upper[axis] = upper[axis].max(point[axis]);
        }
    }
    if lower[0] >= upper[0] || lower[1] >= upper[1] {
        return Err(UNSUPPORTED);
    }
    let corners = [lower, [upper[0], lower[1]], upper, [lower[0], upper[1]]];
    let corner_ids = corners
        .iter()
        .map(|corner| {
            points
                .iter()
                .find_map(|(id, p)| (p == corner).then_some(*id))
                .ok_or(UNSUPPORTED)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut segments = Vec::new();
    for index in 0..corners.len() {
        let next = (index + 1) % corners.len();
        let mut pair = [corner_ids[index], corner_ids[next]];
        pair.sort();
        segments.push(edges.get(&pair).ok_or(UNSUPPORTED)?);
    }
    let extrusion = plan.extrusion.as_ref().ok_or(UNSUPPORTED)?;
    if extrusion.profile_id != id(&profile["profile"]["id"])? || extrusion.signed_height_m > 1.0 {
        return Err(UNSUPPORTED);
    }
    let length = extrusion.signed_height_m.to_string();
    if length.len() > 32 {
        return Err(UNSUPPORTED);
    }
    Ok(json!({
        "protocol": "luma.living-ir.recipe/1",
        "source_ids": {
            "sketch": sketch["sketch_id"],
            "profile": extrusion.profile_id,
            "points": corner_ids,
            "segments": segments,
        },
        "sketch": {"operation":"sketch_create","plane":"XY"},
        "rectangle": {"operation":"sketch_rectangle","a":{"xy":lower},"b":{"xy":upper}},
        "extrusion": {"feature_id":extrusion.feature_id,"length_m":length},
    }))
}

#[derive(Debug)]
pub enum BridgeError {
    Preparation(cad::PreparationError),
    Unsupported(&'static str),
}

pub fn prepare_recipe(bundle: &host::SourceBundle) -> Result<Value, BridgeError> {
    let planned = cad::prepare_sketch_recipe(bundle, 100_000).map_err(BridgeError::Preparation)?;
    let mut proposal = rectangle(&planned).map_err(BridgeError::Unsupported)?;
    proposal["source_digest"] = json!(bundle.source_digest());
    Ok(proposal)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandInput {
    pub source: host::WireSourceBundle,
    pub context: CommandContext,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandContext {
    pub document: String,
    pub revision: u64,
    pub epoch: u64,
    pub incarnation: String,
    pub definition: String,
    pub parameters_mm: BTreeMap<String, String>,
}

pub fn prepare_literal(
    bundle: &host::SourceBundle,
    context: CommandContext,
) -> Result<Value, BridgeError> {
    const SINGLE: &str =
        "Expected one changed Length parameter set to an integral millimeter value";
    let unsupported = || BridgeError::Unsupported(SINGLE);
    let failure = |error| BridgeError::Preparation(cad::PreparationError::Parameters(error));
    let mut parameters = BTreeMap::new();
    for (key, text) in &context.parameters_mm {
        let value = text.parse::<f64>().map_err(|_| unsupported())?;
        parameters.insert(
            key.clone(),
            Parameter {
                declaration: Declaration {
                    default: ParamValue::Length(value * 0.001),
                    bounds: None,
                    exposed: true,
                },
                override_value: None,
            },
        );
    }
    let inherited = ParameterSnapshot::new(parameters).map_err(failure)?;
    let command = CommandHost::new(context.document.clone(), inherited.clone()).map_err(failure)?;
    let report = host::run_with_host(bundle, host::Mode::Command, &command, 100_000)
        .map_err(|error| BridgeError::Preparation(cad::PreparationError::Execution(error)))?;
    let effects: Vec<_> = report
        .calls
        .iter()
        .filter(|call| matches!(call.operation.as_str(), "param.set" | "param.reset"))
        .collect();
    if !matches!(effects.as_slice(), [call] if call.operation == "param.set") {
        return Err(unsupported());
    }
    let overlay = command.finish().map_err(failure)?;
    let mut changes = overlay
        .entries()
        .iter()
        .filter(|(key, value)| value.effective() != inherited.entries()[*key].effective());
    let (key, value) = changes.next().ok_or_else(unsupported)?;
    let ParamValue::Length(meters) = value.effective() else {
        return Err(unsupported());
    };
    let millimeters = (meters * 1000.0).round();
    // The existing Luma runtime evaluates lengths in binary64. Require a
    // round-trip through the actual units.mm scale before emitting an integer.
    if changes.next().is_some() || millimeters.abs() > 1000.0 || millimeters * 0.001 != meters {
        return Err(unsupported());
    }
    Ok(json!({
        "source_digest": bundle.source_digest(),
        "scope": {
            "document": context.document, "revision": context.revision, "epoch": context.epoch,
            "incarnation": context.incarnation, "definition": context.definition
        },
        "intent": {"operation":"literal_mm","parameter":key,"decimal":format!("{millimeters:.0}")},
    }))
}
