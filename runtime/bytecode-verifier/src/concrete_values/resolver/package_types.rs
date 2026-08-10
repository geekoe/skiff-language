use skiff_artifact_model::{
    PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, TypeDescriptorIr,
    ValueLifecycleResolverError,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use super::{
    descriptor::owner_normalize_descriptor, resolver_error, HydratedValueLifecycleResolver,
    PACKAGE_SYMBOL_AUTHORITY,
};

impl HydratedValueLifecycleResolver<'_> {
    pub(super) fn resolve_package_type(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Result<ResolvedPackageType, ValueLifecycleResolverError> {
        let scope_build_id = self
            .current_package(PACKAGE_SYMBOL_AUTHORITY)?
            .reference()
            .package_build_id
            .clone();
        let PackageRefIr::PackageId { package_id } = &symbol.package else {
            return Err(resolver_error(
                PACKAGE_SYMBOL_AUTHORITY,
                "dependency package references are not exact package symbol owners",
            ));
        };
        if symbol.symbol_path.is_empty() {
            return Err(resolver_error(
                PACKAGE_SYMBOL_AUTHORITY,
                "package symbol path is empty",
            ));
        }
        let package = self.package_for_id(package_id, PACKAGE_SYMBOL_AUTHORITY)?;
        let package_abi = &package.artifact().package_local_abi;
        let is_self = package.reference().package_build_id == scope_build_id;
        let has_exact_build_authority = is_self
            || self.has_row_private_type_authority(package)
            || self.has_unique_direct_exact_build_authority(package);
        let exact_abi = package.reference().package_local_abi_identity
            == package_abi.local_abi_identity
            && match symbol
                .abi_expectation
                .as_deref()
                .filter(|expectation| !expectation.is_empty())
            {
                Some(expectation) => expectation == package_abi.local_abi_identity.as_str(),
                None => has_exact_build_authority,
            };
        if !exact_abi {
            return Err(resolver_error(
                PACKAGE_SYMBOL_AUTHORITY,
                format!(
                    "package symbol {package_id}/{} does not name the required exact ABI or exact-build authority",
                    symbol.symbol_path
                ),
            ));
        }

        let public = package_abi.public_symbols.get(&symbol.symbol_path);
        let implementation = package_abi.implementation_symbols.get(&symbol.symbol_path);
        if let (Some(public), Some(implementation)) = (public, implementation) {
            if !same_type_semantics(public, implementation) {
                return Err(resolver_error(
                    PACKAGE_SYMBOL_AUTHORITY,
                    format!(
                        "package symbol {package_id}/{} has different public and implementation semantics",
                        symbol.symbol_path
                    ),
                ));
            }
        }

        let entry = match (is_self, public, implementation) {
            (true, _, Some(implementation)) => {
                require_exact_implementation_type(package, &symbol.symbol_path, implementation)?;
                implementation
            }
            (_, Some(public), _) => public,
            (false, None, Some(implementation)) => {
                if !has_exact_build_authority {
                    return Err(resolver_error(
                        PACKAGE_SYMBOL_AUTHORITY,
                        format!(
                            "package symbol {package_id}/{} is a cross-package implementation type without unique direct exact-build authority",
                            symbol.symbol_path
                        ),
                    ));
                }
                require_exact_implementation_type(package, &symbol.symbol_path, implementation)?;
                implementation
            }
            (_, None, None) => {
                return Err(resolver_error(
                    PACKAGE_SYMBOL_AUTHORITY,
                    format!(
                        "unknown package type symbol {package_id}/{}",
                        symbol.symbol_path
                    ),
                ));
            }
        };

        let PackageLocalAbiSymbol::Type {
            descriptor,
            is_interface,
            type_params,
            ..
        } = entry
        else {
            return Err(resolver_error(
                PACKAGE_SYMBOL_AUTHORITY,
                format!(
                    "package symbol {package_id}/{} is not a type",
                    symbol.symbol_path
                ),
            ));
        };
        Ok(ResolvedPackageType {
            type_parameters: type_params.clone(),
            descriptor: owner_normalize_descriptor(package, descriptor)?,
            is_interface: *is_interface,
        })
    }
}

pub(super) struct ResolvedPackageType {
    pub(super) type_parameters: Vec<String>,
    pub(super) descriptor: TypeDescriptorIr,
    pub(super) is_interface: bool,
}

fn require_exact_implementation_type(
    package: &HydratedBytecodePackage,
    symbol_path: &str,
    symbol: &PackageLocalAbiSymbol,
) -> Result<(), ValueLifecycleResolverError> {
    let export = package
        .artifact()
        .implementation_links
        .types
        .get(symbol_path)
        .ok_or_else(|| {
            resolver_error(
                PACKAGE_SYMBOL_AUTHORITY,
                format!("implementation package type {symbol_path:?} has no exact link"),
            )
        })?;
    let PackageLocalAbiSymbol::Type {
        descriptor,
        is_interface,
        type_params,
        interface_methods,
        actor,
        ..
    } = symbol
    else {
        return Err(resolver_error(
            PACKAGE_SYMBOL_AUTHORITY,
            format!("implementation package symbol {symbol_path:?} is not a type"),
        ));
    };
    if export.descriptor.as_ref() != Some(descriptor)
        || export.is_interface != *is_interface
        || export.type_params != *type_params
        || export.interface_methods != *interface_methods
        || export.actor != *actor
    {
        return Err(resolver_error(
            PACKAGE_SYMBOL_AUTHORITY,
            format!("implementation package type {symbol_path:?} link and ABI disagree"),
        ));
    }
    Ok(())
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
