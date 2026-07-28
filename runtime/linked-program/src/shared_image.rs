//! Replica-local, activation-independent hydration of a canonical package link plan.
//!
//! These types deliberately do not implement serde. Their identity is the declared
//! [`RuntimeAssembly::assembly_identity`], and their code-slot order and package edges come
//! exclusively from [`RuntimeAssembly::package_link_plan`].

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use skiff_artifact_model::{
    file_ir_package_call_sites, validate_file_ir_package_calls, validate_file_ir_service_calls,
    AssemblyIdentity, CanonicalPackageLinkPlan, ContractOperationId, FileIrRef, FileIrUnit,
    OperationTargetRef, PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId,
    PackageCallableId, PackageLocalAbi, PackageLocalAbiIdentity, PackageRefIr,
    PackageRequirementKey, PackageSchemaIndex, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageSymbolRef, RuntimeAssembly, ServiceCallRef, ServiceCallRefIndex,
    ServiceProtocolIdentity, ServiceSymbolRef, TypeRefIr,
};

use crate::{DbObjectTargetId, ExecutableAddr, FileAddr, PublicationResourceTable, UnitAddr};

/// Loader-owned immutable inputs for one canonical package code slot.
#[derive(Debug)]
pub struct HydratedPackageCode {
    artifact: Arc<PackageArtifact>,
    files: Vec<Arc<FileIrUnit>>,
    static_resources: PublicationResourceTable,
    schema_index: Option<Arc<PackageSchemaIndex>>,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl HydratedPackageCode {
    pub fn new(
        artifact: Arc<PackageArtifact>,
        files: Vec<Arc<FileIrUnit>>,
        static_resources: PublicationResourceTable,
    ) -> Self {
        Self {
            artifact,
            files,
            static_resources,
            schema_index: None,
            schema_records: BTreeMap::new(),
        }
    }

    pub fn with_schema_index(mut self, schema_index: Arc<PackageSchemaIndex>) -> Self {
        self.schema_index = Some(schema_index);
        self
    }

    pub fn with_schema_records(
        mut self,
        schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ) -> Self {
        self.schema_records = schema_records;
        self
    }

    pub fn artifact(&self) -> &Arc<PackageArtifact> {
        &self.artifact
    }
}

/// Deterministic index into `CanonicalPackageLinkPlan.code_slots`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageCodeSlotIndex(usize);

impl PackageCodeSlotIndex {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(self) -> usize {
        self.0
    }
}

/// Read-only code owned exactly once for a package build inside one replica.
#[derive(Debug)]
pub struct SharedPackageCode {
    code_slot: PackageCodeSlotIndex,
    artifact_ref: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    files: Vec<Arc<FileIrUnit>>,
    files_by_identity: BTreeMap<String, usize>,
    static_resources: PublicationResourceTable,
    schema_index: Arc<PackageSchemaIndex>,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl SharedPackageCode {
    pub fn code_slot(&self) -> PackageCodeSlotIndex {
        self.code_slot
    }

    pub fn artifact_ref(&self) -> &PackageArtifactRef {
        &self.artifact_ref
    }

    pub fn package_build_id(&self) -> &PackageBuildId {
        &self.artifact_ref.package_build_id
    }

    pub fn local_abi_identity(&self) -> &PackageLocalAbiIdentity {
        &self.artifact_ref.package_local_abi_identity
    }

    pub fn local_abi(&self) -> &PackageLocalAbi {
        &self.artifact.package_local_abi
    }

    pub fn artifact(&self) -> &PackageArtifact {
        self.artifact.as_ref()
    }

    pub fn files(&self) -> &[Arc<FileIrUnit>] {
        &self.files
    }

    pub fn file(&self, file_ir_identity: &str) -> Option<&Arc<FileIrUnit>> {
        self.files_by_identity
            .get(file_ir_identity)
            .and_then(|index| self.files.get(*index))
    }

    pub fn static_resources(&self) -> &PublicationResourceTable {
        &self.static_resources
    }

    pub fn schema_records(&self) -> &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>> {
        &self.schema_records
    }

    pub fn schema_index(&self) -> &Arc<PackageSchemaIndex> {
        &self.schema_index
    }

    pub fn callable_target(&self, callable_id: &PackageCallableId) -> Option<&OperationTargetRef> {
        self.artifact
            .callable_links
            .get(callable_id)
            .map(|fact| &fact.target)
    }
}

/// Fully resolved direct package call. It contains only package-code facts and never an
/// activation-relative service binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPackageDirectCall {
    caller_package_build_id: PackageBuildId,
    dependency_code_slot: PackageCodeSlotIndex,
    dependency_package_build_id: PackageBuildId,
    package_callable_id: PackageCallableId,
    target: OperationTargetRef,
    executable_addr: ExecutableAddr,
}

impl LinkedPackageDirectCall {
    pub fn caller_package_build_id(&self) -> &PackageBuildId {
        &self.caller_package_build_id
    }

    pub fn dependency_code_slot(&self) -> PackageCodeSlotIndex {
        self.dependency_code_slot
    }

    pub fn dependency_package_build_id(&self) -> &PackageBuildId {
        &self.dependency_package_build_id
    }

    pub fn package_callable_id(&self) -> &PackageCallableId {
        &self.package_callable_id
    }

    pub fn target(&self) -> &OperationTargetRef {
        &self.target
    }

    pub fn executable_addr(&self) -> &ExecutableAddr {
        &self.executable_addr
    }
}

/// Linked service instruction kept activation-relative by construction.
///
/// There is intentionally no provider deployment, provider package, code slot, executable, route,
/// callback table, or host handle in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationRelativeServiceCall {
    caller_package_build_id: PackageBuildId,
    service_requirement_slot: u32,
    contract_operation_id: ContractOperationId,
    expected_protocol_identity: ServiceProtocolIdentity,
}

impl ActivationRelativeServiceCall {
    pub fn caller_package_build_id(&self) -> &PackageBuildId {
        &self.caller_package_build_id
    }

    pub fn service_requirement_slot(&self) -> u32 {
        self.service_requirement_slot
    }

    pub fn operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }

    pub fn expected_protocol_identity(&self) -> &ServiceProtocolIdentity {
        &self.expected_protocol_identity
    }
}

