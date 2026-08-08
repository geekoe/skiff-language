pub(crate) use std::{collections::BTreeMap, sync::Arc};

pub(crate) use crate::error::{Error as RuntimeError, Result};
pub(crate) use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkOverlay, LinkedFileUnit, LinkedNamedUnionBranch,
    LinkedNominalTypeRefBase, LinkedProgramImage, LinkedTypeDescriptor, LinkedTypeRef, LiteralIr,
    PackageRefIr, PackageSymbolRef, ResolvedSymbol, RuntimeExecutionPackage, RuntimeTypeContext,
    ServiceSymbolRef, TypeAddr, TypeDeclIr, UnitAddr,
};
pub(crate) use skiff_runtime_model::recoverable::{
    RuntimeRecoverableExpectedAnyInterfacePlan, RuntimeRecoverableExpectedRecordFieldPlan,
    RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan,
    RuntimeRecoverableInterfaceTypeRef, RuntimeRecoverableTypeIdentityRef,
};

pub use skiff_runtime_boundary::type_descriptor::{bare_type_name, type_name_root};
pub use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
};

mod address;
mod builtins;
mod context;
mod labels;
mod linked;
mod nominal;
mod recoverable;
mod recoverable_behavior;
#[cfg(test)]
mod tests;

pub(crate) use address::{
    is_actor_declaration_symbol, program_db_object_type_addr, program_package_type_addr,
    program_publication_type_addr, program_service_symbol_type_addr,
};
pub(crate) use builtins::native_builtin_plan;
pub(crate) use builtins::{db_result_node, structural_builtin_node, PlanInput};
pub use context::{PlanContext, ProgramTypeView};
pub(crate) use labels::{
    linked_type_descriptor_label, linked_type_ref_kind, linked_type_ref_label,
    linked_type_ref_named_type_name, unknown_plan_for_descriptor, unknown_plan_for_type_ref,
};
pub(crate) use nominal::{
    applied_nominal_plan, apply_nominal_owner_context, close_linked_type_ref,
    linked_named_union_branch_plan,
};
#[cfg(test)]
pub(crate) use recoverable::sorted_json_string;
pub use recoverable::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    recoverable_interface_projection_identity,
};
pub use recoverable_behavior::{
    build_recoverable_behavior_index, interface_method_table_from_linked,
    method_tables_runtime_equivalent, runtime_interface_method_table_id,
};
#[cfg(test)]
pub(crate) use tests::test_runtime_package;

pub trait RuntimeTypePlanLinkedExt: Sized {
    fn from_artifact_type_ref(type_ref: &skiff_artifact_model::TypeRefIr) -> Result<Self>;

    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_nested_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;
}

pub trait RuntimeRecoverableExpectedTypePlanLinkedExt: Sized {
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;
}
