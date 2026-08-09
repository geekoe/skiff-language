//! Typed source-to-service-contract handoffs.

use skiff_compiler_contract::{
    ContractDefinitionError, ServicePublicInstanceInterfaceOperations,
    ServicePublicInstanceOperationFacts, ServicePublicInstanceOperationSlot,
};
use skiff_compiler_source::PackageSourceModel;

/// Adapts the canonical source-owned public-instance operation table into the
/// contract-owned projection DTO.
///
/// Every root, exact interface instantiation, method ABI id, stable operation
/// key, and declaration slot is copied directly from source facts. In
/// particular, marker rows remain present with zero slots. This seam does not
/// inspect lowered/File IR executables and does not derive operation keys from
/// method names.
pub fn build_public_instance_operation_facts(
    source: &PackageSourceModel,
) -> Result<ServicePublicInstanceOperationFacts, ContractDefinitionError> {
    let interfaces = source
        .public_instance_operations()
        .iter()
        .map(|source_interface| {
            let slots = source_interface
                .slots()
                .iter()
                .map(|source_slot| {
                    ServicePublicInstanceOperationSlot::try_new(
                        source_slot.method_abi_id().to_owned(),
                        source_slot.operation_stable_key().to_owned(),
                    )
                })
                .collect::<Result<Vec<_>, ContractDefinitionError>>()?;
            ServicePublicInstanceInterfaceOperations::try_new(
                source_interface.public_root().to_owned(),
                source_interface.interface().clone(),
                slots,
            )
        })
        .collect::<Result<Vec<_>, ContractDefinitionError>>()?;

    ServicePublicInstanceOperationFacts::try_from_interfaces(interfaces)
}
