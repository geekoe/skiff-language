use skiff_artifact_model::{PackageLocalAbiSymbol, PackageSymbolRef, TypeExport, TypeRefIr};
use skiff_runtime_loader::HydratedBytecodePackage;

use super::TypeOwnerNormalizer;
use crate::VerificationError;

impl TypeOwnerNormalizer<'_, '_> {
    pub(super) fn normalize_publication_type(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Result<TypeRefIr, VerificationError> {
        let caller = self
            .resolver
            .current_owner_package()
            .map_err(|error| self.authority_violation("reading publication owner", error))?;
        let matches = caller
            .artifact()
            .implementation_links
            .types
            .iter()
            .filter(|(_, export)| {
                export.file.module_path == module_path && export.type_index == type_index
            })
            .collect::<Vec<_>>();
        let (_, export) = self.unique_canonical_export(
            &matches,
            format!("publication type {module_path}#{type_index}"),
        )?;
        let path = canonical_implementation_path(export).ok_or_else(|| {
            self.violation(format!(
                "publication type {module_path}#{type_index} has an incomplete implementation link"
            ))
        })?;
        self.require_exact_implementation_type(caller, &path, export)?;
        Ok(exact_package_symbol(caller, path))
    }

    pub(super) fn normalize_service_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Result<TypeRefIr, VerificationError> {
        let caller = self
            .resolver
            .current_owner_package()
            .map_err(|error| self.authority_violation("reading service-symbol owner", error))?;
        let expected_path = format!("{module_path}.{symbol}");
        let matches = caller
            .artifact()
            .implementation_links
            .types
            .iter()
            .filter(|(_, export)| export.file.module_path == module_path && export.symbol == symbol)
            .collect::<Vec<_>>();
        let (path, export) =
            self.unique_canonical_export(&matches, format!("service type {expected_path}"))?;
        if path.as_str() != expected_path {
            return Err(self.violation(format!(
                "service type {expected_path} resolves to non-canonical path {path:?}"
            )));
        }
        self.require_exact_implementation_type(caller, path, export)?;
        Ok(exact_package_symbol(caller, path.clone()))
    }

    pub(super) fn normalize_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<TypeRefIr, VerificationError> {
        if symbol.symbol_path.is_empty() {
            return Err(self.violation("package symbol path is empty"));
        }
        let (target, private_authority) = self
            .resolver
            .resolve_package_symbol_owner(symbol)
            .map_err(|error| self.authority_violation("resolving package-symbol owner", error))?;
        if symbol.abi_expectation.as_deref().is_some_and(|expected| {
            expected != target.reference().package_local_abi_identity.as_str()
        }) {
            return Err(self.violation(format!(
                "package type {:?} ABI expectation {:?} disagrees with exact target {}",
                symbol.symbol_path,
                symbol.abi_expectation,
                target.reference().package_local_abi_identity
            )));
        }
        let is_self = target.reference().package_build_id
            == self
                .resolver
                .current_owner_package()
                .map_err(|error| self.authority_violation("reading package-symbol scope", error))?
                .reference()
                .package_build_id;
        self.require_package_type(target, &symbol.symbol_path, private_authority, is_self)?;
        if private_authority
            && matches!(
                &symbol.package,
                skiff_artifact_model::PackageRefIr::Dependency { .. }
            )
        {
            self.private_type_authority
                .insert(target.reference().package_build_id.clone());
        }
        Ok(exact_package_symbol(target, symbol.symbol_path.clone()))
    }

    fn require_package_type(
        &self,
        target: &HydratedBytecodePackage,
        symbol_path: &str,
        private_authority: bool,
        is_self: bool,
    ) -> Result<(), VerificationError> {
        let artifact = target.artifact();
        let public = artifact.package_local_abi.public_symbols.get(symbol_path);
        let implementation = artifact
            .package_local_abi
            .implementation_symbols
            .get(symbol_path);
        if let (Some(public), Some(implementation)) = (public, implementation) {
            if !same_type_semantics(public, implementation) {
                return Err(self.violation(format!(
                    "package type {symbol_path:?} has different public and implementation semantics in exact target {}",
                    target.reference().package_build_id
                )));
            }
        }
        if is_self {
            if let Some(implementation) = implementation {
                let export = artifact
                    .implementation_links
                    .types
                    .get(symbol_path)
                    .ok_or_else(|| {
                        self.violation(format!(
                            "self implementation type {symbol_path:?} has no exact implementation link"
                        ))
                    })?;
                self.require_type_export_matches_symbol(
                    implementation,
                    export,
                    format!("self implementation type {symbol_path:?}"),
                )?;
                return Ok(());
            }
        }
        if let Some(public) = public {
            self.require_type_symbol(public, format!("public package type {symbol_path:?}"))?;
            return Ok(());
        }
        let implementation = implementation.ok_or_else(|| {
            self.violation(format!(
                "package type {symbol_path:?} is absent from exact target {}",
                target.reference().package_build_id
            ))
        })?;
        if !private_authority {
            return Err(self.violation(format!(
                "package type {symbol_path:?} is private and has no unique direct exact-build authority for target {}",
                target.reference().package_build_id
            )));
        }
        let export = artifact
            .implementation_links
            .types
            .get(symbol_path)
            .ok_or_else(|| {
                self.violation(format!(
                    "private implementation type {symbol_path:?} has no exact implementation link"
                ))
            })?;
        self.require_type_export_matches_symbol(
            implementation,
            export,
            format!("private implementation type {symbol_path:?}"),
        )?;
        Ok(())
    }

    fn unique_canonical_export<'a>(
        &self,
        matches: &[(&'a String, &'a TypeExport)],
        label: String,
    ) -> Result<(&'a String, &'a TypeExport), VerificationError> {
        let selected = matches
            .first()
            .copied()
            .ok_or_else(|| self.violation(format!("{label} has no exact implementation link")))?;
        if matches
            .iter()
            .skip(1)
            .any(|(_, candidate)| *candidate != selected.1)
        {
            return Err(self.violation(format!("{label} has conflicting implementation links")));
        }
        let canonical = matches
            .iter()
            .copied()
            .filter(|(path, export)| {
                canonical_implementation_path(export).as_deref() == Some(path.as_str())
            })
            .collect::<Vec<_>>();
        if canonical.len() != 1 {
            return Err(self.violation(format!(
                "{label} has no unique canonical implementation Type path"
            )));
        }
        Ok(canonical[0])
    }

