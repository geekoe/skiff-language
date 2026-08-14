mod authority;
mod descriptor;
mod package_types;

use std::collections::BTreeSet;

use skiff_artifact_model::{
    BytecodePoolEntry, ContractTypeDescriptor, ContractTypeRef, InterfaceInstantiationRef,
    PackageBuildId, PackageSchemaTypeId, PackageSchemaTypeRecord, PackageSymbolRef,
    ResolvedPackageValueType, TypeDescriptorIr, TypeRefIr, ValidatedFunction,
    ValueLifecycleFactResolver, ValueLifecycleResolverError, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, LinkedBytecodeCandidate, SpecializationKey,
};
use skiff_runtime_loader::{HydratedBytecodePackage, HydratedDeploymentBytecode};

const SCOPE_AUTHORITY: &str = "hydratedValueLifecycle.scope";
const SOURCE_TYPE_AUTHORITY: &str = "hydratedValueLifecycle.sourceType";
const SOURCE_FUNCTION_AUTHORITY: &str = "hydratedValueLifecycle.sourceFunction";
const PACKAGE_SYMBOL_AUTHORITY: &str = "hydratedValueLifecycle.packageSymbol";
const PACKAGE_SCHEMA_AUTHORITY: &str = "hydratedValueLifecycle.packageSchema";
const INTERFACE_AUTHORITY: &str = "hydratedValueLifecycle.interface";
const CONTRACT_INTERFACE_AUTHORITY: &str = "hydratedValueLifecycle.contractInterface";

/// Exact hydration-backed resolver port for value-lifecycle policy calls.
///
/// Every linked type row must establish its exact package-build scope before
/// asking for source rows or package-owned lifecycle facts. Scope is owned by
/// the resolver rather than inferred from a package id or ABI string.
pub(crate) struct HydratedValueLifecycleResolver<'a> {
    hydrated: &'a HydratedDeploymentBytecode,
    candidate: &'a LinkedBytecodeCandidate,
    scope_package_build_id: Option<PackageBuildId>,
    row_private_type_authority: BTreeSet<PackageBuildId>,
}

impl<'a> HydratedValueLifecycleResolver<'a> {
    pub(crate) fn new(
        hydrated: &'a HydratedDeploymentBytecode,
        candidate: &'a LinkedBytecodeCandidate,
    ) -> Self {
        Self {
            hydrated,
            candidate,
            scope_package_build_id: None,
            row_private_type_authority: BTreeSet::new(),
        }
    }

    /// Starts one linked-type row at its exact artifact origin. A failed call
    /// leaves the resolver unscoped so facts can never leak from the prior row.
    pub(crate) fn begin_row(
        &mut self,
        origin: &PackageBuildId,
    ) -> Result<(), ValueLifecycleResolverError> {
        self.scope_package_build_id = None;
        self.row_private_type_authority.clear();

        let mut candidate_rows = self
            .candidate
            .packages()
            .iter()
            .filter(|row| row.package_build_id() == origin);
        let candidate_row = candidate_rows.next().ok_or_else(|| {
            resolver_error(
                SCOPE_AUTHORITY,
                format!("unknown exact origin package build {origin}"),
            )
        })?;
        if candidate_rows.next().is_some() {
            return Err(resolver_error(
                SCOPE_AUTHORITY,
                format!("ambiguous exact origin package build {origin}"),
            ));
        }

        let package = self.hydrated.packages().get(origin).ok_or_else(|| {
            resolver_error(
                SCOPE_AUTHORITY,
                format!("exact origin package build {origin} is not hydrated"),
            )
        })?;
        let exact_owner = package.reference().package_build_id == *origin
            && package.artifact().package_build_id == *origin
            && package.reference().package_id == package.artifact().package_id
            && package.reference().package_version == package.artifact().package_version
            && package.reference().package_local_abi_identity
                == package.artifact().package_local_abi.local_abi_identity
            && package
                .bytecode()
                .is_some_and(|bytecode| candidate_row.artifact_ref() == bytecode.reference());
        if !exact_owner {
            return Err(resolver_error(
                SCOPE_AUTHORITY,
                format!("exact origin package build {origin} has incomplete owner authority"),
            ));
        }

        self.scope_package_build_id = Some(origin.clone());
        Ok(())
    }

