use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallableEffectFacts, ConfigMetadataFacts, MetadataValue, PackageImplementationLinks,
    PackageOperationTarget, PublicationAbiUnit,
};
use skiff_compiler_projection_input::ProjectionCallableEffectFacts;

use crate::{error::ProjectionError, ConfigProjection};

pub fn config_metadata_from_config_projection(
    config_projection: &ConfigProjection,
) -> ConfigMetadataFacts {
    let mut config = BTreeMap::new();
    config.insert(
        "shape".to_string(),
        MetadataValue::from_serializable(&config_projection.shape),
    );
    config.insert(
        "uses".to_string(),
        MetadataValue::from_serializable(&config_projection.uses),
    );
    config.insert(
        "activation".to_string(),
        MetadataValue::from_serializable(&config_projection.activation),
    );
    config.insert(
        "requirements".to_string(),
        MetadataValue::from_serializable(&config_projection.requirements),
    );
    config.into()
}

pub(super) fn package_callable_effect_facts(
    source_effects: &ProjectionCallableEffectFacts,
    publication_abi: &PublicationAbiUnit,
    implementation_links: &PackageImplementationLinks,
) -> Result<CallableEffectFacts, ProjectionError> {
    let mut effects = BTreeMap::new();
    for operation in &publication_abi.operation_exports {
        let target = implementation_links
            .operation_targets
            .get(&operation.operation_abi_id)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "public operation {} has no typed implementation target for effect mapping",
                    operation.operation_abi_id
                ),
            })?;
        let (module_path, executable_index) = match target {
            PackageOperationTarget::LocalExecutable { target, .. } => (
                target.file_ref.module_path.as_str(),
                target.executable_index,
            ),
            PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => (
                target.executable_target.file_ref.module_path.as_str(),
                target.executable_target.executable_index,
            ),
        };
        let summary = source_effects
            .operation(module_path, executable_index)
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: format!(
                    "public operation {} implementation {}#{} has no callable effect fact",
                    operation.operation_abi_id, module_path, executable_index
                ),
            })?;
        if effects
            .insert(operation.operation_abi_id.clone(), summary.clone())
            .is_some()
        {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "duplicate callable effect operation ABI id {}",
                    operation.operation_abi_id
                ),
            });
        }
    }
    Ok(CallableEffectFacts::from_operations(effects))
}
