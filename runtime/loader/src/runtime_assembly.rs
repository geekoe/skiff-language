use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_model::{
    package_schema_descriptor_refs, BoundaryCallbackContract, BoundaryOperationDescriptor,
    BoundaryStreamContract, ContractOperationId, ContractTypeDescriptor, ContractTypeRef,
    FileIrRef, FileIrUnit, GatewayEntryKey, IngressSelector, PackageArtifact, PackageArtifactRef,
    PackageBuildId, PackageSchemaIndex, PackageSchemaIndexRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord, PackageSchemaTypeRecordRef, PublicationResourceRef, RuntimeAssembly,
    RuntimeAssemblyRef, ServiceContract, ServiceContractRef, ServiceDeployment,
    ServiceDeploymentRef,
};

mod content_validation;
mod gateway_ingress;
mod graph_validation;

use content_validation::{
    validate_assembly, validate_contract_ref, validate_file_ref, validate_file_ref_path,
    validate_package_file_targets, validate_package_ref, validate_resource_content,
    validate_resource_ref_path,
};
use gateway_ingress::hydrate_gateway_ingress;
pub use gateway_ingress::{HydratedGatewayCallable, HydratedGatewayEntry};
use graph_validation::validate_hydrated_graph;

/// Trusted content-addressed storage boundary used by the typed assembly loader.
///
/// Every method receives an exact typed reference from a validated
/// [`RuntimeAssembly`] or [`PackageArtifact`]. Implementations may map those
/// references to storage paths, but the loader never reconstructs semantic
/// coordinates from a path or display name.
pub trait RuntimeAssemblyContentResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>>;

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>>;

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        anyhow::bail!(
            "runtime assembly resolver does not implement exact PackageSchemaIndex lookup for {reference:?}"
        )
    }

    fn resolve_package_schema_type(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>>;

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>>;

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>>;

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>>;
}

/// Production resolver boundary for the root immutable assembly record.
///
/// Keeping the root lookup typed prevents control-plane code from recovering an
/// assembly through a display coordinate, pointer graph, or artifact-root scan.
pub trait RuntimeAssemblyRecordResolver: RuntimeAssemblyContentResolver {
    fn resolve_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>>;
}

/// Immutable canonical contract store retained after assembly hydration.
#[derive(Debug, Default)]
pub struct ServiceContractStore {
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    schemas: BTreeMap<ServiceContractRef, Arc<ResolvedServiceSchema>>,
    shared_schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl ServiceContractStore {
    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    pub fn contract(&self, reference: &ServiceContractRef) -> Option<&Arc<ServiceContract>> {
        self.contracts.get(reference)
    }

    pub fn resolved_schema(
        &self,
        reference: &ServiceContractRef,
    ) -> Option<&Arc<ResolvedServiceSchema>> {
        self.schemas.get(reference)
    }

    pub fn shared_schema_record(
        &self,
        type_id: &PackageSchemaTypeId,
    ) -> Option<&Arc<PackageSchemaTypeRecord>> {
        self.shared_schema_records.get(type_id)
    }

    /// Typed operation lookup. The returned canonical descriptor owns all
    /// parameter, return, error and stream value plans; activation templates
    /// retain only the contract ref and operation id.
    pub fn operation_descriptor(
        &self,
        contract: &ServiceContractRef,
        operation: &ContractOperationId,
    ) -> Option<&BoundaryOperationDescriptor> {
        self.contracts
            .get(contract)
            .and_then(|contract| contract.operations.get(operation))
    }

    pub fn operation(
        &self,
        contract: &ServiceContractRef,
        operation: &ContractOperationId,
    ) -> Option<&BoundaryOperationDescriptor> {
        self.operation_descriptor(contract, operation)
    }

    pub fn contracts(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ServiceContractRef, &Arc<ServiceContract>)> {
        self.contracts.iter()
    }
}

/// The exact immutable Package-owned type closure admitted for one contract.
///
/// Records may share their payload allocation with other admitted contracts,
/// while membership remains contract-local and was validated before this value
/// became observable.
#[derive(Debug)]
pub struct ResolvedServiceSchema {
    contract: ServiceContractRef,
    records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl ResolvedServiceSchema {
    pub fn contract(&self) -> &ServiceContractRef {
        &self.contract
    }

    pub fn record(&self, type_id: &PackageSchemaTypeId) -> Option<&Arc<PackageSchemaTypeRecord>> {
        self.records.get(type_id)
    }

    pub fn records(
        &self,
    ) -> impl ExactSizeIterator<Item = (&PackageSchemaTypeId, &Arc<PackageSchemaTypeRecord>)> {
        self.records.iter()
    }
}

#[derive(Debug, Clone)]
pub struct HydratedStaticResource {
    reference: PublicationResourceRef,
    bytes: Arc<[u8]>,
}

impl HydratedStaticResource {
    pub fn reference(&self) -> &PublicationResourceRef {
        &self.reference
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
}

/// One deterministic package-code slot. Its artifact, File IR and resources
/// are immutable and may be shared by every activation in the replica.
#[derive(Debug)]
pub struct HydratedPackageCodeSlot {
    reference: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    files: Vec<Arc<FileIrUnit>>,
    file_slots: BTreeMap<String, usize>,
    resources: Vec<HydratedStaticResource>,
    resource_slots: BTreeMap<String, usize>,
    schema_index: Arc<PackageSchemaIndex>,
    schema_records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
}

impl HydratedPackageCodeSlot {
    pub fn reference(&self) -> &PackageArtifactRef {
        &self.reference
    }

