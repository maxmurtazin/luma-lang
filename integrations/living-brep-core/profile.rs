//! Canonical wire projection of a pinned Core research sketch.
use crate::{Failure, Result};
use hir_evaluation_api::Cancellation;
use living_cad_features::{derived_id, extrude};
use living_cad_viewer::checkpoint::project::PartExtrusion;
use living_sketch::{
    research_example_limits, research_example_policy, validate_sketch, Curve2, SketchError,
};
use serde_json::{json, Value};

pub(crate) fn sketch_profile(source: &PartExtrusion) -> Result<Value> {
    let (snapshot, sketch_id, _) = source.input();
    let record = snapshot
        .definition
        .entities
        .get(&sketch_id)
        .filter(|record| {
            record.payload.schema == extrude::source_schema() && record.payload.required.is_empty()
        })
        .ok_or(Failure("UnsupportedSketchSource", Value::Null))?;
    let raw = std::str::from_utf8(&record.payload.bytes)
        .map_err(|_| Failure("InvalidSketchSource", Value::Null))?;
    let sketch = extrude::model(raw)
        .map_err(|error| Failure("InvalidSketchSource", json!(format!("{error:?}"))))?;
    if sketch.identity() != sketch_id {
        return Err(Failure("InvalidSketchSource", Value::Null));
    }
    let policy = research_example_policy();
    let validated = validate_sketch(
        &sketch,
        policy,
        research_example_limits(),
        &Cancellation::default(),
    )
    .map_err(profile_error)?;
    let profile = validated.profile();
    if profile
        .loops
        .iter()
        .filter(|loop_| loop_.nesting_depth == 0)
        .count()
        != 1
        || profile.loops.iter().any(|loop_| loop_.nesting_depth > 1)
    {
        return Err(Failure("UnsupportedProfileTopology", Value::Null));
    }
    let plane = sketch.plane();
    let [x, y, z] = plane.basis().map(|v| decimals(v.xyz()));
    let frame = json!({"origin_m": decimals(plane.origin().xyz()), "x": x, "y": y, "z": z});
    let mut curves = Vec::new();
    for &id in sketch.elements().keys() {
        if let Some(curve) = sketch.curve(id, policy).map_err(profile_error)? {
            let value = match curve {
                Curve2::Segment(segment) => json!({
                    "id":id.to_string(), "kind":"segment",
                    "start":decimals(segment.start().xy()), "end":decimals(segment.end().xy())
                }),
                Curve2::Circle(circle) => json!({
                    "id":id.to_string(), "kind":"circle",
                    "center":decimals(circle.center().xy()), "radius":decimal(circle.radius())
                }),
                Curve2::Arc(arc) => json!({
                    "id":id.to_string(), "kind":"arc",
                    "center":decimals(arc.circle().center().xy()), "radius":decimal(arc.circle().radius()),
                    "start_rad":decimal(arc.start_angle()), "sweep_rad":decimal(arc.sweep())
                }),
            };
            curves.push(value);
        }
    }
    let ids: Vec<_> = profile
        .loops
        .iter()
        .map(|loop_| {
            let anchor = loop_
                .edges
                .iter()
                .map(|edge| edge.id)
                .min()
                .ok_or(Failure("InvalidProfile", Value::Null))?;
            Ok(derived_id(sketch_id, &format!("luma-profile-loop/{anchor}")).to_string())
        })
        .collect::<Result<_>>()?;
    let loops: Vec<_> = profile
        .loops
        .iter()
        .zip(&ids)
        .map(|(loop_, id)| {
            let mut value = json!({
                "id":id,"role":if loop_.is_hole() { "hole" } else { "outer" },
                "edges":loop_.edges.iter().map(|edge| json!({
                    "curve_id":edge.id.to_string(),
                    "direction":if edge.reversed { "reverse" } else { "forward" }
                })).collect::<Vec<_>>()
            });
            if let Some(parent) = loop_.parent {
                value["parent_id"] = json!(ids[parent]);
            }
            value
        })
        .collect();
    let head = source.head();
    Ok(json!({
        "sketch":{
            "kind":"Sketch","schema_version":1,"length_unit":"m","angular_unit":"rad",
            "frame":frame,"id":sketch_id.to_string(),"curves":curves,"constraints":[],
        },
        "profile":{
            "kind":"Profile","schema_version":1,"length_unit":"m","angular_unit":"rad",
            "frame":frame,"loops":loops,"tolerance_m":decimal(policy.linear_tolerance_m),"minimum_web_m":"0",
            "source":{
                "type":"Sketch","entity_id":sketch_id.to_string(),"revision":head.native_revision.to_string(),
                "target":{"kind":"core_project","authority_id":"living-ir-core",
                    "project_id":head.project_id,"document_id":head.document_id}
            },
        },
        "source_definition_hash":head.definition_hash,
        "validation":{"provider":"living-sketch","material_area_m2":profile.material_area_m2,
            "area_uncertainty_m2":profile.area_uncertainty_m2, "operations":profile.operations},
    }))
}

fn profile_error(error: SketchError) -> Failure {
    let code = match error {
        SketchError::Cancelled => "Cancelled",
        SketchError::ResourceLimit(_) => "ResourceLimit",
        _ => "InvalidProfile",
    };
    Failure(code, json!(format!("{error:?}")))
}

fn decimals<const N: usize>(values: [f64; N]) -> [String; N] {
    values.map(decimal)
}

fn decimal(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}
