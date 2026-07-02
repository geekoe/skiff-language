use std::collections::BTreeSet;

use skiff_artifact_model::OperationRouteBinding;
use skiff_runtime_linked_program::LinkedProgramImage;
use skiff_runtime_linker::{ProgramError, ProgramResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActivationRequirements {
    linked_package_count: usize,
    operation_abi_ids: BTreeSet<String>,
}

impl ActivationRequirements {
    pub(super) fn from_linked_image(image: &LinkedProgramImage) -> Self {
        Self {
            linked_package_count: image.packages.len(),
            operation_abi_ids: image.operations.keys().cloned().collect(),
        }
    }

    pub(super) fn validate_runtime_facts(
        &self,
        package_config_count: usize,
        operation_route_bindings: &[OperationRouteBinding],
    ) -> ProgramResult<()> {
        if package_config_count > self.linked_package_count {
            return Err(
                ProgramError::ActivationPackageConfigsExceedLinkedPackageSlots {
                    package_config_count,
                    linked_package_count: self.linked_package_count,
                },
            );
        }

        for binding in operation_route_bindings {
            if !self.operation_abi_ids.contains(&binding.operation_abi_id) {
                return Err(ProgramError::ActivationRouteBindingUnknownOperation {
                    selector: binding.selector.clone(),
                    operation_abi_id: binding.operation_abi_id.clone(),
                });
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(super) fn for_test(
        linked_package_count: usize,
        operation_abi_ids: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            linked_package_count,
            operation_abi_ids: operation_abi_ids.into_iter().map(Into::into).collect(),
        }
    }
}
