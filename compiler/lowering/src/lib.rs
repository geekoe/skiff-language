pub mod callable_return_types;
mod db_lowering;
mod declaration_lowering;
pub mod dependency_operation_indexes;
pub mod entrypoint_abi;
pub mod entrypoint_abi_model;
mod executable_declaration_lowering;
mod external_refs;
pub mod file_ir;
mod function_lowering;
pub mod lowered;
pub mod source_file_lowering;
pub mod source_metadata;
mod source_unit_lowering;
pub mod storage_projection;
mod suspend_analysis;
mod type_inference;
mod type_lowering;

use dependency_operation_indexes::LoweringDependencyOperationIndexes;
pub use entrypoint_abi::{
    package_entrypoint_function_signature, package_public_schema_abi_types_for_module,
    package_public_schema_type_names_for_module, EntrypointAbiIndex,
};
pub use entrypoint_abi_model::{
    type_ref_ir_source_text_with_local_types, EntryFunctionSignature, EntryParamSpec,
    EntryTypeSpec, PackageAbiType, PackageAbiTypeDescriptor,
};
pub use lowered::{
    LoweredPublication, SyntheticEntrypointExecutableKind, SyntheticEntrypointIndex,
    SyntheticEntrypointModule, SyntheticOperationIndex,
};
pub use source_metadata::CompiledPublicationSource;
pub use storage_projection::{
    service_spawn_targets_with_packages, CompiledPublicationStorageProjection,
    PackageSpawnTargetSource,
};

use skiff_compiler_source::{PublicationKind, SourceCompileError, SourceCompileModel};

pub fn lower(model: &SourceCompileModel) -> Result<LoweredPublication, SourceCompileError> {
    let operation_indexes = LoweringDependencyOperationIndexes::build(model)?;
    let mut lowered = LoweredPublication::lower(model, &operation_indexes)?;
    if matches!(model.publication_kind(), PublicationKind::Service) {
        let storage_projection =
            storage_projection::project_service_storage_projection(model, &lowered)?;
        lowered.set_service_storage_projection(storage_projection);
    }
    Ok(lowered)
}