/// Activation-independent package code hydrated from one `RuntimeAssembly` link plan.
///
/// All state is immutable after construction. Activation config, state/resource bindings,
/// service-provider bindings, callback tables, replica identity, and request state are absent.
#[derive(Debug)]
pub struct SharedPackageLinkedImage {
    assembly_identity: AssemblyIdentity,
    package_link_plan: CanonicalPackageLinkPlan,
    code_slots: Vec<Arc<SharedPackageCode>>,
    code_slot_by_build: BTreeMap<PackageBuildId, PackageCodeSlotIndex>,
    package_links: BTreeMap<PackageRequirementKey, PackageBinding>,
}

impl SharedPackageLinkedImage {
    /// Hydrates one immutable image from the exact canonical code-slot order and package edges.
    pub fn from_runtime_assembly(
        assembly: &RuntimeAssembly,
        hydrated_packages: impl IntoIterator<Item = HydratedPackageCode>,
    ) -> SharedPackageImageResult<Self> {
        let mut hydrated_by_build = BTreeMap::new();
        for hydrated in hydrated_packages {
            let build_id = hydrated.artifact.package_build_id.clone();
            if hydrated_by_build
                .insert(build_id.clone(), hydrated)
                .is_some()
            {
                return Err(SharedPackageImageError::DuplicateHydratedPackage { build_id });
            }
        }

        let mut code_slots = Vec::with_capacity(assembly.package_link_plan.code_slots.len());
        let mut code_slot_by_build = BTreeMap::new();
        for (index, slot) in assembly.package_link_plan.code_slots.iter().enumerate() {
            let build_id = slot.package.package_build_id.clone();
            let code_slot = PackageCodeSlotIndex::new(index);
            if let Some(first_slot) = code_slot_by_build.insert(build_id.clone(), code_slot) {
                return Err(SharedPackageImageError::DuplicateCodeSlotBuild {
                    build_id,
                    first_slot,
                    duplicate_slot: code_slot,
                });
            }
            let hydrated = hydrated_by_build.remove(&build_id).ok_or_else(|| {
                SharedPackageImageError::MissingHydratedPackage {
                    code_slot,
                    build_id: build_id.clone(),
                }
            })?;
            code_slots.push(Arc::new(SharedPackageCode::hydrate(
                code_slot,
                &slot.package,
                hydrated,
            )?));
        }

        if let Some((build_id, _)) = hydrated_by_build.into_iter().next() {
            return Err(SharedPackageImageError::HydratedPackageOutsideLinkPlan { build_id });
        }

        let mut package_links = BTreeMap::new();
        for binding in &assembly.package_link_plan.package_links {
            if package_links
                .insert(binding.key.clone(), binding.clone())
                .is_some()
            {
                return Err(SharedPackageImageError::DuplicatePackageLink {
                    key: binding.key.clone(),
                });
            }
        }

        let image = Self {
            assembly_identity: assembly.assembly_identity.clone(),
            package_link_plan: assembly.package_link_plan.clone(),
            code_slots,
            code_slot_by_build,
            package_links,
        };
        image.validate_package_edges()?;
        image.validate_canonical_call_sites()?;
        Ok(image)
    }

    pub fn assembly_identity(&self) -> &AssemblyIdentity {
        &self.assembly_identity
    }

    pub fn package_link_plan(&self) -> &CanonicalPackageLinkPlan {
        &self.package_link_plan
    }

    pub fn code_slots(&self) -> &[Arc<SharedPackageCode>] {
        &self.code_slots
    }

    pub fn is_empty(&self) -> bool {
        self.code_slots.is_empty()
    }

    pub fn code_by_slot(&self, slot: PackageCodeSlotIndex) -> Option<&Arc<SharedPackageCode>> {
        self.code_slots.get(slot.index())
    }

    pub fn code_by_build(
        &self,
        package_build_id: &PackageBuildId,
    ) -> Option<&Arc<SharedPackageCode>> {
        self.code_slot_by_build
            .get(package_build_id)
            .and_then(|slot| self.code_by_slot(*slot))
    }

