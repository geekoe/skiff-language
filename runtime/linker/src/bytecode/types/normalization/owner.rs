mod dependency;

use skiff_artifact_model::{
    PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, TypeExport, TypeRefIr,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation};

use super::{obligation_error, TypeNormalizer};
use dependency::require_dependency_type;

impl TypeNormalizer<'_> {
    pub(super) fn normalize_publication_type(
        &self,
        module_path: &str,
        type_index: u32,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let matches = self
            .caller
            .artifact()
            .implementation_links
            .types
            .iter()
            .filter(|(_, export)| {
                export.file.module_path == module_path && export.type_index == type_index
            })
            .collect::<Vec<_>>();
        let (_, export) = unique_canonical_export(
            &matches,
            self.location,
            format!("publication type {module_path}#{type_index}"),
        )?;
        let path = canonical_implementation_path(export).ok_or_else(|| {
            self.error(format!(
                "publication type {module_path}#{type_index} has an incomplete implementation link"
            ))
        })?;
        self.require_exact_implementation_type(&path, export)?;
        Ok(self.exact_package_symbol(self.caller, path))
    }

    pub(super) fn normalize_service_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let expected_path = format!("{module_path}.{symbol}");
        let matches = self
            .caller
            .artifact()
            .implementation_links
            .types
            .iter()
            .filter(|(_, export)| {
                export.file.module_path == module_path
                    && canonical_implementation_path(export).as_deref()
                        == Some(expected_path.as_str())
            })
            .collect::<Vec<_>>();
        let (path, export) = unique_canonical_export(
            &matches,
            self.location,
            format!("service type {expected_path}"),
        )?;
        if path.as_str() != expected_path {
            return Err(self.error(format!(
                "service type {expected_path} resolves to non-canonical implementation path {path:?}"
            )));
        }
        self.require_exact_implementation_type(path, export)?;
        Ok(self.exact_package_symbol(self.caller, path.clone()))
    }

    pub(super) fn normalize_package_symbol(
        &self,
        symbol: &PackageSymbolRef,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let target = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => {
                let (target, has_exact_build_authority) =
                    self.resolve_dependency(dependency_ref)?;
                self.check_abi_expectation(symbol, target)?;
                require_dependency_type(
                    target,
                    &symbol.symbol_path,
                    has_exact_build_authority,
                    self.location,
                )?;
                target
            }
            PackageRefIr::PackageId { package_id } => {
                let target = self.unique_package_owner(package_id)?;
                self.check_abi_expectation(symbol, target)?;
                if target.reference().package_build_id == self.caller.reference().package_build_id {
                    self.require_self_type(&symbol.symbol_path)?;
                } else {
                    require_dependency_type(
                        target,
                        &symbol.symbol_path,
                        self.has_unique_direct_exact_build_authority(target),
                        self.location,
                    )?;
                }
                target
            }
        };
        Ok(self.exact_package_symbol(target, symbol.symbol_path.clone()))
    }

    fn resolve_dependency(
        &self,
        dependency_ref: &str,
    ) -> Result<(&HydratedBytecodePackage, bool), BytecodeLinkError> {
        let mut requirements = self
            .caller
            .artifact()
            .package_requirements
            .iter()
            .filter(|requirement| requirement.alias == dependency_ref);
        let requirement = requirements.next().ok_or_else(|| {
            self.error(format!(
                "dependency alias {dependency_ref:?} has no caller package requirement"
            ))
        })?;
        if requirements.next().is_some() {
            return Err(self.error(format!(
                "dependency alias {dependency_ref:?} has ambiguous caller package requirements"
            )));
        }

        let key = skiff_artifact_model::PackageRequirementKey {
            caller_package_build_id: self.caller.reference().package_build_id.clone(),
            package_requirement_alias: dependency_ref.to_string(),
        };
        let mut bindings = self
            .deployment
            .deployment()
            .package_bindings
            .iter()
            .filter(|binding| binding.key == key);
        let binding = bindings.next().ok_or_else(|| {
            self.error(format!(
                "dependency alias {dependency_ref:?} has no exact deployment binding"
            ))
        })?;
        if bindings.next().is_some() {
            return Err(self.error(format!(
                "dependency alias {dependency_ref:?} has duplicate deployment bindings"
            )));
        }
        if binding.package.package_id != requirement.package_id
            || binding.package.package_version != requirement.exact_version
            || binding.package.package_local_abi_identity != requirement.expected_local_abi
            || requirement
                .expected_package_build
                .as_ref()
                .is_some_and(|expected| expected != &binding.package.package_build_id)
        {
            return Err(self.error(format!(
                "dependency alias {dependency_ref:?} binding violates its exact package requirement"
            )));
        }
        let target = self
            .deployment
            .packages()
            .get(&binding.package.package_build_id)
            .filter(|package| package.reference() == &binding.package)
            .ok_or_else(|| {
                self.error(format!(
                    "dependency alias {dependency_ref:?} exact target is absent from the hydrated closure"
                ))
            })?;
        let unique = self.unique_package_owner(&binding.package.package_id)?;
        if unique.reference() != target.reference() {
            return Err(self.error(format!(
                "dependency alias {dependency_ref:?} does not resolve to its unique package owner"
            )));
        }
        let has_exact_build_authority = requirement.expected_package_build.as_ref()
            == Some(&target.reference().package_build_id);
        Ok((target, has_exact_build_authority))
    }

    pub(super) fn unique_package_owner(
        &self,
        package_id: &str,
    ) -> Result<&HydratedBytecodePackage, BytecodeLinkError> {
        let mut matches = self
            .deployment
            .packages()
            .values()
            .filter(|package| package.reference().package_id == package_id);
        let owner = matches.next().ok_or_else(|| {
            self.error(format!(
                "package id {package_id:?} has no owner in the exact hydrated closure"
            ))
        })?;
        if matches.next().is_some() {
            return Err(self.error(format!(
                "package id {package_id:?} has more than one owner in the exact hydrated closure"
            )));
        }
        if owner.artifact().package_id != owner.reference().package_id
            || owner.artifact().package_version != owner.reference().package_version
            || owner.artifact().package_build_id != owner.reference().package_build_id
            || owner.artifact().package_local_abi.local_abi_identity
                != owner.reference().package_local_abi_identity
        {
            return Err(self.error(format!(
                "package id {package_id:?} owner disagrees with its exact artifact reference"
            )));
        }
        Ok(owner)
    }

    fn check_abi_expectation(
        &self,
        symbol: &PackageSymbolRef,
        target: &HydratedBytecodePackage,
    ) -> Result<(), BytecodeLinkError> {
        if symbol.abi_expectation.as_deref().is_some_and(|expected| {
            expected != target.reference().package_local_abi_identity.as_str()
        }) {
            return Err(self.error(format!(
                "package type {:?} ABI expectation {:?} disagrees with exact target {}",
                symbol.symbol_path,
                symbol.abi_expectation,
                target.reference().package_local_abi_identity
            )));
        }
        Ok(())
    }

    fn require_self_type(&self, symbol_path: &str) -> Result<(), BytecodeLinkError> {
        let artifact = self.caller.artifact();
        let implementation = artifact
            .package_local_abi
            .implementation_symbols
            .get(symbol_path);
        let public = artifact.package_local_abi.public_symbols.get(symbol_path);
        if let (Some(implementation), Some(public)) = (implementation, public) {
            if !same_type_semantics(implementation, public) {
                return Err(self.error(format!(
                    "self package type {symbol_path:?} has different public and implementation semantics"
                )));
            }
        }

        if let Some(implementation) = implementation {
            let export = artifact
                .implementation_links
                .types
                .get(symbol_path)
                .ok_or_else(|| {
                    self.error(format!(
                        "self package type {symbol_path:?} has no exact implementation link"
                    ))
                })?;
            require_type_export_matches_symbol(
                implementation,
                export,
                self.location,
                format!("self package type {symbol_path:?}"),
            )?;
            return Ok(());
        }
        let public = public.ok_or_else(|| {
            self.error(format!(
                "self package type {symbol_path:?} is absent from implementation and public ABI surfaces"
            ))
        })?;
        require_type_symbol(
            public,
            self.location,
            format!("self package type {symbol_path:?}"),
        )
    }

    fn require_exact_implementation_type(
        &self,
        symbol_path: &str,
        export: &TypeExport,
    ) -> Result<(), BytecodeLinkError> {
        let implementation = self
            .caller
            .artifact()
            .package_local_abi
            .implementation_symbols
            .get(symbol_path)
            .ok_or_else(|| {
                self.error(format!(
                    "canonical implementation type {symbol_path:?} has no exact implementation ABI descriptor"
                ))
            })?;
        require_type_export_matches_symbol(
            implementation,
            export,
            self.location,
            format!("canonical implementation type {symbol_path:?}"),
        )
    }

    fn exact_package_symbol(
        &self,
        owner: &HydratedBytecodePackage,
        symbol_path: String,
    ) -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
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
}