    pub fn artifact(&self) -> &Arc<PackageArtifact> {
        &self.artifact
    }

    pub fn files(&self) -> &[Arc<FileIrUnit>] {
        &self.files
    }

    pub fn file(&self, identity: &str) -> Option<&Arc<FileIrUnit>> {
        self.file_slots
            .get(identity)
            .and_then(|slot| self.files.get(*slot))
    }

    pub fn resources(&self) -> &[HydratedStaticResource] {
        &self.resources
    }

    pub fn resource(&self, logical_path: &str) -> Option<&HydratedStaticResource> {
        self.resource_slots
            .get(logical_path)
            .and_then(|slot| self.resources.get(*slot))
    }

    pub fn schema_records(&self) -> &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>> {
        &self.schema_records
    }

    pub fn schema_index(&self) -> &Arc<PackageSchemaIndex> {
        &self.schema_index
    }
}

/// Fully validated immutable input handed to the runtime linker.
///
/// Activation templates remain inside the canonical assembly. The loader does
/// not turn them into activation-owned mutable state.
#[derive(Debug)]
pub struct HydratedRuntimeAssembly {
    assembly: Arc<RuntimeAssembly>,
    deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    contracts: Arc<ServiceContractStore>,
    code_slots: Vec<HydratedPackageCodeSlot>,
    code_slots_by_build: BTreeMap<PackageBuildId, usize>,
    gateway_entries: BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), Arc<HydratedGatewayEntry>>,
    gateway_ingress: BTreeMap<IngressSelector, Arc<HydratedGatewayEntry>>,
}

impl HydratedRuntimeAssembly {
    pub fn assembly(&self) -> &Arc<RuntimeAssembly> {
        &self.assembly
    }

    pub fn deployment(&self, reference: &ServiceDeploymentRef) -> Option<&Arc<ServiceDeployment>> {
        self.deployments.get(reference)
    }

    pub fn deployments(
        &self,
    ) -> impl ExactSizeIterator<Item = (&ServiceDeploymentRef, &Arc<ServiceDeployment>)> {
        self.deployments.iter()
    }

    /// Immutable canonical store handoff for boundary linking and admission.
    pub fn contract_store(&self) -> &Arc<ServiceContractStore> {
        &self.contracts
    }

    pub fn code_slots(&self) -> &[HydratedPackageCodeSlot] {
        &self.code_slots
    }

    pub fn code_slot(&self, slot: usize) -> Option<&HydratedPackageCodeSlot> {
        self.code_slots.get(slot)
    }

    pub fn code_slot_index(&self, build_id: &PackageBuildId) -> Option<usize> {
        self.code_slots_by_build.get(build_id).copied()
    }

    pub fn package(&self, build_id: &PackageBuildId) -> Option<&HydratedPackageCodeSlot> {
        self.code_slots_by_build
            .get(build_id)
            .and_then(|slot| self.code_slots.get(*slot))
    }

