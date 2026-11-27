//! Provider-independent CAD planning.
mod brep_tasks;
mod command;
mod dependencies;
mod expression;
mod geometry;
mod ownership;
mod parameters;
mod planar;
mod recipe;
mod sketch;
mod voxel_tasks;

pub use brep_tasks::{BrepTask, BrepTaskHost, EngineeringQuery};
pub use command::CommandHost;
pub use dependencies::{
    AdmittedRecipe, AdmittedRecipes, BindingRevision, ClosureError, PreparedRecipe,
    PreparedRecipes, RecipeBinding, RecipeOutputs,
};
pub use expression::{ExprNode, ExprOp, ParamExpr};
pub use geometry::{
    Feature, Geometry, GeometryDefinition, GeometryHost, RecipeGeometryPlan, ViewIntent,
    GEOMETRY_PROFILE,
};
pub use ownership::{
    Definition, DetachedRegion, OwnedKey, OwnedRecord, OwnedRegion, Ownership, Provenance,
    ReconciledRegion, Reconciliation,
};
pub use parameters::{Declaration, Kind, ParamValue, Parameter, ParameterSnapshot};
pub use planar::{Operand as PlanarOperand, PlanarHost, PlanarOperation, PlanarPlan};
pub use recipe::{ParameterHost, ParameterPlan};
pub use sketch::{ProfileExtrusion, SketchHost, SketchPlan};
pub use voxel_tasks::VoxelTaskHost;
