use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation};

use super::super::{obligation_error, TypeNormalizer};
use super::{require_type_export_matches_symbol, require_type_symbol, same_type_semantics};

impl TypeNormalizer<'_> {
    /// A normalized PackageId no longer carries its source alias. Recover
    /// private authority only from one exact direct edge owned by this caller;
    /// mere presence elsewhere in the hydrated closure is never authority.
    pub(super) fn has_unique_direct_exact_build_authority(
        &self,
        target: &HydratedBytecodePackage,
    ) -> bool {
        let mut requirements =
            self.caller
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
                        && self.requirement_has_exact_binding(&requirement.alias, target)
                });
        requirements.next().is_some() && requirements.next().is_none()
    }

    fn requirement_has_exact_binding(&self, alias: &str, target: &HydratedBytecodePackage) -> bool {
        if self
            .caller
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
            caller_package_build_id: self.caller.reference().package_build_id.clone(),
            package_requirement_alias: alias.to_string(),
        };
        let mut bindings = self
            .deployment
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

pub(super) fn require_dependency_type(
    target: &HydratedBytecodePackage,
    symbol_path: &str,
    has_exact_build_authority: bool,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let artifact = target.artifact();
    let public = artifact.package_local_abi.public_symbols.get(symbol_path);
    let implementation = artifact
        .package_local_abi
        .implementation_symbols
        .get(symbol_path);
    if let (Some(public), Some(implementation)) = (public, implementation) {
        if !same_type_semantics(public, implementation) {
            return Err(obligation_error(
                location.clone(),
                format!(
                    "dependency package type {symbol_path:?} is ambiguous across different public and implementation semantics in exact target {}",
                    target.reference().package_build_id
                ),
            ));
        }
    }

    if let Some(public) = public {
        return require_type_symbol(
            public,
            location,
            format!("public package type {symbol_path:?}"),
        );
    }

    let implementation = implementation.ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!(
                "package type {symbol_path:?} is absent from exact target {}",
                target.reference().package_build_id
            ),
        )
    })?;
    if !has_exact_build_authority {
        return Err(obligation_error(
            location.clone(),
            format!(
                "package type {symbol_path:?} is not public and has no unique direct exact-build authority for target {}",
                target.reference().package_build_id
            ),
        ));
    }
    let export = artifact
        .implementation_links
        .types
        .get(symbol_path)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!(
                    "dependency implementation type {symbol_path:?} has no exact implementation link"
                ),
            )
        })?;
    require_type_export_matches_symbol(
        implementation,
        export,
        location,
        format!("dependency implementation type {symbol_path:?}"),
    )
}
