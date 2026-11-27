//! Typed declarations against one body captured by the native caller.
use crate::{sketch::length_m, Kind, ParameterHost};
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum EngineeringQuery {
    Volume,
    BoundaryDistance { point_m: [f64; 3] },
}

#[derive(Debug)]
pub enum BrepTask {
    Mesh,
    Section { z_m: f64 },
    Query(EngineeringQuery),
}

pub struct BrepTaskHost {
    handles: ParameterHost,
    registry: Registry,
    tasks: Mutex<Vec<BrepTask>>,
}

impl BrepTaskHost {
    pub fn new() -> Result<Self, Error> {
        let opaque = |name: &str| Type::Opaque(name.into());
        let length = Kind::Length.value_type();
        let body = opaque("BodyRef");
        let options = opaque("BrepTaskOptions");
        let plane = opaque("ParallelPlane");
        let query = opaque("BrepQuery");
        let specs = [
            (
                "brep",
                "published_body",
                vec![],
                body.clone(),
                Effect::ReadPinned,
            ),
            (
                "brep",
                "task_options",
                vec![],
                options.clone(),
                Effect::Pure,
            ),
            ("brep", "volume", vec![], query.clone(), Effect::Pure),
            (
                "brep",
                "distance",
                vec![length.clone(); 3],
                query.clone(),
                Effect::Pure,
            ),
            (
                "brep",
                "xy_section",
                vec![length],
                plane.clone(),
                Effect::Pure,
            ),
            (
                "cad",
                "mesh",
                vec![body.clone(), options.clone()],
                opaque("TaskSpecRef<Mesh>"),
                Effect::DeclareTask,
            ),
            (
                "cad",
                "section_task",
                vec![body.clone(), plane, options.clone()],
                opaque("TaskSpecRef<Section>"),
                Effect::DeclareTask,
            ),
            (
                "cad",
                "query_task",
                vec![body, query, options],
                opaque("TaskSpecRef<QueryResult>"),
                Effect::DeclareTask,
            ),
        ];
        let handles = ParameterHost::new(Default::default())?;
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            handles
                .registry()
                .descriptors()
                .filter(|item| item.module == "units")
                .cloned()
                .chain(
                    specs
                        .into_iter()
                        .map(|(module, name, parameters, result, effect)| Descriptor {
                            module: module.into(),
                            name: name.into(),
                            operation: format!("{module}.{name}"),
                            major: 1,
                            parameters: parameters.into_iter().map(Parameter::Exact).collect(),
                            result: ResultType::Exact(result),
                            effect,
                            modes: [Mode::Command].into(),
                        }),
                ),
        )?;
        Ok(Self {
            handles,
            registry,
            tasks: Mutex::default(),
        })
    }

    pub fn finish(self) -> Result<Vec<BrepTask>, Error> {
        let tasks = self.tasks.into_inner().map_err(|_| poisoned())?;
        if tasks.is_empty() {
            return Err(Error::new(
                "EmptyBrepTaskPlan",
                "Expected a B-rep task declaration",
            ));
        }
        Ok(tasks)
    }

    fn declare(
        &self,
        body: &Value,
        options: &Value,
        task: BrepTask,
        kind: &str,
    ) -> Result<Value, Error> {
        self.handles.unwrap_handle::<()>(body, "BodyRef")?;
        self.handles
            .unwrap_handle::<()>(options, "BrepTaskOptions")?;
        let mut tasks = self.tasks.lock().map_err(|_| poisoned())?;
        if tasks.len() >= 8
            || (matches!(task, BrepTask::Mesh)
                && tasks.iter().any(|item| matches!(item, BrepTask::Mesh)))
        {
            return Err(Error::new(
                "BrepTaskLimit",
                "Expected at most eight tasks and one mesh",
            ));
        }
        let handle = self.handles.wrap_handle(kind, tasks.len())?;
        tasks.push(task);
        Ok(handle)
    }
}

impl Dispatch for BrepTaskHost {
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
        match (descriptor.operation.as_str(), arguments) {
            ("brep.published_body", []) => self.handles.wrap_handle("BodyRef", ()),
            ("brep.task_options", []) => self.handles.wrap_handle("BrepTaskOptions", ()),
            ("brep.volume", []) => self
                .handles
                .wrap_handle("BrepQuery", EngineeringQuery::Volume),
            ("brep.distance", [x, y, z]) => self.handles.wrap_handle(
                "BrepQuery",
                EngineeringQuery::BoundaryDistance {
                    point_m: [length_m(x)?, length_m(y)?, length_m(z)?],
                },
            ),
            ("brep.xy_section", [z]) => self.handles.wrap_handle("ParallelPlane", length_m(z)?),
            ("cad.mesh", [body, options]) => {
                self.declare(body, options, BrepTask::Mesh, "TaskSpecRef<Mesh>")
            }
            ("cad.section_task", [body, plane, options]) => self.declare(
                body,
                options,
                BrepTask::Section {
                    z_m: self.handles.unwrap_handle(plane, "ParallelPlane")?,
                },
                "TaskSpecRef<Section>",
            ),
            ("cad.query_task", [body, query, options]) => self.declare(
                body,
                options,
                BrepTask::Query(self.handles.unwrap_handle(query, "BrepQuery")?),
                "TaskSpecRef<QueryResult>",
            ),
            _ if descriptor.module == "units" => {
                self.handles.call(descriptor, mode, site, arguments)
            }
            _ => Err(Error::new("BrepTaskInputType", &descriptor.operation)),
        }
    }
}

fn poisoned() -> Error {
    Error::new(
        "BrepTaskPlanUnavailable",
        "Cannot access the B-rep task plan",
    )
}