    /// Resolves the canonical package-direct-call chain without consulting activation state.
    pub fn resolve_package_direct_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        package_ref: &PackageRefIr,
        package_callable_id: &PackageCallableId,
    ) -> SharedPackageImageResult<LinkedPackageDirectCall> {
        let PackageRefIr::Dependency { dependency_ref } = package_ref else {
            return Err(
                SharedPackageImageError::PackageDirectCallRequiresDependencyAlias {
                    caller_package_build_id: caller_package_build_id.clone(),
                },
            );
        };
        self.resolve_package_direct_call_by_alias(
            caller_package_build_id,
            dependency_ref,
            package_callable_id,
        )
    }

    /// Resolves caller build + requirement alias -> exact ref/local ABI -> code slot -> callable.
    pub fn resolve_package_direct_call_by_alias(
        &self,
        caller_package_build_id: &PackageBuildId,
        requirement_alias: &str,
        package_callable_id: &PackageCallableId,
    ) -> SharedPackageImageResult<LinkedPackageDirectCall> {
        let caller = self.required_code_by_build(caller_package_build_id)?;
        let requirement = unique_package_requirement(caller, requirement_alias)?;
        let key = PackageRequirementKey {
            caller_package_build_id: caller_package_build_id.clone(),
            package_requirement_alias: requirement_alias.to_string(),
        };
        let binding = self
            .package_links
            .get(&key)
            .ok_or_else(|| SharedPackageImageError::MissingPackageLink { key: key.clone() })?;
        validate_requirement_binding(caller, requirement, binding)?;
        let dependency_ref = &binding.package;

        let dependency = self.required_code_by_build(&dependency_ref.package_build_id)?;
        if dependency.artifact_ref() != dependency_ref {
            return Err(SharedPackageImageError::PackageLinkTargetRefMismatch {
                key,
                linked: dependency_ref.clone(),
                loaded: dependency.artifact_ref().clone(),
            });
        }
        let fact = dependency
            .artifact
            .callable_links
            .get(package_callable_id)
            .ok_or_else(|| SharedPackageImageError::MissingPackageCallable {
                dependency_package_build_id: dependency.package_build_id().clone(),
                package_callable_id: package_callable_id.clone(),
            })?;
        if fact.callable_id != *package_callable_id {
            return Err(SharedPackageImageError::CallableLinkKeyMismatch {
                package_build_id: dependency.package_build_id().clone(),
                map_key: package_callable_id.clone(),
                fact_callable_id: fact.callable_id.clone(),
            });
        }
        dependency.validate_callable_target(package_callable_id, &fact.target)?;
        let executable_addr = dependency.executable_addr(&fact.target)?;

        Ok(LinkedPackageDirectCall {
            caller_package_build_id: caller_package_build_id.clone(),
            dependency_code_slot: dependency.code_slot,
            dependency_package_build_id: dependency.package_build_id().clone(),
            package_callable_id: package_callable_id.clone(),
            target: fact.target.clone(),
            executable_addr,
        })
    }

    /// Resolves a foreign DB object through the caller's exact package edge.
    pub fn resolve_package_db_object_target(
        &self,
        caller_package_build_id: &PackageBuildId,
        symbol: &PackageSymbolRef,
    ) -> SharedPackageImageResult<DbObjectTargetId> {
        let PackageRefIr::Dependency { dependency_ref } = &symbol.package else {
            return Err(SharedPackageImageError::DbTargetRequiresDependencyAlias {
                caller_package_build_id: caller_package_build_id.clone(),
                symbol_path: symbol.symbol_path.clone(),
            });
        };
        let caller = self.required_code_by_build(caller_package_build_id)?;
        let requirement = unique_package_requirement(caller, dependency_ref)?;
        if requirement.expected_package_build.is_none() {
            return Err(SharedPackageImageError::MissingDbTargetBuildExpectation {
                caller_package_build_id: caller_package_build_id.clone(),
                alias: dependency_ref.clone(),
            });
        }
        let key = PackageRequirementKey {
            caller_package_build_id: caller_package_build_id.clone(),
            package_requirement_alias: dependency_ref.clone(),
        };
        let binding = self
            .package_links
            .get(&key)
            .ok_or_else(|| SharedPackageImageError::MissingPackageLink { key: key.clone() })?;
        validate_requirement_binding(caller, requirement, binding)?;
        let dependency = self.required_code_by_build(&binding.package.package_build_id)?;
        if dependency.artifact_ref() != &binding.package {
            return Err(SharedPackageImageError::PackageLinkTargetRefMismatch {
                key,
                linked: binding.package.clone(),
                loaded: dependency.artifact_ref().clone(),
            });
        }
        if symbol
            .abi_expectation
            .as_deref()
            .is_some_and(|expected| expected != dependency.local_abi_identity().as_str())
        {
            return Err(SharedPackageImageError::DbTargetAbiExpectationMismatch {
                caller_package_build_id: caller_package_build_id.clone(),
                symbol_path: symbol.symbol_path.clone(),
                expected: symbol.abi_expectation.clone().unwrap(),
                actual: dependency.local_abi_identity().to_string(),
            });
        }
        let export = dependency
            .artifact()
            .implementation_links
            .types
            .get(&symbol.symbol_path)
            .ok_or_else(|| SharedPackageImageError::MissingDbTargetTypeExport {
                dependency_package_build_id: dependency.package_build_id().clone(),
                symbol_path: symbol.symbol_path.clone(),
            })?;
        let file_ref = unique_semantic_artifact_file_ref(dependency, &export.file)?;
        let file = dependency.file(&file_ref.file_ir_identity).ok_or_else(|| {
            SharedPackageImageError::DbTargetFileNotLoaded {
                dependency_package_build_id: dependency.package_build_id().clone(),
                file_ir_identity: file_ref.file_ir_identity.clone(),
            }
        })?;
        validate_db_attachment(
            dependency.package_build_id(),
            file,
            export.type_index as usize,
            Some(export.symbol.as_str()),
        )?;
        Ok(DbObjectTargetId {
            package_artifact_ref: dependency.artifact_ref().clone(),
            file_ir_ref: file_ref.clone(),
            type_index: export.type_index as usize,
        })
    }

    pub fn resolve_db_object_target(
        &self,
        caller_package_build_id: &PackageBuildId,
        target_type: &TypeRefIr,
    ) -> SharedPackageImageResult<DbObjectTargetId> {
        match target_type {
            TypeRefIr::PackageSymbol { symbol } => {
                self.resolve_package_db_object_target(caller_package_build_id, symbol)
            }
            TypeRefIr::DbObjectSymbol { symbol } => {
                self.resolve_local_db_object_target(caller_package_build_id, symbol)
            }
            _ => Err(SharedPackageImageError::UnsupportedDbTargetType {
                caller_package_build_id: caller_package_build_id.clone(),
            }),
        }
    }

    pub fn validate_db_object_target_id(
        &self,
        target: &DbObjectTargetId,
    ) -> SharedPackageImageResult<()> {
        let dependency =
            self.required_code_by_build(&target.package_artifact_ref.package_build_id)?;
        if dependency.artifact_ref() != &target.package_artifact_ref {
            return Err(SharedPackageImageError::DbTargetPackageRefMismatch {
                expected: target.package_artifact_ref.clone(),
                loaded: dependency.artifact_ref().clone(),
            });
        }
        let file_ref = unique_semantic_artifact_file_ref(dependency, &target.file_ir_ref)?;
        if file_ref != &target.file_ir_ref {
            return Err(SharedPackageImageError::DbTargetFileRefMismatch {
                dependency_package_build_id: dependency.package_build_id().clone(),
                expected: file_ref.clone(),
                actual: target.file_ir_ref.clone(),
            });
        }
        let file = dependency.file(&file_ref.file_ir_identity).ok_or_else(|| {
            SharedPackageImageError::DbTargetFileNotLoaded {
                dependency_package_build_id: dependency.package_build_id().clone(),
                file_ir_identity: file_ref.file_ir_identity.clone(),
            }
        })?;
        validate_db_attachment(dependency.package_build_id(), file, target.type_index, None)
    }

    fn resolve_local_db_object_target(
        &self,
        caller_package_build_id: &PackageBuildId,
        symbol: &ServiceSymbolRef,
    ) -> SharedPackageImageResult<DbObjectTargetId> {
        let caller = self.required_code_by_build(caller_package_build_id)?;
        let mut matches = caller
            .artifact()
            .files
            .iter()
            .filter(|file_ref| file_ref.module_path == symbol.module_path)
            .filter_map(|file_ref| {
                caller.file(&file_ref.file_ir_identity).and_then(|file| {
                    file.declarations
                        .types
                        .get(&symbol.symbol)
                        .map(|declaration| (file_ref, file, declaration))
                })
            });
        let (file_ref, file, declaration) =
            matches
                .next()
                .ok_or_else(|| SharedPackageImageError::MissingLocalDbTarget {
                    caller_package_build_id: caller_package_build_id.clone(),
                    module_path: symbol.module_path.clone(),
                    symbol: symbol.symbol.clone(),
                })?;
        if matches.next().is_some() {
            return Err(SharedPackageImageError::AmbiguousLocalDbTarget {
                caller_package_build_id: caller_package_build_id.clone(),
                module_path: symbol.module_path.clone(),
                symbol: symbol.symbol.clone(),
            });
        }
        validate_db_attachment(
            caller.package_build_id(),
            file,
            declaration.type_index as usize,
            Some(symbol.symbol.as_str()),
        )?;
        Ok(DbObjectTargetId {
            package_artifact_ref: caller.artifact_ref().clone(),
            file_ir_ref: file_ref.clone(),
            type_index: declaration.type_index as usize,
        })
    }

    /// Preserves a canonical service-call ref as an activation-relative linked instruction.
    pub fn resolve_activation_relative_service_call(
        &self,
        caller_package_build_id: &PackageBuildId,
        caller_file_ir_identity: &str,
        service_call_ref_index: ServiceCallRefIndex,
    ) -> SharedPackageImageResult<ActivationRelativeServiceCall> {
        let caller = self.required_code_by_build(caller_package_build_id)?;
        let file = caller.file(caller_file_ir_identity).ok_or_else(|| {
            SharedPackageImageError::CallerFileNotLoaded {
                caller_package_build_id: caller_package_build_id.clone(),
                file_ir_identity: caller_file_ir_identity.to_string(),
            }
        })?;
        let service_call = file
            .external_refs
            .service_call_ref(service_call_ref_index)
            .ok_or_else(|| SharedPackageImageError::ServiceCallRefOutOfBounds {
                caller_package_build_id: caller_package_build_id.clone(),
                file_ir_identity: caller_file_ir_identity.to_string(),
                index: service_call_ref_index.index(),
                ref_count: file.external_refs.service_call_refs.len(),
            })?;
        validate_service_call_requirement(caller, service_call)?;
        Ok(ActivationRelativeServiceCall {
            caller_package_build_id: caller_package_build_id.clone(),
            service_requirement_slot: service_call.service_requirement_slot,
            contract_operation_id: service_call.contract_operation_id.clone(),
            expected_protocol_identity: service_call.expected_protocol_identity.clone(),
        })
    }

    fn required_code_by_build(
        &self,
        package_build_id: &PackageBuildId,
    ) -> SharedPackageImageResult<&SharedPackageCode> {
        self.code_by_build(package_build_id)
            .map(Arc::as_ref)
            .ok_or_else(|| SharedPackageImageError::PackageBuildNotLoaded {
                build_id: package_build_id.clone(),
            })
    }

    fn validate_package_edges(&self) -> SharedPackageImageResult<()> {
        for (key, binding) in &self.package_links {
            let caller = self.required_code_by_build(&key.caller_package_build_id)?;
            let requirement = unique_package_requirement(caller, &key.package_requirement_alias)?;
            validate_requirement_binding(caller, requirement, binding)?;
            let dependency = self.required_code_by_build(&binding.package.package_build_id)?;
            if dependency.artifact_ref() != &binding.package {
                return Err(SharedPackageImageError::PackageLinkTargetRefMismatch {
                    key: key.clone(),
                    linked: binding.package.clone(),
                    loaded: dependency.artifact_ref().clone(),
                });
            }
        }

        for caller in &self.code_slots {
            let mut aliases = BTreeSet::new();
            for requirement in &caller.artifact.package_requirements {
                if !aliases.insert(requirement.alias.as_str()) {
                    return Err(SharedPackageImageError::DuplicatePackageRequirementAlias {
                        caller_package_build_id: caller.package_build_id().clone(),
                        alias: requirement.alias.clone(),
                    });
                }
                let key = PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id().clone(),
                    package_requirement_alias: requirement.alias.clone(),
                };
                if !self.package_links.contains_key(&key) {
                    return Err(SharedPackageImageError::MissingPackageLink { key });
                }
            }
        }
        Ok(())
    }

    fn validate_canonical_call_sites(&self) -> SharedPackageImageResult<()> {
        for caller in &self.code_slots {
            for file in caller.files() {
                validate_file_ir_package_calls(file).map_err(|error| {
                    SharedPackageImageError::InvalidPackageCallTable {
                        caller_package_build_id: caller.package_build_id().clone(),
                        file_ir_identity: file.file_ir_identity.clone(),
                        detail: error.to_string(),
                    }
                })?;
                validate_file_ir_service_calls(file).map_err(|error| {
                    SharedPackageImageError::InvalidServiceCallTable {
                        caller_package_build_id: caller.package_build_id().clone(),
                        file_ir_identity: file.file_ir_identity.clone(),
                        detail: error.to_string(),
                    }
                })?;

                for site in file_ir_package_call_sites(file) {
                    self.resolve_package_direct_call(
                        caller.package_build_id(),
                        &site.package_ref,
                        &site.package_callable_id,
                    )?;
                }
                for (index, service_call) in file.external_refs.service_call_refs.iter().enumerate()
                {
                    validate_service_call_requirement(caller, service_call)?;
                    let index = ServiceCallRefIndex::try_from(index).map_err(|_| {
                        SharedPackageImageError::ServiceCallRefIndexOverflow {
                            caller_package_build_id: caller.package_build_id().clone(),
                            file_ir_identity: file.file_ir_identity.clone(),
                            index,
                        }
                    })?;
                    self.resolve_activation_relative_service_call(
                        caller.package_build_id(),
                        &file.file_ir_identity,
                        index,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl SharedPackageCode {
    pub fn executable_addr(
        &self,
        target: &OperationTargetRef,
    ) -> SharedPackageImageResult<ExecutableAddr> {
        let file_index = self
            .files_by_identity
            .get(&target.file_ref.file_ir_identity)
            .copied()
            .ok_or_else(|| SharedPackageImageError::ExecutableTargetFileNotLoaded {
                package_build_id: self.package_build_id().clone(),
                file_ir_identity: target.file_ref.file_ir_identity.clone(),
            })?;
        let expected_file_ref = self
            .artifact
            .files
            .get(file_index)
            .expect("hydrated files preserve artifact file order");
        let file = self
            .files
            .get(file_index)
            .expect("hydrated files preserve artifact file order");
        if !semantic_file_ref_matches_loaded(&target.file_ref, file) {
            return Err(SharedPackageImageError::ExecutableTargetFileRefMismatch {
                package_build_id: self.package_build_id().clone(),
                expected: expected_file_ref.clone(),
                actual: target.file_ref.clone(),
            });
        }
        let executable = target.executable_index as usize;
        if executable >= file.executables.len() {
            return Err(SharedPackageImageError::ExecutableTargetOutOfBounds {
                package_build_id: self.package_build_id().clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                executable_index: target.executable_index,
                executable_count: file.executables.len(),
            });
        }
        Ok(ExecutableAddr {
            unit: UnitAddr::Package(self.code_slot.index()),
            file: FileAddr::LoadedFileIndex(file_index),
            executable,
        })
    }

    fn hydrate(
        code_slot: PackageCodeSlotIndex,
        expected_ref: &PackageArtifactRef,
        hydrated: HydratedPackageCode,
    ) -> SharedPackageImageResult<Self> {
        validate_artifact_ref(expected_ref, &hydrated.artifact)?;

        let mut loaded_files = BTreeMap::new();
        for file in hydrated.files {
            let identity = file.file_ir_identity.clone();
            if loaded_files.insert(identity.clone(), file).is_some() {
                return Err(SharedPackageImageError::DuplicateHydratedFile {
                    package_build_id: expected_ref.package_build_id.clone(),
                    file_ir_identity: identity,
                });
            }
        }

        let mut files = Vec::with_capacity(hydrated.artifact.files.len());
        let mut files_by_identity = BTreeMap::new();
        let mut expected_file_ids = BTreeSet::new();
        for expected_file in &hydrated.artifact.files {
            if !expected_file_ids.insert(expected_file.file_ir_identity.as_str()) {
                return Err(SharedPackageImageError::DuplicateArtifactFileRef {
                    package_build_id: expected_ref.package_build_id.clone(),
                    file_ir_identity: expected_file.file_ir_identity.clone(),
                });
            }
            let file = loaded_files
                .remove(&expected_file.file_ir_identity)
                .ok_or_else(|| SharedPackageImageError::MissingHydratedFile {
                    package_build_id: expected_ref.package_build_id.clone(),
                    file_ir_identity: expected_file.file_ir_identity.clone(),
                })?;
            validate_file_ref(expected_ref, expected_file, &file)?;
            files_by_identity.insert(expected_file.file_ir_identity.clone(), files.len());
            files.push(file);
        }
        if let Some((file_ir_identity, _)) = loaded_files.into_iter().next() {
            return Err(SharedPackageImageError::HydratedFileOutsideArtifact {
                package_build_id: expected_ref.package_build_id.clone(),
                file_ir_identity,
            });
        }

        validate_static_resources(expected_ref, &hydrated.artifact, &hydrated.static_resources)?;
        let schema_index = hydrated.schema_index.ok_or_else(|| {
            SharedPackageImageError::MissingHydratedSchemaIndex {
                package_build_id: expected_ref.package_build_id.clone(),
            }
        })?;
        if schema_index.package_id != hydrated.artifact.package_schema_index.package_id
            || schema_index.package_schema_index_identity
                != hydrated
                    .artifact
                    .package_schema_index
                    .package_schema_index_identity
        {
            return Err(SharedPackageImageError::HydratedSchemaIndexMismatch {
                package_build_id: expected_ref.package_build_id.clone(),
            });
        }

        let code = Self {
            code_slot,
            artifact_ref: expected_ref.clone(),
            artifact: hydrated.artifact,
            files,
            files_by_identity,
            static_resources: hydrated.static_resources,
            schema_index,
            schema_records: hydrated.schema_records,
        };
        for (callable_id, fact) in &code.artifact.callable_links {
            if *callable_id != fact.callable_id {
                return Err(SharedPackageImageError::CallableLinkKeyMismatch {
                    package_build_id: code.package_build_id().clone(),
                    map_key: callable_id.clone(),
                    fact_callable_id: fact.callable_id.clone(),
                });
            }
            code.validate_callable_target(callable_id, &fact.target)?;
        }
        validate_package_service_call_aggregate(&code)?;
        Ok(code)
    }

    fn validate_callable_target(
        &self,
        callable_id: &PackageCallableId,
        target: &OperationTargetRef,
    ) -> SharedPackageImageResult<()> {
        if target.callable_abi_id != callable_id.as_str() {
            return Err(SharedPackageImageError::CallableAbiMismatch {
                package_build_id: self.package_build_id().clone(),
                package_callable_id: callable_id.clone(),
                target_callable_abi_id: target.callable_abi_id.clone(),
            });
        }
        let file = self
            .file(&target.file_ref.file_ir_identity)
            .ok_or_else(|| SharedPackageImageError::CallableTargetFileNotLoaded {
                package_build_id: self.package_build_id().clone(),
                package_callable_id: callable_id.clone(),
                file_ir_identity: target.file_ref.file_ir_identity.clone(),
            })?;
        let expected_file_ref = self
            .artifact
            .files
            .iter()
            .find(|candidate| candidate.file_ir_identity == target.file_ref.file_ir_identity)
            .expect("loaded file identities were validated from artifact refs");
        if !semantic_file_ref_matches_loaded(&target.file_ref, file) {
            return Err(SharedPackageImageError::CallableTargetFileRefMismatch {
                package_build_id: self.package_build_id().clone(),
                package_callable_id: callable_id.clone(),
                expected: expected_file_ref.clone(),
                actual: target.file_ref.clone(),
            });
        }
        if target.executable_index as usize >= file.executables.len() {
            return Err(
                SharedPackageImageError::CallableTargetExecutableOutOfBounds {
                    package_build_id: self.package_build_id().clone(),
                    package_callable_id: callable_id.clone(),
                    file_ir_identity: file.file_ir_identity.clone(),
                    executable_index: target.executable_index,
                    executable_count: file.executables.len(),
                },
            );
        }
        Ok(())
    }
}

fn validate_artifact_ref(
    expected: &PackageArtifactRef,
    artifact: &PackageArtifact,
) -> SharedPackageImageResult<()> {
    let actual = PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    };
    if &actual != expected {
        return Err(SharedPackageImageError::HydratedArtifactRefMismatch {
            expected: expected.clone(),
            actual,
        });
    }
    Ok(())
}

fn unique_semantic_artifact_file_ref<'a>(
    dependency: &'a SharedPackageCode,
    candidate: &FileIrRef,
) -> SharedPackageImageResult<&'a FileIrRef> {
    let mut matches = dependency.artifact().files.iter().filter(|expected| {
        expected.file_ir_identity == candidate.file_ir_identity
            && expected.module_path == candidate.module_path
            && expected.source_ast_hash == candidate.source_ast_hash
    });
    let file_ref =
        matches
            .next()
            .ok_or_else(|| SharedPackageImageError::DbTargetFileRefOutsideArtifact {
                dependency_package_build_id: dependency.package_build_id().clone(),
                actual: candidate.clone(),
            })?;
    if matches.next().is_some() {
        return Err(SharedPackageImageError::DbTargetFileRefAmbiguous {
            dependency_package_build_id: dependency.package_build_id().clone(),
            actual: candidate.clone(),
        });
    }
    Ok(file_ref)
}

fn validate_db_attachment(
    package_build_id: &PackageBuildId,
    file: &FileIrUnit,
    type_index: usize,
    expected_symbol: Option<&str>,
) -> SharedPackageImageResult<()> {
    if type_index >= file.type_table.len() {
        return Err(SharedPackageImageError::DbTargetTypeOutOfBounds {
            dependency_package_build_id: package_build_id.clone(),
            file_ir_identity: file.file_ir_identity.clone(),
            type_index,
            type_count: file.type_table.len(),
        });
    }
    let mut declarations = file
        .declarations
        .types
        .iter()
        .filter(|(symbol, declaration)| {
            declaration.type_index as usize == type_index
                && expected_symbol.is_none_or(|expected| {
                    symbol.as_str() == expected && declaration.symbol == expected
                })
        });
    let (symbol, declaration) = declarations.next().ok_or_else(|| {
        SharedPackageImageError::MissingDbTargetTypeDeclaration {
            dependency_package_build_id: package_build_id.clone(),
            file_ir_identity: file.file_ir_identity.clone(),
            type_index,
        }
    })?;
    if declarations.next().is_some() || declaration.symbol != *symbol {
        return Err(SharedPackageImageError::AmbiguousDbTargetTypeDeclaration {
            dependency_package_build_id: package_build_id.clone(),
            file_ir_identity: file.file_ir_identity.clone(),
            type_index,
        });
    }
    let db = file.declarations.db.get(symbol).ok_or_else(|| {
        SharedPackageImageError::MissingDbTargetAttachment {
            dependency_package_build_id: package_build_id.clone(),
            file_ir_identity: file.file_ir_identity.clone(),
            type_index,
        }
    })?;
    if db.type_ref
        != (skiff_artifact_model::TypeRefIr::LocalType {
            type_index: type_index as u32,
        })
    {
        return Err(SharedPackageImageError::DbTargetAttachmentTypeMismatch {
            dependency_package_build_id: package_build_id.clone(),
            file_ir_identity: file.file_ir_identity.clone(),
            type_index,
        });
    }
    Ok(())
}

fn validate_file_ref(
    package: &PackageArtifactRef,
    expected: &FileIrRef,
    file: &FileIrUnit,
) -> SharedPackageImageResult<()> {
    if !semantic_file_ref_matches_loaded(expected, file) {
        return Err(SharedPackageImageError::HydratedFileRefMismatch {
            package_build_id: package.package_build_id.clone(),
            expected: expected.clone(),
            loaded_module_path: file.module_path.clone(),
            loaded_source_ast_hash: file.source_ast_hash.clone(),
        });
    }
    Ok(())
}

/// Matches semantic File IR facts; `artifact_path` remains a loader-owned record locator.
fn semantic_file_ref_matches_loaded(reference: &FileIrRef, file: &FileIrUnit) -> bool {
    reference.file_ir_identity == file.file_ir_identity
        && reference.module_path == file.module_path
        && reference
            .source_ast_hash
            .as_deref()
            .is_none_or(|hash| hash == file.source_ast_hash)
}

fn validate_static_resources(
    package: &PackageArtifactRef,
    artifact: &PackageArtifact,
    resources: &PublicationResourceTable,
) -> SharedPackageImageResult<()> {
    let mut expected_paths = BTreeSet::new();
    for expected in &artifact.static_resources {
        if !expected_paths.insert(expected.path.as_str()) {
            return Err(SharedPackageImageError::DuplicateStaticResourceRef {
                package_build_id: package.package_build_id.clone(),
                path: expected.path.clone(),
            });
        }
        let loaded = resources.get(&expected.path).ok_or_else(|| {
            SharedPackageImageError::MissingHydratedStaticResource {
                package_build_id: package.package_build_id.clone(),
                path: expected.path.clone(),
            }
        })?;
        if &loaded.meta != expected || loaded.bytes.len() as u64 != expected.byte_len {
            return Err(SharedPackageImageError::HydratedStaticResourceMismatch {
                package_build_id: package.package_build_id.clone(),
                path: expected.path.clone(),
            });
        }
    }
    if let Some(path) = resources
        .resources_by_path
        .keys()
        .find(|path| !expected_paths.contains(path.as_str()))
    {
        return Err(
            SharedPackageImageError::HydratedStaticResourceOutsideArtifact {
                package_build_id: package.package_build_id.clone(),
                path: path.clone(),
            },
        );
    }
    Ok(())
}

fn validate_package_service_call_aggregate(
    code: &SharedPackageCode,
) -> SharedPackageImageResult<()> {
    let declared = code
        .artifact
        .service_call_refs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if declared.len() != code.artifact.service_call_refs.len() {
        return Err(SharedPackageImageError::DuplicatePackageServiceCallRef {
            caller_package_build_id: code.package_build_id().clone(),
        });
    }
    let mut loaded = BTreeSet::new();
    for file in code.files() {
        validate_file_ir_service_calls(file).map_err(|error| {
            SharedPackageImageError::InvalidServiceCallTable {
                caller_package_build_id: code.package_build_id().clone(),
                file_ir_identity: file.file_ir_identity.clone(),
                detail: error.to_string(),
            }
        })?;
        loaded.extend(file.external_refs.service_call_refs.iter().cloned());
    }
    if loaded != declared {
        return Err(
            SharedPackageImageError::PackageServiceCallAggregateMismatch {
                caller_package_build_id: code.package_build_id().clone(),
            },
        );
    }
    Ok(())
}

fn unique_package_requirement<'a>(
    caller: &'a SharedPackageCode,
    alias: &str,
) -> SharedPackageImageResult<&'a skiff_artifact_model::PackageRequirement> {
    let mut matches = caller
        .artifact
        .package_requirements
        .iter()
        .filter(|requirement| requirement.alias == alias);
    let Some(requirement) = matches.next() else {
        return Err(SharedPackageImageError::MissingPackageRequirement {
            caller_package_build_id: caller.package_build_id().clone(),
            alias: alias.to_string(),
        });
    };
    if matches.next().is_some() {
        return Err(SharedPackageImageError::DuplicatePackageRequirementAlias {
            caller_package_build_id: caller.package_build_id().clone(),
            alias: alias.to_string(),
        });
    }
    Ok(requirement)
}

