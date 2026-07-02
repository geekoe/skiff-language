use crate::contract::ContractProjection;
use crate::error::ProjectionError;
use crate::runtime::EntryOperationSpec;
use crate::service::dependency_abi::{
    resolve_service_dependency_symbol_refs, validate_service_dependency_symbol_refs,
    ServiceDependencyAbiResolutionInput, ServiceDependencyAbiValidationInput,
};
use crate::service::service_unit::ensure_service_operation_adapters;
use crate::source_map::{publication_source_map_from_file_ir_units, PublicationSourceMap};
use crate::typed_artifacts::ServiceDependencyConstraint;
use skiff_artifact_model::FileIrUnit;
use skiff_compiler_core::file_ir_identity::assign_file_ir_identity;

pub struct ServiceFileIrPreparationInput<'a> {
    pub file_ir_units: &'a [FileIrUnit],
    pub service_dependencies: &'a [ServiceDependencyConstraint],
    pub contract_projection: &'a ContractProjection,
    pub entry_service_operations: &'a [EntryOperationSpec],
}

pub struct PreparedServiceFileIr {
    pub file_ir_units: Vec<FileIrUnit>,
    pub source_map: PublicationSourceMap,
}

pub fn prepare_service_file_ir(
    input: ServiceFileIrPreparationInput<'_>,
) -> Result<PreparedServiceFileIr, ProjectionError> {
    let mut file_ir_units = input.file_ir_units.to_vec();
    resolve_service_dependency_symbol_refs(ServiceDependencyAbiResolutionInput {
        dependencies: input.service_dependencies,
        service_file_units: &mut file_ir_units,
    })?;
    validate_service_dependency_symbol_refs(ServiceDependencyAbiValidationInput {
        dependencies: input.service_dependencies,
        service_file_units: &file_ir_units,
    })?;
    ensure_service_operation_adapters(
        &mut file_ir_units,
        input.contract_projection,
        input.entry_service_operations,
    )?;
    for unit in &mut file_ir_units {
        assign_file_ir_identity(unit);
    }
    let source_map = publication_source_map_from_file_ir_units(&file_ir_units)?;
    Ok(PreparedServiceFileIr {
        file_ir_units,
        source_map,
    })
}
