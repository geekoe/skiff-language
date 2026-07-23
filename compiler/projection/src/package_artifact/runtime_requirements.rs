use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    CallTargetIr, ExprIr, FileIrUnit, PackageConfigRequirement,
    PackageRuntimeCapabilityRequirement, PackageRuntimeRequirements,
};
use skiff_compiler_projection_input::{ConfigRequirementAccessProjection, ConfigRequirementsSeed};
use skiff_trusted_registry_contract::{
    trusted_registry_native_capability_spec, TRUSTED_REGISTRY_CAPABILITY_ID,
    TRUSTED_REGISTRY_CAPABILITY_VERSION,
};

use crate::error::ProjectionError;

pub(super) fn project_runtime_requirements(
    package_id: &str,
    requirements: &ConfigRequirementsSeed,
    file_ir_units: &[FileIrUnit],
) -> Result<PackageRuntimeRequirements, ProjectionError> {
    let mut config_by_path = BTreeMap::<String, PackageConfigRequirement>::new();
    for requirement in requirements.effective().requirements() {
        let (value_type, required) = match requirement.access() {
            ConfigRequirementAccessProjection::Require { ty } => (ty.clone(), true),
            ConfigRequirementAccessProjection::Optional { ty } => (ty.clone(), false),
            ConfigRequirementAccessProjection::Has => continue,
        };
        let projected = PackageConfigRequirement {
            path: requirement.path().to_string(),
            value_type,
            required,
        };
        match config_by_path.get(&projected.path) {
            Some(existing) if existing == &projected => {}
            Some(existing) => {
                return Err(ProjectionError::InvalidPackageArtifact {
                    message: format!(
                        "package {package_id} config requirement {} conflicts: {:?} vs {:?}",
                        projected.path, existing, projected
                    ),
                });
            }
            None => {
                config_by_path.insert(projected.path.clone(), projected);
            }
        }
    }
    let native_binding_keys = file_ir_units
        .iter()
        .flat_map(|unit| {
            unit.constants
                .iter()
                .map(|constant| &constant.body)
                .chain(unit.executables.iter().map(|executable| &executable.body))
        })
        .flat_map(|body| &body.expressions)
        .filter_map(|expression| {
            let ExprIr::Call { call } = expression else {
                return None;
            };
            let CallTargetIr::Native { target } = &call.target else {
                return None;
            };
            target.binding_key.as_deref()
        });
    let runtime_capabilities = project_native_runtime_capabilities(native_binding_keys);
    Ok(PackageRuntimeRequirements {
        config: config_by_path.into_values().collect(),
        resources: Vec::new(),
        runtime_capabilities,
    })
}

fn project_native_runtime_capabilities<'a>(
    binding_keys: impl Iterator<Item = &'a str>,
) -> Vec<PackageRuntimeCapabilityRequirement> {
    let operation_scopes = binding_keys
        .filter_map(trusted_registry_native_capability_spec)
        .map(|spec| spec.operation_scope.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let runtime_capabilities = if operation_scopes.is_empty() {
        Vec::new()
    } else {
        vec![PackageRuntimeCapabilityRequirement {
            capability: TRUSTED_REGISTRY_CAPABILITY_ID.to_string(),
            required_version: TRUSTED_REGISTRY_CAPABILITY_VERSION.to_string(),
            operation_scopes: operation_scopes.into_iter().collect(),
        }]
    };
    runtime_capabilities
}

#[cfg(test)]
mod tests {
    use super::project_native_runtime_capabilities;

    #[test]
    fn exact_native_keys_project_deduplicated_sorted_capability_and_scopes() {
        let projected = project_native_runtime_capabilities(
            [
                "registry.packageArtifact.pointer.cas",
                "registry.activation.activate",
                "registry.packageArtifact.pointer.cas",
                "registry.packageArtifact.read",
            ]
            .into_iter(),
        );
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].capability, "skiff.registry.trusted");
        assert_eq!(projected[0].required_version, "1");
        assert_eq!(
            projected[0].operation_scopes,
            ["activation.activate", "artifact.read", "pointer.cas"]
        );
    }

    #[test]
    fn package_names_manifest_strings_and_unknown_bindings_grant_nothing() {
        for untrusted in [
            ["skiff.run/registry"].as_slice(),
            ["skiff.registry.trusted"].as_slice(),
            ["registry.packageArtifact.unknown"].as_slice(),
            ["registry.packageArtifact.read.forged"].as_slice(),
            ["ordinary.package.native"].as_slice(),
        ] {
            assert!(project_native_runtime_capabilities(untrusted.iter().copied()).is_empty());
        }
    }
}