    pub(super) fn establish_row_private_type_authority(
        &mut self,
        authority: BTreeSet<PackageBuildId>,
    ) -> Result<(), ValueLifecycleResolverError> {
        if self.scope_package_build_id.is_none() {
            return Err(resolver_error(
                SCOPE_AUTHORITY,
                "cannot establish private type authority without an exact row scope",
            ));
        }
        self.row_private_type_authority = authority;
        Ok(())
    }

    pub(super) fn has_row_private_type_authority(&self, package: &HydratedBytecodePackage) -> bool {
        self.row_private_type_authority
            .contains(&package.reference().package_build_id)
    }

    pub(super) fn current_package_build_id(&self) -> Option<&PackageBuildId> {
        self.scope_package_build_id.as_ref()
    }

    /// Reads one type row only from the current package's admitted bytecode
    /// view. Raw artifact content and rows owned by another build are absent.
    pub(super) fn source_type(
        &self,
        artifact_index: ArtifactTypeIndex,
    ) -> Result<&TypeRefIr, ValueLifecycleResolverError> {
        let package = self.current_package(SOURCE_TYPE_AUTHORITY)?;
        let index = usize::try_from(artifact_index.get()).map_err(|_| {
            resolver_error(
                SOURCE_TYPE_AUTHORITY,
                "artifact type index does not fit usize",
            )
        })?;
        let bytecode = package.bytecode().ok_or_else(|| {
            resolver_error(
                SOURCE_TYPE_AUTHORITY,
                "current package is type-only and has no bytecode type pool".to_string(),
            )
        })?;
        match bytecode.view().pools().types.get(index) {
            Some(BytecodePoolEntry::TypeRef { ty }) => Ok(ty),
            Some(_) => Err(resolver_error(
                SOURCE_TYPE_AUTHORITY,
                format!(
                    "admitted artifact type row {} has the wrong pool kind",
                    artifact_index.get()
                ),
            )),
            None => Err(resolver_error(
                SOURCE_TYPE_AUTHORITY,
                format!(
                    "unknown admitted artifact type row {} in package build {}",
                    artifact_index.get(),
                    package.reference().package_build_id
                ),
            )),
        }
    }

    /// Reads the exact admitted template function for a specialization owned
    /// by the current package scope.
    pub(super) fn source_function(
        &self,
        specialization: &SpecializationKey,
    ) -> Result<&ValidatedFunction, ValueLifecycleResolverError> {
        let package = self.current_package(SOURCE_FUNCTION_AUTHORITY)?;
        if specialization.package_build_id() != &package.reference().package_build_id {
            return Err(resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                format!(
                    "specialization owner {} differs from current exact package build {}",
                    specialization.package_build_id(),
                    package.reference().package_build_id
                ),
            ));
        }