    fn require_exact_implementation_type(
        &self,
        owner: &HydratedBytecodePackage,
        symbol_path: &str,
        export: &TypeExport,
    ) -> Result<(), VerificationError> {
        let implementation = owner
            .artifact()
            .package_local_abi
            .implementation_symbols
            .get(symbol_path)
            .ok_or_else(|| {
                self.violation(format!(
                    "canonical implementation type {symbol_path:?} has no exact ABI descriptor"
                ))
            })?;
        self.require_type_export_matches_symbol(
            implementation,
            export,
            format!("canonical implementation type {symbol_path:?}"),
        )
    }

    fn require_type_symbol(
        &self,
        symbol: &PackageLocalAbiSymbol,
        label: String,
    ) -> Result<(), VerificationError> {
        if matches!(symbol, PackageLocalAbiSymbol::Type { .. }) {
            Ok(())
        } else {
            Err(self.violation(format!("{label} is not a Type symbol")))
        }
    }

    fn require_type_export_matches_symbol(
        &self,
        symbol: &PackageLocalAbiSymbol,
        export: &TypeExport,
        label: String,
    ) -> Result<(), VerificationError> {
        let PackageLocalAbiSymbol::Type {
            descriptor,
            is_interface,
            type_params,
            interface_methods,
            actor,
            ..
        } = symbol
        else {
            return Err(self.violation(format!("{label} ABI entry is not a Type symbol")));
        };
        if export.descriptor.as_ref() != Some(descriptor)
            || export.is_interface != *is_interface
            || export.type_params != *type_params
            || export.interface_methods != *interface_methods
            || export.actor != *actor
        {
            return Err(self.violation(format!(
                "{label} implementation link and typed descriptor disagree"
            )));
        }
        Ok(())
    }
}

fn exact_package_symbol(owner: &HydratedBytecodePackage, symbol_path: String) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: owner.reference().package_id.clone(),
            },
            symbol_path,
            abi_expectation: Some(
                owner
                    .reference()
                    .package_local_abi_identity
                    .as_str()
                    .to_string(),
            ),
        },
    }
}

fn canonical_implementation_path(export: &TypeExport) -> Option<String> {
    if export.file.module_path.is_empty() || export.symbol.is_empty() {
        return None;
    }
    let prefix = format!("{}.", export.file.module_path);
    Some(if export.symbol.starts_with(&prefix) {
        export.symbol.clone()
    } else {
        format!("{}.{}", export.file.module_path, export.symbol)
    })
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
