use skiff_artifact_model::{
    PackageBuildId, PackageRefIr, PackageRequirementKey, PackageSymbolRef,
    ValueLifecycleResolverError,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use super::{resolver_error, HydratedValueLifecycleResolver};

const OWNER_AUTHORITY: &str = "hydratedValueLifecycle.ownerNormalization";

impl<'a> HydratedValueLifecycleResolver<'a> {
    pub(in crate::concrete_values) fn current_owner_package(
        &self,
    ) -> Result<&HydratedBytecodePackage, ValueLifecycleResolverError> {
        self.current_package(OWNER_AUTHORITY)
    }

    pub(in crate::concrete_values) fn unique_package_owner(
        &self,
        package_id: &str,
    ) -> Result<&HydratedBytecodePackage, ValueLifecycleResolverError> {
        let mut matches = self
            .hydrated
            .packages()
            .values()
            .filter(|package| package.reference().package_id == package_id);
        let owner = matches.next().ok_or_else(|| {
            resolver_error(
                OWNER_AUTHORITY,
                format!("package id {package_id:?} has no exact hydrated owner"),
            )
        })?;
        if matches.next().is_some() {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!("package id {package_id:?} has ambiguous hydrated owners"),
            ));
        }
        let reference = owner.reference();
        let artifact = owner.artifact();
        if artifact.package_id != reference.package_id
            || artifact.package_version != reference.package_version
            || artifact.package_build_id != reference.package_build_id
            || artifact.package_local_abi.local_abi_identity != reference.package_local_abi_identity
        {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!("package id {package_id:?} owner is not exact"),
            ));
        }
        Ok(owner)
    }

    pub(in crate::concrete_values) fn resolve_package_symbol_owner(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Result<(&HydratedBytecodePackage, bool), ValueLifecycleResolverError> {
        match &symbol.package {
            PackageRefIr::PackageId { package_id } => {
                let owner = self.unique_package_owner(package_id)?;
                let is_self = owner.reference().package_build_id
                    == self.current_owner_package()?.reference().package_build_id;
                let private_authority =
                    is_self || self.has_unique_direct_exact_build_authority(owner);
                Ok((owner, private_authority))
            }
            PackageRefIr::Dependency { dependency_ref } => {
                self.resolve_dependency_owner(dependency_ref)
            }
        }
    }

    fn resolve_dependency_owner(
        &self,
        dependency_ref: &str,
    ) -> Result<(&HydratedBytecodePackage, bool), ValueLifecycleResolverError> {
        let caller = self.current_owner_package()?;
        let mut requirements = caller
            .artifact()
            .package_requirements
            .iter()
            .filter(|requirement| requirement.alias == dependency_ref);
        let requirement = requirements.next().ok_or_else(|| {
            resolver_error(
                OWNER_AUTHORITY,
                format!("dependency alias {dependency_ref:?} has no direct requirement"),
            )
        })?;
        if requirements.next().is_some() {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!("dependency alias {dependency_ref:?} has ambiguous direct requirements"),
            ));
        }
        let key = PackageRequirementKey {
            caller_package_build_id: caller.reference().package_build_id.clone(),
            package_requirement_alias: dependency_ref.to_string(),
        };
        let mut bindings = self
            .hydrated
            .deployment()
            .package_bindings
            .iter()
            .filter(|binding| binding.key == key);
        let binding = bindings.next().ok_or_else(|| {
            resolver_error(
                OWNER_AUTHORITY,
                format!("dependency alias {dependency_ref:?} has no exact deployment binding"),
            )
        })?;
        if bindings.next().is_some() {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!("dependency alias {dependency_ref:?} has ambiguous deployment bindings"),
            ));
        }
        if binding.package.package_id != requirement.package_id
            || binding.package.package_version != requirement.exact_version
            || binding.package.package_local_abi_identity != requirement.expected_local_abi
            || requirement
                .expected_package_build
                .as_ref()
                .is_some_and(|expected| expected != &binding.package.package_build_id)
        {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!(
                    "dependency alias {dependency_ref:?} binding violates exact package/version/ABI/build authority"
                ),
            ));
        }
        let target = self
            .hydrated
            .packages()
            .get(&binding.package.package_build_id)
            .filter(|package| package.reference() == &binding.package)
            .ok_or_else(|| {
                resolver_error(
                    OWNER_AUTHORITY,
                    format!("dependency alias {dependency_ref:?} target is not exactly hydrated"),
                )
            })?;
        if self
            .unique_package_owner(&binding.package.package_id)?
            .reference()
            != target.reference()
        {
            return Err(resolver_error(
                OWNER_AUTHORITY,
                format!("dependency alias {dependency_ref:?} does not select its unique owner"),
            ));
        }
        let exact_build = requirement.expected_package_build.as_ref()
            == Some(&target.reference().package_build_id);
        Ok((target, exact_build))
    }

    pub(super) fn has_unique_direct_exact_build_authority(
        &self,
        target: &HydratedBytecodePackage,
    ) -> bool {
        let Ok(caller) = self.current_owner_package() else {
            return false;
        };
        let mut requirements =
            caller
                .artifact()
                .package_requirements
                .iter()
                .filter(|requirement| {
                    requirement.package_id == target.reference().package_id
                        && requirement.exact_version == target.reference().package_version
                        && requirement.expected_local_abi
                            == target.reference().package_local_abi_identity
                        && requirement.expected_package_build.as_ref()
                            == Some(&target.reference().package_build_id)
                        && self.requirement_has_exact_binding(
                            &caller.reference().package_build_id,
                            &requirement.alias,
                            target,
                        )
                });
        requirements.next().is_some() && requirements.next().is_none()
    }

    fn requirement_has_exact_binding(
        &self,
        caller_build: &PackageBuildId,
        alias: &str,
        target: &HydratedBytecodePackage,
    ) -> bool {
        let Ok(caller) = self.current_owner_package() else {
            return false;
        };
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
        let key = PackageRequirementKey {
            caller_package_build_id: caller_build.clone(),
            package_requirement_alias: alias.to_string(),
        };
        let mut bindings = self
            .hydrated
            .deployment()
            .package_bindings
            .iter()
            .filter(|binding| binding.key == key);
        bindings
            .next()
            .is_some_and(|binding| &binding.package == target.reference())
            && bindings.next().is_none()
    }
}