        let mut candidate_rows = self
            .candidate
            .functions()
            .iter()
            .filter(|function| function.key() == specialization);
        if candidate_rows.next().is_none() {
            return Err(resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                "unknown exact candidate specialization",
            ));
        }
        if candidate_rows.next().is_some() {
            return Err(resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                "ambiguous exact candidate specialization",
            ));
        }

        let function_key = specialization.artifact_function_key().as_str();
        let bytecode = package.bytecode().ok_or_else(|| {
            resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                "current package is type-only and has no bytecode functions".to_string(),
            )
        })?;
        let mut source_rows = bytecode
            .view()
            .functions()
            .iter()
            .filter(|function| function.function_key == function_key);
        let source = source_rows.next().ok_or_else(|| {
            resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                format!("unknown admitted artifact function {function_key:?}"),
            )
        })?;
        if source_rows.next().is_some() {
            return Err(resolver_error(
                SOURCE_FUNCTION_AUTHORITY,
                format!("ambiguous admitted artifact function {function_key:?}"),
            ));
        }
        Ok(source)
    }

    fn current_package(
        &self,
        authority: &'static str,
    ) -> Result<&HydratedBytecodePackage, ValueLifecycleResolverError> {
        let build_id = self.scope_package_build_id.as_ref().ok_or_else(|| {
            resolver_error(
                authority,
                "exact origin package scope has not been established",
            )
        })?;
        let package = self.hydrated.packages().get(build_id).ok_or_else(|| {
            resolver_error(
                authority,
                format!("current exact package build {build_id} is not hydrated"),
            )
        })?;
        if package.reference().package_build_id != *build_id
            || package.artifact().package_build_id != *build_id
            || package.reference().package_id != package.artifact().package_id
            || package.reference().package_version != package.artifact().package_version
            || package.reference().package_local_abi_identity
                != package.artifact().package_local_abi.local_abi_identity
        {
            return Err(resolver_error(
                authority,
                format!("current exact package build {build_id} has incomplete owner authority"),
            ));
        }
        Ok(package)
    }

    fn package_for_id(
        &self,
        package_id: &str,
        authority: &'static str,
    ) -> Result<&HydratedBytecodePackage, ValueLifecycleResolverError> {
        if package_id.is_empty() {
            return Err(resolver_error(authority, "package owner id is empty"));
        }

        let mut matches = self.hydrated.packages().iter().filter(|(_, package)| {
            package.reference().package_id == package_id
                || package.artifact().package_id == package_id
        });
        let (map_build_id, package) = matches.next().ok_or_else(|| {
            resolver_error(
                authority,
                format!("unknown hydrated package owner {package_id:?}"),
            )
        })?;
        if matches.next().is_some() {
            return Err(resolver_error(
                authority,
                format!("ambiguous hydrated package owner {package_id:?}"),
            ));
        }

        let exact_owner = package.reference().package_id == package_id
            && package.artifact().package_id == package_id
            && &package.reference().package_build_id == map_build_id
            && &package.artifact().package_build_id == map_build_id
            && package.reference().package_version == package.artifact().package_version
            && package.reference().package_local_abi_identity
                == package.artifact().package_local_abi.local_abi_identity;
        if !exact_owner {
            return Err(resolver_error(
                authority,
                format!("hydrated package owner {package_id:?} is incomplete"),
            ));
        }
        Ok(package)
    }
}

