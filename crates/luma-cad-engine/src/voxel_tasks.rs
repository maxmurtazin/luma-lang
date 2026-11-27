//! Command declarations against a Core sphere and grid selected by the native host.
use crate::ParameterHost;
use luma_cad_contract::SEALED_MODULES;
use luma_host_api::{
    CallSite, Descriptor, Dispatch, Effect, Error, Mode, Parameter, Registry, ResultType, Type,
    Value,
};
use std::sync::atomic::{AtomicBool, Ordering};

const FIELD: &str = "FieldRef";
const GRID: &str = "GridSpecRef";
const OPTIONS: &str = "SampleOptionsRef";
const TASK: &str = "TaskSpecRef<Volume>";

pub struct VoxelTaskHost {
    handles: ParameterHost,
    registry: Registry,
    declared: AtomicBool,
}

impl VoxelTaskHost {
    pub fn new() -> Result<Self, Error> {
        let specs = [
            ("pinned_field", vec![], FIELD, Effect::ReadPinned),
            ("grid", vec![], GRID, Effect::Pure),
            ("sample_options", vec![], OPTIONS, Effect::Pure),
            (
                "sample",
                vec![FIELD, GRID, OPTIONS],
                TASK,
                Effect::DeclareTask,
            ),
        ];
        let registry = Registry::new(
            SEALED_MODULES.iter().map(|name| (*name).into()),
            specs
                .into_iter()
                .map(|(name, args, result, effect)| Descriptor {
                    module: "voxel".into(),
                    name: name.into(),
                    operation: format!("voxel.{name}"),
                    major: 1,
                    parameters: args
                        .into_iter()
                        .map(|arg| Parameter::Exact(Type::Opaque(arg.into())))
                        .collect(),
                    result: ResultType::Exact(Type::Opaque(result.into())),
                    effect,
                    modes: [Mode::Command].into(),
                }),
        )?;
        Ok(Self {
            handles: ParameterHost::new(Default::default())?,
            registry,
            declared: AtomicBool::new(false),
        })
    }

    pub fn finish(self) -> Result<(), Error> {
        if !self.declared.into_inner() {
            return Err(Error::new(
                "MissingVoxelTask",
                "Expected one voxel.sample declaration",
            ));
        }
        Ok(())
    }
}

impl Dispatch for VoxelTaskHost {
    fn registry(&self) -> &Registry {
        &self.registry
    }

    fn call(
        &self,
        descriptor: &Descriptor,
        mode: Mode,
        _: &CallSite,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.registry.get(&descriptor.module, &descriptor.name) != Some(descriptor) {
            return Err(Error::new("HostRegistryMismatch", &descriptor.operation));
        }
        descriptor.check_mode(mode)?;
        match (descriptor.name.as_str(), args) {
            ("pinned_field", []) => self.handles.wrap_handle(FIELD, ()),
            ("grid", []) => self.handles.wrap_handle(GRID, ()),
            ("sample_options", []) => self.handles.wrap_handle(OPTIONS, ()),
            ("sample", [field, grid, options]) => {
                self.handles.unwrap_handle::<()>(field, FIELD)?;
                self.handles.unwrap_handle::<()>(grid, GRID)?;
                self.handles.unwrap_handle::<()>(options, OPTIONS)?;
                if self.declared.swap(true, Ordering::Relaxed) {
                    return Err(Error::new(
                        "VoxelTaskLimit",
                        "Expected one voxel.sample declaration",
                    ));
                }
                self.handles.wrap_handle(TASK, ())
            }
            _ => Err(Error::new("VoxelTaskInputType", &descriptor.operation)),
        }
    }
}
