pub mod callable_return_types;
mod contract_dependency_operation_index;
mod db_lowering;
mod declaration_lowering;
pub mod entrypoint_abi;
pub mod entrypoint_abi_model;
mod executable_declaration_lowering;
mod external_refs;
pub mod file_ir;
mod function_lowering;
pub mod lowered;
mod publication_local_refs;
mod service_call_error;
mod service_call_lowering;
pub mod source_file_lowering;
pub mod source_metadata;
mod source_unit_lowering;
pub mod storage_projection;
mod suspend_analysis;
mod type_inference;
mod type_lowering;

pub use contract_dependency_operation_index::{
    ContractDependencyOperationIndex, ContractDependencyOperationIndexEntry,
};
pub use entrypoint_abi::{
    package_entrypoint_function_signature, package_public_schema_abi_types_for_module,
    package_public_schema_type_names_for_module, EntrypointAbiIndex,
};
pub use entrypoint_abi_model::{
    type_ref_ir_source_text_with_local_types, EntryFunctionSignature, EntryParamSpec,
    EntryTypeSpec, PackageAbiType, PackageAbiTypeDescriptor,
};
pub use lowered::{
    LoweredPackage, SyntheticEntrypointExecutableKind, SyntheticEntrypointIndex,
    SyntheticEntrypointModule, SyntheticOperationIndex,
};
pub use service_call_error::ServiceCallLoweringError;
pub use service_call_lowering::{lower_service_calls, LoweredServiceCallSite, LoweredServiceCalls};
pub use source_metadata::CompiledPackageSource;
pub use storage_projection::{
    service_spawn_targets_with_packages, CompiledPackageStorageProjection, PackageSpawnTargetSource,
};

use skiff_compiler_source::{PackageSourceModel, SourceCompileError};

pub fn lower(model: &PackageSourceModel) -> Result<LoweredPackage, SourceCompileError> {
    lower_with_contract_operations(model, &ContractDependencyOperationIndex::default())
}

/// T05 facade entry for canonical package compilation. Contract operations
/// come from compiler/input's validated ServiceContract-only index; no provider
/// artifact is accepted by this boundary.
pub fn lower_with_contract_operations(
    model: &PackageSourceModel,
    contract_operations: &ContractDependencyOperationIndex,
) -> Result<LoweredPackage, SourceCompileError> {
    let service_calls = lower_service_calls(model.resolved_call_targets(), contract_operations)
        .map_err(|error| SourceCompileError::ContractValidation {
            message: format!("service call lowering failed: {error}"),
        })?;
    lower_with_service_calls(model, service_calls)
}

fn lower_with_service_calls(
    model: &PackageSourceModel,
    service_calls: LoweredServiceCalls,
) -> Result<LoweredPackage, SourceCompileError> {
    let mut lowered = LoweredPackage::lower(model, service_calls)?;
    let storage_projection =
        storage_projection::project_service_storage_projection(model, &lowered)?;
    lowered.set_service_storage_projection(storage_projection);
    Ok(lowered)
}