fn unique_canonical_export<'a>(
    matches: &[(&'a String, &'a TypeExport)],
    location: &BytecodeLinkLocation,
    label: String,
) -> Result<(&'a String, &'a TypeExport), BytecodeLinkError> {
    let selected = matches.first().copied().ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("{label} has no exact implementation type link"),
        )
    })?;
    if matches.iter().skip(1).any(|(_, candidate)| {
        candidate.file != selected.1.file
            || candidate.type_index != selected.1.type_index
            || candidate.is_interface != selected.1.is_interface
            || candidate.descriptor != selected.1.descriptor
            || candidate.type_params != selected.1.type_params
            || candidate.interface_methods != selected.1.interface_methods
            || candidate.actor != selected.1.actor
    }) {
        return Err(obligation_error(
            location.clone(),
            format!("{label} has conflicting implementation type links"),
        ));
    }
    let canonical = matches
        .iter()
        .copied()
        .filter(|(path, export)| {
            canonical_implementation_path(export).as_deref() == Some(path.as_str())
        })
        .collect::<Vec<_>>();
    if canonical.len() != 1 {
        return Err(obligation_error(
            location.clone(),
            format!("{label} has no unique canonical implementation Type path"),
        ));
    }
    Ok(canonical[0])
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

fn require_type_symbol(
    symbol: &PackageLocalAbiSymbol,
    location: &BytecodeLinkLocation,
    label: String,
) -> Result<(), BytecodeLinkError> {
    if matches!(symbol, PackageLocalAbiSymbol::Type { .. }) {
        Ok(())
    } else {
        Err(obligation_error(
            location.clone(),
            format!("{label} is not a Type symbol"),
        ))
    }
}

fn require_type_export_matches_symbol(
    symbol: &PackageLocalAbiSymbol,
    export: &TypeExport,
    location: &BytecodeLinkLocation,
    label: String,
) -> Result<(), BytecodeLinkError> {
    let PackageLocalAbiSymbol::Type {
        descriptor,
        is_interface,
        type_params,
        interface_methods,
        actor,
        ..
    } = symbol
    else {
        return Err(obligation_error(
            location.clone(),
            format!("{label} implementation ABI entry is not a Type symbol"),
        ));
    };
    if export.descriptor.as_ref() != Some(descriptor)
        || export.is_interface != *is_interface
        || export.type_params != *type_params
        || export.interface_methods != *interface_methods
        || export.actor != *actor
    {
        return Err(obligation_error(
            location.clone(),
            format!("{label} implementation link and typed descriptor disagree"),
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
