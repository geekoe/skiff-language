use std::collections::BTreeMap;

use skiff_artifact_model::{
    PackageBuildId, PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, ServiceDeployment,
};

use super::{
    exact_package_requirement, manifest_error, resolve_package_ref, validate_abi_expectation,
    DeploymentBytecodeHydrationError, DeploymentBytecodeManifestKind, HydratedBytecodePackage,
};

pub(super) fn validate_package_type(
    caller: &HydratedBytecodePackage,
    symbol: &PackageSymbolRef,
    deployment: &ServiceDeployment,
    packages: &BTreeMap<PackageBuildId, HydratedBytecodePackage>,
) -> Result<(), DeploymentBytecodeHydrationError> {
    let target = resolve_package_ref(caller, &symbol.package, deployment, packages)?;
    validate_abi_expectation(caller, symbol, target)?;
    let abi = &target.artifact().package_local_abi;
    let public = abi.public_symbols.get(&symbol.symbol_path);
    let implementation = abi.implementation_symbols.get(&symbol.symbol_path);
    if let (Some(public), Some(implementation)) = (public, implementation) {
        if !same_type_semantics(public, implementation) {
            return manifest_error(
                caller.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "package type {:?} has different public and implementation semantics in exact target {}",
                    symbol.symbol_path,
                    target.reference().package_build_id
                ),
            );
        }
    }

    if let Some(public) = public {
        return require_type_symbol(caller, symbol, target, public, "public");
    }
    if let Some(implementation) = implementation {
        require_implementation_authority(caller, &symbol.package, target, deployment)?;
        return require_type_symbol(caller, symbol, target, implementation, "implementation");
    }

    manifest_error(
        caller.reference(),
        DeploymentBytecodeManifestKind::PackageReference,
        format!(
            "package type {:?} is absent from exact target package {}",
            symbol.symbol_path,
            target.reference().package_build_id
        ),
    )
}

fn require_type_symbol(
    caller: &HydratedBytecodePackage,
    symbol: &PackageSymbolRef,
    target: &HydratedBytecodePackage,
    entry: &PackageLocalAbiSymbol,
    surface: &str,
) -> Result<(), DeploymentBytecodeHydrationError> {
    if matches!(entry, PackageLocalAbiSymbol::Type { .. }) {
        return Ok(());
    }
    manifest_error(
        caller.reference(),
        DeploymentBytecodeManifestKind::PackageReference,
        format!(
            "package symbol {:?} is not a {surface} type in exact target {}",
            symbol.symbol_path,
            target.reference().package_build_id
        ),
    )
}

fn require_implementation_authority(
    caller: &HydratedBytecodePackage,
    package_ref: &PackageRefIr,
    target: &HydratedBytecodePackage,
    deployment: &ServiceDeployment,
) -> Result<(), DeploymentBytecodeHydrationError> {
    match package_ref {
        PackageRefIr::PackageId { .. }
            if caller.reference().package_build_id == target.reference().package_build_id =>
        {
            Ok(())
        }
        PackageRefIr::Dependency { dependency_ref } => {
            let requirement = exact_package_requirement(caller, dependency_ref)?;
            if requirement.expected_package_build.as_ref()
                == Some(&target.reference().package_build_id)
            {
                return Ok(());
            }
            manifest_error(
                caller.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "dependency implementation type access through alias {dependency_ref:?} requires exact build pin {}, found {:?}",
                    target.reference().package_build_id,
                    requirement.expected_package_build
                ),
            )
        }
        PackageRefIr::PackageId { package_id } => {
            if has_unique_direct_exact_build_authority(caller, target, deployment) {
                return Ok(());
            }
            manifest_error(
                caller.reference(),
                DeploymentBytecodeManifestKind::PackageReference,
                format!(
                    "cross-package implementation type access to {package_id:?} has no unique direct exact version/ABI/build requirement and binding"
                ),
            )
        }
    }
}

fn has_unique_direct_exact_build_authority(
    caller: &HydratedBytecodePackage,
    target: &HydratedBytecodePackage,
    deployment: &ServiceDeployment,
) -> bool {
    let mut requirements = caller
        .artifact()
        .package_requirements
        .iter()
        .filter(|requirement| {
            requirement.package_id == target.reference().package_id
                && requirement.exact_version == target.reference().package_version
                && requirement.expected_local_abi == target.reference().package_local_abi_identity
                && requirement.expected_package_build.as_ref()
                    == Some(&target.reference().package_build_id)
                && requirement_has_exact_binding(caller, &requirement.alias, target, deployment)
        });
    requirements.next().is_some() && requirements.next().is_none()
}

fn requirement_has_exact_binding(
    caller: &HydratedBytecodePackage,
    alias: &str,
    target: &HydratedBytecodePackage,
    deployment: &ServiceDeployment,
) -> bool {
    if caller
        .artifact()
        .package_requirements
        .iter()
        .filter(|requirement| requirement.alias == alias)
        .count()
        != 1
    {
        return false;
    }
    let key = skiff_artifact_model::PackageRequirementKey {
        caller_package_build_id: caller.reference().package_build_id.clone(),
        package_requirement_alias: alias.to_string(),
    };
    let mut bindings = deployment
        .package_bindings
        .iter()
        .filter(|binding| binding.key == key);
    bindings
        .next()
        .is_some_and(|binding| &binding.package == target.reference())
        && bindings.next().is_none()
}

fn same_type_semantics(left: &PackageLocalAbiSymbol, right: &PackageLocalAbiSymbol) -> bool {
    match (left, right) {
        (
            PackageLocalAbiSymbol::Type {
                descriptor: left_descriptor,
                is_alias: left_is_alias,
                is_interface: left_is_interface,
                type_params: left_type_params,
                interface_methods: left_interface_methods,
                actor: left_actor,
                ..
            },
            PackageLocalAbiSymbol::Type {
                descriptor: right_descriptor,
                is_alias: right_is_alias,
                is_interface: right_is_interface,
                type_params: right_type_params,
                interface_methods: right_interface_methods,
                actor: right_actor,
                ..
            },
        ) => {
            left_descriptor == right_descriptor
                && left_is_alias == right_is_alias
                && left_is_interface == right_is_interface
                && left_type_params == right_type_params
                && left_interface_methods == right_interface_methods
                && left_actor == right_actor
        }
        _ => false,
    }
}
