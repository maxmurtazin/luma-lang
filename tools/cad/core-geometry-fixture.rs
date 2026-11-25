//! Opt-in integration fixture compiled by check-core-geometry.py.
use hir_definition::{Definition, Entity};
use hir_dependency::{Dependency, DependencyGraph};
use hir_domain_registry::DomainRegistry;
use hir_transaction::PreparationContext;
use hir_types::{DependencyId, DocumentId, EntityId, ModuleId};
use living_cad_features::{
    hole::{HoleEnd, HoleHead, HoleOptions},
    new_atomic_cad_store, pinned_patch,
    subtractive::CutDirection,
    FeatureRegistry, FeatureSpec, OutputPort, OutputRef,
};
use living_frep::{Point3, Pose3};
use living_symbolic::{Dimension, Node, Quantity};
use luma::{
    cad::{prepare_geometry, prepare_geometry_edits},
    cad_engine::{Geometry, ParamValue, RecipeGeometryPlan},
    host::WireSourceBundle,
};
use std::collections::BTreeMap;

fn lower(plan: &RecipeGeometryPlan, expected_x: f64) {
    let module = ModuleId::from_u128(1);
    let mut definition = Definition::new(DocumentId::from_u128(1), module);
    let mut dependencies = DependencyGraph::default();
    let mut outputs = BTreeMap::<String, OutputRef>::new();
    let mut ids = 1u128..;
    for feature in &plan.features {
        let id = EntityId::from_u128(ids.next().unwrap());
        let mut edge = |prerequisite, slot: &str| {
            let edge_id = DependencyId::from_u128(ids.next().unwrap());
            dependencies.edges.insert(
                edge_id,
                Dependency {
                    id: edge_id,
                    consumer: id,
                    prerequisite,
                    slot: slot.into(),
                },
            );
        };
        let (spec, slots) = match &feature.geometry {
            Geometry::Plate {
                size_m: [width, height, thickness],
            } => (
                FeatureSpec::plate(Pose3::default()),
                vec![
                    ("width", *width),
                    ("height", *height),
                    ("thickness", *thickness),
                ],
            ),
            Geometry::ThroughHole {
                base,
                diameter_m,
                centre_m: [x, y],
                entry_z_m,
            } => {
                let output = outputs[base];
                edge(output.feature_id, "base");
                let origin = Point3::new([*x, *y, *entry_z_m]).unwrap();
                let datum = Pose3::new(origin, Pose3::default().rotation()).unwrap();
                (
                    FeatureSpec::hole(
                        output,
                        datum,
                        HoleOptions {
                            end: HoleEnd::ThroughAll,
                            head: HoleHead::None,
                            direction: CutDirection::OppositeNormal,
                        },
                    ),
                    vec![("diameter", *diameter_m)],
                )
            }
        };
        definition.entities.insert(
            id,
            Entity {
                id,
                module,
                payload: spec.payload().unwrap(),
            },
        );
        for (slot, si) in slots {
            // Complete all scalar prerequisites in the same DefinitionStore snapshot.
            let parameter_id = EntityId::from_u128(u128::MAX - definition.entities.len() as u128);
            let node = Node::Parameter(Quantity {
                si,
                dimension: Dimension::LENGTH,
            });
            definition.entities.insert(
                parameter_id,
                Entity {
                    id: parameter_id,
                    module,
                    payload: node.payload().unwrap(),
                },
            );
            edge(parameter_id, slot);
        }
        outputs.insert(
            feature.key.clone(),
            OutputRef {
                feature_id: id,
                port: OutputPort::Solid,
            },
        );
    }

    let mut registry = DomainRegistry::default();
    living_symbolic::register_domain(&mut registry).unwrap();
    living_frep::register_domain(&mut registry).unwrap();
    FeatureRegistry::register(&mut registry).unwrap();
    let store = new_atomic_cad_store(
        definition,
        dependencies,
        registry,
        &PreparationContext::default(),
    )
    .unwrap();
    let patch = pinned_patch(store.snapshot().unwrap(), outputs[&plan.outputs["solid"]]).unwrap();
    let sample = |x, y, z| {
        patch
            .patch()
            .solid
            .sample_at(Point3::new([x, y, z]).unwrap())
            .unwrap()
            .value_m
            .unwrap()
    };
    for x in [-expected_x, expected_x] {
        for z in [-0.0039, 0.0, 0.0039] {
            assert!(sample(x, 0.0, z) > 0.0, "hole must cut through the plate");
            assert!(
                sample(x, 0.0032, z) < 0.0,
                "stock must remain outside the bore"
            );
        }
    }
    if expected_x > 0.04 {
        assert!(
            sample(0.055, 0.015, 0.0) < 0.0,
            "stock reaches the new width"
        );
        assert!(
            sample(0.04, 0.0, 0.0) < 0.0,
            "old bore must be absent after override"
        );
    } else {
        assert!(sample(0.055, 0.015, 0.0) > 0.0, "stock ends at 50 mm");
    }
    assert!(sample(0.0, 0.0, 0.0) < 0.0);
}

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("expected the control recipe path");
    let bundle = serde_json::from_slice::<WireSourceBundle>(&std::fs::read(path).unwrap())
        .unwrap()
        .admit()
        .unwrap();
    let base = prepare_geometry(&bundle, &Default::default(), 1000).unwrap();
    let updated = prepare_geometry_edits(
        &bundle,
        &base.parameters.parameters,
        [("width", Some(ParamValue::Length(0.12)))],
        1000,
    )
    .unwrap();
    lower(&base, 0.04);
    lower(&updated, 0.05);
    println!("PASS Core D1 plate and through-hole chain at 100 mm and 120 mm");
}