fn validate_requirement_binding(
    caller: &SharedPackageCode,
    requirement: &skiff_artifact_model::PackageRequirement,
    binding: &PackageBinding,
) -> SharedPackageImageResult<()> {
    let key = PackageRequirementKey {
        caller_package_build_id: caller.package_build_id().clone(),
        package_requirement_alias: requirement.alias.clone(),
    };
    if requirement.collection_name_mapping != binding.collection_name_mapping {
        return Err(
            SharedPackageImageError::PackageRequirementCollectionMappingMismatch {
                key,
                expected: requirement.collection_name_mapping.clone(),
                actual: binding.collection_name_mapping.clone(),
            },
        );
    }
    let dependency = &binding.package;
    if requirement.package_id != dependency.package_id
        || requirement.exact_version != dependency.package_version
    {
        return Err(
            SharedPackageImageError::PackageRequirementCoordinateMismatch {
                key,
                expected_package_id: requirement.package_id.clone(),
                expected_version: requirement.exact_version.clone(),
                linked: dependency.clone(),
            },
        );
    }
    if requirement.expected_local_abi != dependency.package_local_abi_identity {
        return Err(
            SharedPackageImageError::PackageRequirementLocalAbiMismatch {
                key,
                expected: requirement.expected_local_abi.clone(),
                actual: dependency.package_local_abi_identity.clone(),
            },
        );
    }
    if let Some(expected) = &requirement.expected_package_build {
        if expected != &dependency.package_build_id {
            return Err(SharedPackageImageError::PackageRequirementBuildMismatch {
                key,
                expected: expected.clone(),
                actual: dependency.package_build_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_service_call_requirement(
    caller: &SharedPackageCode,
    service_call: &ServiceCallRef,
) -> SharedPackageImageResult<()> {
    let mut requirements = caller
        .artifact
        .service_requirements
        .iter()
        .filter(|requirement| {
            requirement.service_binding_slot == service_call.service_requirement_slot
        });
    let Some(requirement) = requirements.next() else {
        return Err(SharedPackageImageError::MissingServiceRequirement {
            caller_package_build_id: caller.package_build_id().clone(),
            service_requirement_slot: service_call.service_requirement_slot,
        });
    };
    if requirements.next().is_some() {
        return Err(SharedPackageImageError::DuplicateServiceRequirementSlot {
            caller_package_build_id: caller.package_build_id().clone(),
            service_requirement_slot: service_call.service_requirement_slot,
        });
    }
    if requirement.contract_requirement.expected_protocol_identity
        != service_call.expected_protocol_identity
    {
        return Err(SharedPackageImageError::ServiceCallProtocolMismatch {
            caller_package_build_id: caller.package_build_id().clone(),
            service_requirement_slot: service_call.service_requirement_slot,
            expected: requirement
                .contract_requirement
                .expected_protocol_identity
                .clone(),
            actual: service_call.expected_protocol_identity.clone(),
        });
    }
    if !requirement
        .used_operations
        .contains(&service_call.contract_operation_id)
    {
        return Err(SharedPackageImageError::ServiceCallOperationNotDeclared {
            caller_package_build_id: caller.package_build_id().clone(),
            service_requirement_slot: service_call.service_requirement_slot,
            contract_operation_id: service_call.contract_operation_id.clone(),
        });
    }
    Ok(())
}

pub type SharedPackageImageResult<T> = Result<T, SharedPackageImageError>;

/// Fail-closed image hydration and lookup errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedPackageImageError {
    DuplicateHydratedPackage {
        build_id: PackageBuildId,
    },
    DuplicateCodeSlotBuild {
        build_id: PackageBuildId,
        first_slot: PackageCodeSlotIndex,
        duplicate_slot: PackageCodeSlotIndex,
    },
    MissingHydratedPackage {
        code_slot: PackageCodeSlotIndex,
        build_id: PackageBuildId,
    },
    HydratedPackageOutsideLinkPlan {
        build_id: PackageBuildId,
    },
    HydratedArtifactRefMismatch {
        expected: PackageArtifactRef,
        actual: PackageArtifactRef,
    },
    DuplicateHydratedFile {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    DuplicateArtifactFileRef {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    MissingHydratedFile {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    HydratedFileOutsideArtifact {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    HydratedFileRefMismatch {
        package_build_id: PackageBuildId,
        expected: FileIrRef,
        loaded_module_path: String,
        loaded_source_ast_hash: String,
    },
    DuplicateStaticResourceRef {
        package_build_id: PackageBuildId,
        path: String,
    },
    MissingHydratedStaticResource {
        package_build_id: PackageBuildId,
        path: String,
    },
    HydratedStaticResourceMismatch {
        package_build_id: PackageBuildId,
        path: String,
    },
    HydratedStaticResourceOutsideArtifact {
        package_build_id: PackageBuildId,
        path: String,
    },
    MissingHydratedSchemaIndex {
        package_build_id: PackageBuildId,
    },
    HydratedSchemaIndexMismatch {
        package_build_id: PackageBuildId,
    },
    DuplicatePackageLink {
        key: PackageRequirementKey,
    },
    PackageBuildNotLoaded {
        build_id: PackageBuildId,
    },
    MissingPackageRequirement {
        caller_package_build_id: PackageBuildId,
        alias: String,
    },
    DuplicatePackageRequirementAlias {
        caller_package_build_id: PackageBuildId,
        alias: String,
    },
    MissingPackageLink {
        key: PackageRequirementKey,
    },
    PackageRequirementCoordinateMismatch {
        key: PackageRequirementKey,
        expected_package_id: String,
        expected_version: String,
        linked: PackageArtifactRef,
    },
    PackageRequirementLocalAbiMismatch {
        key: PackageRequirementKey,
        expected: PackageLocalAbiIdentity,
        actual: PackageLocalAbiIdentity,
    },
    PackageRequirementBuildMismatch {
        key: PackageRequirementKey,
        expected: PackageBuildId,
        actual: PackageBuildId,
    },
    PackageRequirementCollectionMappingMismatch {
        key: PackageRequirementKey,
        expected: BTreeMap<String, String>,
        actual: BTreeMap<String, String>,
    },
    PackageLinkTargetRefMismatch {
        key: PackageRequirementKey,
        linked: PackageArtifactRef,
        loaded: PackageArtifactRef,
    },
    PackageDirectCallRequiresDependencyAlias {
        caller_package_build_id: PackageBuildId,
    },
    DbTargetRequiresDependencyAlias {
        caller_package_build_id: PackageBuildId,
        symbol_path: String,
    },
    MissingDbTargetBuildExpectation {
        caller_package_build_id: PackageBuildId,
        alias: String,
    },
    UnsupportedDbTargetType {
        caller_package_build_id: PackageBuildId,
    },
    MissingLocalDbTarget {
        caller_package_build_id: PackageBuildId,
        module_path: String,
        symbol: String,
    },
    AmbiguousLocalDbTarget {
        caller_package_build_id: PackageBuildId,
        module_path: String,
        symbol: String,
    },
    DbTargetAbiExpectationMismatch {
        caller_package_build_id: PackageBuildId,
        symbol_path: String,
        expected: String,
        actual: String,
    },
    MissingDbTargetTypeExport {
        dependency_package_build_id: PackageBuildId,
        symbol_path: String,
    },
    DbTargetPackageRefMismatch {
        expected: PackageArtifactRef,
        loaded: PackageArtifactRef,
    },
    DbTargetFileRefOutsideArtifact {
        dependency_package_build_id: PackageBuildId,
        actual: FileIrRef,
    },
    DbTargetFileRefAmbiguous {
        dependency_package_build_id: PackageBuildId,
        actual: FileIrRef,
    },
    DbTargetFileRefMismatch {
        dependency_package_build_id: PackageBuildId,
        expected: FileIrRef,
        actual: FileIrRef,
    },
    DbTargetFileNotLoaded {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    DbTargetTypeOutOfBounds {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
        type_count: usize,
    },
    MissingDbTargetTypeDeclaration {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
    },
    AmbiguousDbTargetTypeDeclaration {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
    },
    MissingDbTargetAttachment {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
    },
    DbTargetAttachmentTypeMismatch {
        dependency_package_build_id: PackageBuildId,
        file_ir_identity: String,
        type_index: usize,
    },
    MissingPackageCallable {
        dependency_package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
    },
    CallableLinkKeyMismatch {
        package_build_id: PackageBuildId,
        map_key: PackageCallableId,
        fact_callable_id: PackageCallableId,
    },
    CallableAbiMismatch {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
        target_callable_abi_id: String,
    },
    CallableTargetFileNotLoaded {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
        file_ir_identity: String,
    },
    CallableTargetFileRefMismatch {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
        expected: FileIrRef,
        actual: FileIrRef,
    },
    CallableTargetExecutableOutOfBounds {
        package_build_id: PackageBuildId,
        package_callable_id: PackageCallableId,
        file_ir_identity: String,
        executable_index: u32,
        executable_count: usize,
    },
    ExecutableTargetFileNotLoaded {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    ExecutableTargetFileRefMismatch {
        package_build_id: PackageBuildId,
        expected: FileIrRef,
        actual: FileIrRef,
    },
    ExecutableTargetOutOfBounds {
        package_build_id: PackageBuildId,
        file_ir_identity: String,
        executable_index: u32,
        executable_count: usize,
    },
    InvalidPackageCallTable {
        caller_package_build_id: PackageBuildId,
        file_ir_identity: String,
        detail: String,
    },
    InvalidServiceCallTable {
        caller_package_build_id: PackageBuildId,
        file_ir_identity: String,
        detail: String,
    },
    DuplicatePackageServiceCallRef {
        caller_package_build_id: PackageBuildId,
    },
    PackageServiceCallAggregateMismatch {
        caller_package_build_id: PackageBuildId,
    },
    CallerFileNotLoaded {
        caller_package_build_id: PackageBuildId,
        file_ir_identity: String,
    },
    ServiceCallRefOutOfBounds {
        caller_package_build_id: PackageBuildId,
        file_ir_identity: String,
        index: u32,
        ref_count: usize,
    },
    ServiceCallRefIndexOverflow {
        caller_package_build_id: PackageBuildId,
        file_ir_identity: String,
        index: usize,
    },
    MissingServiceRequirement {
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
    },
    DuplicateServiceRequirementSlot {
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
    },
    ServiceCallProtocolMismatch {
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
        expected: ServiceProtocolIdentity,
        actual: ServiceProtocolIdentity,
    },
    ServiceCallOperationNotDeclared {
        caller_package_build_id: PackageBuildId,
        service_requirement_slot: u32,
        contract_operation_id: ContractOperationId,
    },
}

impl fmt::Display for SharedPackageImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "shared package linked-image validation failed: {self:?}"
        )
    }
}

impl std::error::Error for SharedPackageImageError {}

#[cfg(test)]
mod tests;