impl ValueLifecycleFactResolver for HydratedValueLifecycleResolver<'_> {
    fn resolve_package_symbol(
        &mut self,
        symbol: &PackageSymbolRef,
    ) -> Result<ResolvedPackageValueType, ValueLifecycleResolverError> {
        let resolved = self.resolve_package_type(symbol)?;
        Ok(ResolvedPackageValueType {
            type_parameters: resolved.type_parameters,
            descriptor: resolved.descriptor,
        })
    }

    fn resolve_package_schema(
        &mut self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<PackageSchemaTypeRecord, ValueLifecycleResolverError> {
        self.current_package(PACKAGE_SCHEMA_AUTHORITY)?;
        if stable_schema_key.is_empty() || package_schema_type_id.as_str().is_empty() {
            return Err(resolver_error(
                PACKAGE_SCHEMA_AUTHORITY,
                "package schema owner triple is incomplete",
            ));
        }
        let package = self.package_for_id(package_id, PACKAGE_SCHEMA_AUTHORITY)?;
        if package.artifact().schema_version != PACKAGE_ARTIFACT_SCHEMA_VERSION {
            return Err(resolver_error(
                PACKAGE_SCHEMA_AUTHORITY,
                format!(
                    "package schema owner {package_id:?} is not an admitted {PACKAGE_ARTIFACT_SCHEMA_VERSION} artifact"
                ),
            ));
        }
        let record = package
            .artifact()
            .bytecode_schema_records
            .get(package_schema_type_id)
            .ok_or_else(|| {
                resolver_error(
                    PACKAGE_SCHEMA_AUTHORITY,
                    format!(
                        "unknown v13 bytecode schema record {package_id}/{stable_schema_key}/{}",
                        package_schema_type_id.as_str()
                    ),
                )
            })?;
        let exact = record.package_id == package_id
            && record.stable_schema_key == stable_schema_key
            && record.package_schema_type_id == *package_schema_type_id;
        if !exact {
            return Err(resolver_error(
                PACKAGE_SCHEMA_AUTHORITY,
                format!(
                    "v13 bytecode schema record does not match exact owner triple {package_id}/{stable_schema_key}/{}",
                    package_schema_type_id.as_str()
                ),
            ));
        }
        Ok(record.clone())
    }

    fn validate_interface(
        &mut self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<(), ValueLifecycleResolverError> {
        let identity =
            serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(|error| {
                resolver_error(
                    INTERFACE_AUTHORITY,
                    format!("interface ABI is not TypeRefIr JSON: {error}"),
                )
            })?;
        let canonical = skiff_canonical_json::canonical_json_bytes(&identity).map_err(|error| {
            resolver_error(
                INTERFACE_AUTHORITY,
                format!("interface ABI canonical projection failed: {error}"),
            )
        })?;
        if canonical.as_slice() != interface.interface_abi_id.as_bytes() {
            return Err(resolver_error(
                INTERFACE_AUTHORITY,
                "interface ABI is not exact canonical TypeRefIr JSON",
            ));
        }

        let TypeRefIr::PackageSymbol { symbol } = identity else {
            return Err(resolver_error(
                INTERFACE_AUTHORITY,
                "interface ABI is not an exact PackageSymbol declaration identity",
            ));
        };
        let resolved = self.resolve_package_type(&symbol)?;
        if !resolved.is_interface || !matches!(resolved.descriptor, TypeDescriptorIr::Interface) {
            return Err(resolver_error(
                INTERFACE_AUTHORITY,
                "exact PackageSymbol does not resolve to an interface descriptor",
            ));
        }
        if resolved.type_parameters.len() != interface.canonical_type_args.len() {
            return Err(resolver_error(
                INTERFACE_AUTHORITY,
                format!(
                    "exact PackageSymbol interface arity is {}, but {} arguments were supplied",
                    resolved.type_parameters.len(),
                    interface.canonical_type_args.len()
                ),
            ));
        }
        Ok(())
    }

    fn validate_contract_interface(
        &mut self,
        interface: &ContractTypeRef,
        arguments: &[ContractTypeRef],
    ) -> Result<(), ValueLifecycleResolverError> {
        let ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } = interface
        else {
            return Err(resolver_error(
                CONTRACT_INTERFACE_AUTHORITY,
                "contract interface is not an exact PackageSchema reference",
            ));
        };
        let record =
            self.resolve_package_schema(package_id, stable_schema_key, package_schema_type_id)?;
        if !matches!(
            record.canonical_descriptor.descriptor,
            ContractTypeDescriptor::CallbackInterface { .. }
        ) {
            return Err(resolver_error(
                CONTRACT_INTERFACE_AUTHORITY,
                "exact PackageSchema record is not a CallbackInterface descriptor",
            ));
        }
        if record.canonical_descriptor.type_params.len() != arguments.len() {
            return Err(resolver_error(
                CONTRACT_INTERFACE_AUTHORITY,
                format!(
                    "exact CallbackInterface arity is {}, but {} arguments were supplied",
                    record.canonical_descriptor.type_params.len(),
                    arguments.len()
                ),
            ));
        }
        Ok(())
    }
}

pub(super) fn resolver_error(
    authority: &'static str,
    message: impl Into<String>,
) -> ValueLifecycleResolverError {
    ValueLifecycleResolverError {
        authority: authority.to_string(),
        message: message.into(),
    }
}