    pub fn gateway_entries(
        &self,
    ) -> impl ExactSizeIterator<
        Item = (
            &(ServiceDeploymentRef, GatewayEntryKey),
            &Arc<HydratedGatewayEntry>,
        ),
    > {
        self.gateway_entries.iter()
    }

    pub fn gateway_entry(
        &self,
        owner: &ServiceDeploymentRef,
        key: &GatewayEntryKey,
    ) -> Option<&Arc<HydratedGatewayEntry>> {
        self.gateway_entries.get(&(owner.clone(), key.clone()))
    }

    pub fn gateway_ingress(
        &self,
    ) -> impl ExactSizeIterator<Item = (&IngressSelector, &Arc<HydratedGatewayEntry>)> {
        self.gateway_ingress.iter()
    }
}

pub struct RuntimeAssemblyLoader<'a, R: ?Sized> {
    resolver: &'a R,
}

impl<'a, R> RuntimeAssemblyLoader<'a, R>
where
    R: RuntimeAssemblyContentResolver + ?Sized,
{
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    /// Resolve and hydrate one exact immutable assembly reference.
    pub fn load_ref(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<HydratedRuntimeAssembly>
    where
        R: RuntimeAssemblyRecordResolver,
    {
        let assembly = self
            .resolver
            .resolve_runtime_assembly(reference)
            .with_context(|| format!("failed to resolve runtime assembly {reference:?}"))?;
        if &skiff_artifact_identity::runtime_assembly_ref(&assembly)? != reference {
            anyhow::bail!("runtime assembly content mismatches exact ref {reference:?}");
        }
        self.load(assembly)
    }

    /// Hydrate an already typed assembly atomically. No partially hydrated
    /// value is observable when any reference or content check fails.
    pub fn load(
        &self,
        assembly: impl Into<Arc<RuntimeAssembly>>,
    ) -> anyhow::Result<HydratedRuntimeAssembly> {
        let assembly = assembly.into();
        validate_assembly(&assembly, "before hydration")?;

        let mut shared_schema_records = BTreeMap::new();
        let (code_slots, code_slots_by_build) =
            self.load_packages(&assembly, &mut shared_schema_records)?;
        let contracts = self.load_contracts(&assembly, &shared_schema_records)?;
        let deployments = self.load_deployments(&assembly)?;

        validate_hydrated_graph(&assembly, &deployments, &contracts, &code_slots)?;
        let gateway =
            hydrate_gateway_ingress(&assembly.gateway_ingress, &deployments, &code_slots)?;
        validate_assembly(&assembly, "after hydration")?;

        Ok(HydratedRuntimeAssembly {
            assembly,
            deployments,
            contracts: Arc::new(contracts),
            code_slots,
            code_slots_by_build,
            gateway_entries: gateway.entries,
            gateway_ingress: gateway.selectors,
        })
    }

    fn load_contracts(
        &self,
        assembly: &RuntimeAssembly,
        package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ) -> anyhow::Result<ServiceContractStore> {
        let mut contracts = BTreeMap::new();
        let mut schemas = BTreeMap::new();
        let mut shared_schema_records = BTreeMap::new();
        for reference in &assembly.resolved_contracts {
            let contract = self
                .resolver
                .resolve_contract(reference)
                .with_context(|| format!("failed to resolve contract {reference:?}"))?;
            validate_contract_ref(reference, &contract)?;
            let schema = self.load_contract_schema(
                reference,
                &contract,
                package_schema_records,
                &mut shared_schema_records,
            )?;
            if contracts.insert(reference.clone(), contract).is_some() {
                anyhow::bail!("duplicate resolved contract {reference:?}");
            }
            schemas.insert(reference.clone(), Arc::new(schema));
        }
        Ok(ServiceContractStore {
            contracts,
            schemas,
            shared_schema_records,
        })
    }

    fn load_contract_schema(
        &self,
        reference: &ServiceContractRef,
        contract: &ServiceContract,
        package_schema_records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
        shared_records: &mut BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ) -> anyhow::Result<ResolvedServiceSchema> {
        let mut records = BTreeMap::<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>::new();
        for requirement in &contract.package_type_requirements {
            for type_id in &requirement.required_type_ids {
                let record = if let Some(record) = shared_records.get(type_id) {
                    Arc::clone(record)
                } else if let Some(record) = package_schema_records.get(type_id) {
                    shared_records.insert(type_id.clone(), Arc::clone(record));
                    Arc::clone(record)
                } else {
                    let record_ref = PackageSchemaTypeRecordRef {
                        package_id: requirement.package_id.clone(),
                        package_schema_type_id: type_id.clone(),
                    };
                    let record = self
                        .resolver
                        .resolve_package_schema_type(&record_ref)
                        .with_context(|| {
                            format!(
                                "failed to resolve package schema type {} for contract {reference:?}",
                                type_id
                            )
                        })?;
                    shared_records.insert(type_id.clone(), Arc::clone(&record));
                    record
                };
                if record.package_id != requirement.package_id
                    || record.package_schema_type_id != *type_id
                {
                    anyhow::bail!(
                        "package schema type {type_id} does not match required owner {} and identity",
                        requirement.package_id
                    );
                }
                if records.insert(type_id.clone(), record).is_some() {
                    anyhow::bail!(
                        "package schema type {type_id} is required more than once by contract {reference:?}"
                    );
                }
            }
        }
        validate_resolved_service_schema(contract, &records)?;
        Ok(ResolvedServiceSchema {
            contract: reference.clone(),
            records,
        })
    }

    fn load_deployments(
        &self,
        assembly: &RuntimeAssembly,
    ) -> anyhow::Result<BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>> {
        let mut deployments = BTreeMap::new();
        for reference in &assembly.resolved_deployments {
            let deployment = self
                .resolver
                .resolve_deployment(reference)
                .with_context(|| format!("failed to resolve deployment {reference:?}"))?;
            skiff_artifact_identity::validate_service_deployment_ref(reference, &deployment)
                .map_err(anyhow::Error::from)
                .with_context(|| format!("deployment content mismatches ref {reference:?}"))?;
            if deployments.insert(reference.clone(), deployment).is_some() {
                anyhow::bail!("duplicate resolved deployment {reference:?}");
            }
        }
        Ok(deployments)
    }

    fn load_packages(
        &self,
        assembly: &RuntimeAssembly,
        shared_schema_records: &mut BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ) -> anyhow::Result<(
        Vec<HydratedPackageCodeSlot>,
        BTreeMap<PackageBuildId, usize>,
    )> {
        let mut references = assembly
            .package_link_plan
            .code_slots
            .iter()
            .map(|slot| slot.package.clone())
            .collect::<Vec<_>>();
        references.sort_by(|left, right| left.package_build_id.cmp(&right.package_build_id));

        let mut shared_files = BTreeMap::<String, Arc<FileIrUnit>>::new();
        let mut code_slots = Vec::with_capacity(references.len());
        let mut code_slots_by_build = BTreeMap::new();
        for reference in references {
            if code_slots_by_build.contains_key(&reference.package_build_id) {
                anyhow::bail!(
                    "package build {} appears in more than one code slot",
                    reference.package_build_id
                );
            }
            let artifact = self
                .resolver
                .resolve_package(&reference)
                .with_context(|| format!("failed to resolve package {reference:?}"))?;
            validate_package_ref(&reference, &artifact)?;
            let schema_index = self.load_package_schema_index(&artifact)?;
            let (files, file_slots) = self.load_files(&reference, &artifact, &mut shared_files)?;
            validate_package_file_targets(&reference, &artifact, &files, &file_slots)?;
            let (resources, resource_slots) = self.load_resources(&reference, &artifact)?;
            let schema_records =
                self.load_package_schema_closure(&artifact, &schema_index, shared_schema_records)?;
            let resolved_schema_records = schema_records
                .iter()
                .map(|(id, record)| (id.clone(), record.as_ref().clone()))
                .collect::<BTreeMap<_, _>>();
            skiff_artifact_identity::validate_package_schema_records(&resolved_schema_records)
                .context("invalid resolved Package schema closure")?;

            let slot = code_slots.len();
            code_slots_by_build.insert(reference.package_build_id.clone(), slot);
            code_slots.push(HydratedPackageCodeSlot {
                reference,
                artifact,
                files,
                file_slots,
                resources,
                resource_slots,
                schema_index,
                schema_records,
            });
        }
        Ok((code_slots, code_slots_by_build))
    }

    fn load_package_schema_index(
        &self,
        artifact: &PackageArtifact,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        let reference = &artifact.package_schema_index;
        if reference.package_id != artifact.package_id {
            anyhow::bail!(
                "package {} schema index ref has mismatched owner {}",
                artifact.package_id,
                reference.package_id
            );
        }
        let index = self
            .resolver
            .resolve_package_schema_index(reference)
            .with_context(|| {
                format!(
                    "failed to resolve exact Package schema index for package {}",
                    artifact.package_id
                )
            })?;
        if index.package_id != reference.package_id
            || index.package_schema_index_identity != reference.package_schema_index_identity
        {
            anyhow::bail!(
                "Package schema index content does not match exact ref for package {}",
                artifact.package_id
            );
        }
        skiff_artifact_identity::validate_package_schema_index(&index)
            .map_err(anyhow::Error::from)
            .with_context(|| {
                format!(
                    "invalid Package schema index for package {}",
                    artifact.package_id
                )
            })?;

        let mut type_ids = BTreeSet::new();
        let mut public_paths = BTreeSet::new();
        for (stable_key, entry) in &index.types {
            if !type_ids.insert(entry.package_schema_type_id.clone()) {
                anyhow::bail!(
                    "package {} schema index assigns type {} to more than one stable key",
                    artifact.package_id,
                    entry.package_schema_type_id
                );
            }
            let public_path = entry.public_path.as_deref().with_context(|| {
                format!(
                    "package {} schema index entry {stable_key} has no public path",
                    artifact.package_id
                )
            })?;
            if !public_paths.insert(public_path) {
                anyhow::bail!(
                    "package {} schema index repeats public path {public_path}",
                    artifact.package_id
                );
            }
            let record_ref = artifact
                .package_schema_type_records
                .get(&entry.package_schema_type_id)
                .with_context(|| {
                    format!(
                        "package {} schema index entry {stable_key} has no exact record ref",
                        artifact.package_id
                    )
                })?;
            if record_ref.package_id != artifact.package_id
                || record_ref.package_schema_type_id != entry.package_schema_type_id
            {
                anyhow::bail!(
                    "package {} schema index entry {stable_key} has mismatched record owner or identity",
                    artifact.package_id
                );
            }
        }
        if artifact.package_schema_type_records.len() != index.types.len()
            || artifact
                .package_schema_type_records
                .keys()
                .any(|type_id| !type_ids.contains(type_id))
        {
            anyhow::bail!(
                "package {} schema record refs do not exactly match its Package schema index",
                artifact.package_id
            );
        }
        Ok(index)
    }

    fn load_package_schema_closure(
        &self,
        artifact: &PackageArtifact,
        index: &PackageSchemaIndex,
        shared_schema_records: &mut BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    ) -> anyhow::Result<BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>> {
        let mut pending = artifact
            .package_schema_type_records
            .iter()
            .map(|(type_id, reference)| {
                let stable_key = index
                    .types
                    .iter()
                    .find_map(|(stable_key, entry)| {
                        (&entry.package_schema_type_id == type_id).then(|| stable_key.clone())
                    })
                    .expect("record refs and validated Package schema index are exact");
                (reference.clone(), Some(stable_key))
            })
            .collect::<Vec<(PackageSchemaTypeRecordRef, Option<String>)>>();
        let mut records = BTreeMap::<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>::new();

        while let Some((record_ref, expected_stable_key)) = pending.pop() {
            let type_id = &record_ref.package_schema_type_id;
            if let Some(record) = records.get(type_id) {
                validate_package_schema_record_ref(
                    &record_ref,
                    expected_stable_key.as_deref(),
                    record,
                )?;
                continue;
            }

            let resolved = self
                .resolver
                .resolve_package_schema_type(&record_ref)
                .with_context(|| {
                    format!(
                        "failed to resolve package schema type {type_id} for package {} closure",
                        artifact.package_id
                    )
                })?;
            let record = if let Some(existing) = shared_schema_records.get(type_id) {
                if existing.as_ref() != resolved.as_ref() {
                    anyhow::bail!(
                        "conflicting Package schema record content for exact type identity {type_id}"
                    );
                }
                Arc::clone(existing)
            } else {
                resolved
            };
            validate_package_schema_record_ref(
                &record_ref,
                expected_stable_key.as_deref(),
                &record,
            )?;

            for child in package_schema_descriptor_refs(&record.canonical_descriptor.descriptor) {
                pending.push((
                    PackageSchemaTypeRecordRef {
                        package_id: child.package_id,
                        package_schema_type_id: child.package_schema_type_id,
                    },
                    Some(child.stable_schema_key),
                ));
            }
            shared_schema_records
                .entry(type_id.clone())
                .or_insert_with(|| Arc::clone(&record));
            records.insert(type_id.clone(), record);
        }

        Ok(records)
    }

    fn load_files(
        &self,
        package_ref: &PackageArtifactRef,
        package: &PackageArtifact,
        shared_files: &mut BTreeMap<String, Arc<FileIrUnit>>,
    ) -> anyhow::Result<(Vec<Arc<FileIrUnit>>, BTreeMap<String, usize>)> {
        let mut references = package.files.clone();
        references.sort_by(|left, right| {
            (&left.file_ir_identity, &left.module_path)
                .cmp(&(&right.file_ir_identity, &right.module_path))
        });
        let mut module_paths = BTreeSet::new();
        let mut files = Vec::with_capacity(references.len());
        let mut file_slots = BTreeMap::new();
        for reference in references {
            validate_file_ref_path(package_ref, &reference)?;
            if !module_paths.insert(reference.module_path.clone()) {
                anyhow::bail!(
                    "package {} repeats File IR module path {}",
                    package_ref.package_build_id,
                    reference.module_path
                );
            }
            if file_slots.contains_key(&reference.file_ir_identity) {
                anyhow::bail!(
                    "package {} repeats File IR identity {}",
                    package_ref.package_build_id,
                    reference.file_ir_identity
                );
            }

            let file = if let Some(file) = shared_files.get(&reference.file_ir_identity) {
                validate_file_ref(package_ref, &reference, file)?;
                Arc::clone(file)
            } else {
                let file = self
                    .resolver
                    .resolve_file_ir(package_ref, &reference)
                    .with_context(|| {
                        format!(
                            "failed to resolve File IR {} for package {}",
                            reference.file_ir_identity, package_ref.package_build_id
                        )
                    })?;
                validate_file_ref(package_ref, &reference, &file)?;
                shared_files.insert(reference.file_ir_identity.clone(), Arc::clone(&file));
                file
            };
            file_slots.insert(reference.file_ir_identity, files.len());
            files.push(file);
        }
        Ok((files, file_slots))
    }

    fn load_resources(
        &self,
        package_ref: &PackageArtifactRef,
        package: &PackageArtifact,
    ) -> anyhow::Result<(Vec<HydratedStaticResource>, BTreeMap<String, usize>)> {
        let mut references = package.static_resources.clone();
        references.sort_by(|left, right| left.path.cmp(&right.path));
        let mut resources = Vec::with_capacity(references.len());
        let mut resource_slots = BTreeMap::new();
        for reference in references {
            validate_resource_ref_path(package_ref, &reference)?;
            if resource_slots.contains_key(&reference.path) {
                anyhow::bail!(
                    "package {} repeats static resource path {}",
                    package_ref.package_build_id,
                    reference.path
                );
            }
            let bytes = self
                .resolver
                .resolve_static_resource(package_ref, &reference)
                .with_context(|| {
                    format!(
                        "failed to resolve static resource {} for package {}",
                        reference.path, package_ref.package_build_id
                    )
                })?;
            validate_resource_content(package_ref, &reference, &bytes)?;
            resource_slots.insert(reference.path.clone(), resources.len());
            resources.push(HydratedStaticResource { reference, bytes });
        }
        Ok((resources, resource_slots))
    }
}

fn validate_package_schema_record_ref(
    reference: &PackageSchemaTypeRecordRef,
    expected_stable_key: Option<&str>,
    record: &PackageSchemaTypeRecord,
) -> anyhow::Result<()> {
    if record.package_id != reference.package_id
        || record.package_schema_type_id != reference.package_schema_type_id
    {
        anyhow::bail!(
            "package schema type {} does not match exact owner {} and identity",
            reference.package_schema_type_id,
            reference.package_id
        );
    }
    if expected_stable_key.is_some_and(|expected| record.stable_schema_key != expected) {
        anyhow::bail!(
            "package schema type {} does not match exact stable key {}",
            reference.package_schema_type_id,
            expected_stable_key.expect("checked as present")
        );
    }
    Ok(())
}

fn validate_resolved_service_schema(
    contract: &ServiceContract,
    records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
) -> anyhow::Result<()> {
    let owned_records = records
        .iter()
        .map(|(type_id, record)| (type_id.clone(), record.as_ref().clone()))
        .collect::<BTreeMap<_, _>>();
    skiff_artifact_identity::validate_package_schema_records(&owned_records)
        .map_err(anyhow::Error::from)
        .context("invalid resolved package schema closure")?;

    let required = contract
        .package_type_requirements
        .iter()
        .flat_map(|requirement| {
            requirement
                .required_type_ids
                .iter()
                .map(move |type_id| (type_id.clone(), requirement.package_id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    if required.len() != records.len()
        || required.keys().ne(records.keys())
        || records.iter().any(|(type_id, record)| {
            required
                .get(type_id)
                .is_none_or(|package_id| *package_id != record.package_id)
        })
    {
        anyhow::bail!("resolved package schema records do not exactly match contract requirements");
    }

    let mut reachable = BTreeSet::new();
    for operation in contract.operations.values() {
        for parameter in &operation.contract.parameters {
            collect_reachable_type_refs(&parameter.ty, records, &mut reachable)?;
        }
        collect_reachable_type_refs(&operation.contract.return_value.ty, records, &mut reachable)?;
        if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.contract.stream {
            collect_reachable_type_refs(item_type, records, &mut reachable)?;
        }
        if let BoundaryCallbackContract::RequestScoped {
            interface_types, ..
        } = &operation.contract.callbacks
        {
            for reference in interface_types {
                collect_reachable_package_type(
                    &reference.package_id,
                    &reference.stable_schema_key,
                    &reference.package_schema_type_id,
                    records,
                    &mut reachable,
                )?;
            }
        }
    }
    if reachable != records.keys().cloned().collect() {
        anyhow::bail!(
            "contract package type requirements do not exactly match operation descriptor closure"
        );
    }
    Ok(())
}

fn collect_reachable_type_refs(
    ty: &ContractTypeRef,
    records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    reachable: &mut BTreeSet<PackageSchemaTypeId>,
) -> anyhow::Result<()> {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => collect_reachable_package_type(
            package_id,
            stable_schema_key,
            package_schema_type_id,
            records,
            reachable,
        ),
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => {
            for argument in arguments {
                collect_reachable_type_refs(argument, records, reachable)?;
            }
            Ok(())
        }
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                collect_reachable_type_refs(field, records, reachable)?;
            }
            Ok(())
        }
        ContractTypeRef::Nullable { inner } => {
            collect_reachable_type_refs(inner, records, reachable)
        }
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_reachable_type_refs(interface, records, reachable)?;
            for argument in arguments {
                collect_reachable_type_refs(argument, records, reachable)?;
            }
            Ok(())
        }
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => Ok(()),
    }
}

fn collect_reachable_package_type(
    package_id: &str,
    stable_schema_key: &str,
    type_id: &PackageSchemaTypeId,
    records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    reachable: &mut BTreeSet<PackageSchemaTypeId>,
) -> anyhow::Result<()> {
    let record = records
        .get(type_id)
        .with_context(|| format!("package schema closure is missing required type {type_id}"))?;
    if record.package_id != package_id || record.stable_schema_key != stable_schema_key {
        anyhow::bail!(
            "package schema reference {type_id} owner or stable key does not match resolved record"
        );
    }
    if !reachable.insert(type_id.clone()) {
        return Ok(());
    }
    collect_descriptor_type_refs(&record.canonical_descriptor.descriptor, records, reachable)
}

fn collect_descriptor_type_refs(
    descriptor: &ContractTypeDescriptor,
    records: &BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    reachable: &mut BTreeSet<PackageSchemaTypeId>,
) -> anyhow::Result<()> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for field in fields.values() {
                collect_reachable_type_refs(field, records, reachable)?;
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for variant in variants {
                collect_reachable_type_refs(variant, records, reachable)?;
            }
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            for branch in branches {
                collect_reachable_type_refs(&branch.branch_type, records, reachable)?;
            }
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            collect_reachable_type_refs(target, records, reachable)?;
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                for parameter in &operation.parameters {
                    collect_reachable_type_refs(parameter, records, reachable)?;
                }
                collect_reachable_type_refs(&operation.return_type, records, reachable)?;
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
