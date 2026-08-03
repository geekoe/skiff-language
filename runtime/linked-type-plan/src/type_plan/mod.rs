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
#[cfg(test)]
mod tests;

pub(crate) use address::{
    is_actor_declaration_symbol, program_db_object_type_addr, program_package_type_addr,
    program_publication_type_addr, program_service_symbol_type_addr,
};
pub(crate) use builtins::native_builtin_plan;
pub(crate) use builtins::RuntimeBuiltinShape;
pub(crate) use builtins::{db_result_node, structural_builtin_node, PlanInput};
pub use context::{PlanContext, ProgramTypeView};
pub(crate) use labels::{
    artifact_type_ref_label, artifact_type_ref_named_type_name, linked_type_descriptor_label,
    linked_type_ref_kind, linked_type_ref_label, linked_type_ref_named_type_name,
    unknown_plan_for_descriptor, unknown_plan_for_type_ref,
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
#[cfg(test)]
pub(crate) use tests::test_runtime_package;

pub trait RuntimeTypePlanLinkedExt: Sized {
    fn from_artifact_type_ref(type_ref: &skiff_artifact_model::TypeRefIr) -> Result<Self>;

    fn from_artifact_type_ref_in_program(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: &LinkedProgramImage,
        current_addr: &ExecutableAddr,
    ) -> Result<Self>;

    fn from_artifact_type_ref_in_type_view(
        type_ref: &skiff_artifact_model::TypeRefIr,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
    ) -> Result<Self>;

    fn from_artifact_type_ref_in_program_ref(
        type_ref: &skiff_artifact_model::TypeRefIr,
        ctx: &PlanContext<'_>,
    ) -> Result<Self>;

    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_nested_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_substituted(bound: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn resolve_addr_or_bridge(
        type_ref: &LinkedTypeRef,
        addr: TypeAddr,
        ctx: &PlanContext,
    ) -> Result<Self>;

    fn from_linked_declaration(declaration: &TypeDeclIr, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_descriptor(descriptor: &LinkedTypeDescriptor, ctx: &PlanContext)
        -> Result<Self>;

    fn builtin_node(
        name: &str,
        args: &[LinkedTypeRef],
        ctx: &PlanContext,
    ) -> Result<RuntimeTypeNode>;

    fn artifact_builtin_node(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
    ) -> Result<RuntimeTypeNode>;

    fn artifact_builtin_node_in_program(
        name: &str,
        args: &[skiff_artifact_model::TypeRefIr],
        ctx: &PlanContext<'_>,
    ) -> Result<RuntimeTypeNode>;
}

pub trait RuntimeRecoverableExpectedTypePlanLinkedExt: Sized {
    fn from_linked(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;

    fn from_linked_ref(type_ref: &LinkedTypeRef, ctx: &PlanContext) -> Result<Self>;
}
