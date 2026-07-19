use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, FileIrRef, FileIrUnit, PackageArtifact,
    PackageArtifactRef, PackageBuildId, PublicationResourceRef, RuntimeAssembly, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

mod content_validation;
mod graph_validation;

use content_validation::{
    validate_assembly, validate_contract_ref, validate_file_ref, validate_file_ref_path,
    validate_package_file_targets, validate_package_ref, validate_resource_content,
    validate_resource_ref_path,
};
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

/// Immutable canonical contract store retained after assembly hydration.
#[derive(Debug, Default)]
pub struct ServiceContractStore {
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
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

    /// Hydrate an already typed assembly atomically. No partially hydrated
    /// value is observable when any reference or content check fails.
    pub fn load(
        &self,
        assembly: impl Into<Arc<RuntimeAssembly>>,
    ) -> anyhow::Result<HydratedRuntimeAssembly> {
        let assembly = assembly.into();
        validate_assembly(&assembly, "before hydration")?;

        let contracts = self.load_contracts(&assembly)?;
        let (code_slots, code_slots_by_build) = self.load_packages(&assembly)?;
        let deployments = self.load_deployments(&assembly)?;

        validate_hydrated_graph(&assembly, &deployments, &contracts, &code_slots)?;
        validate_assembly(&assembly, "after hydration")?;

        Ok(HydratedRuntimeAssembly {
            assembly,
            deployments,
            contracts: Arc::new(contracts),
            code_slots,
            code_slots_by_build,
        })
    }

    fn load_contracts(&self, assembly: &RuntimeAssembly) -> anyhow::Result<ServiceContractStore> {
        let mut contracts = BTreeMap::new();
        for reference in &assembly.resolved_contracts {
            let contract = self
                .resolver
                .resolve_contract(reference)
                .with_context(|| format!("failed to resolve contract {reference:?}"))?;
            validate_contract_ref(reference, &contract)?;
            if contracts.insert(reference.clone(), contract).is_some() {
                anyhow::bail!("duplicate resolved contract {reference:?}");
            }
        }
        Ok(ServiceContractStore { contracts })
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
            let (files, file_slots) = self.load_files(&reference, &artifact, &mut shared_files)?;
            validate_package_file_targets(&reference, &artifact, &files, &file_slots)?;
            let (resources, resource_slots) = self.load_resources(&reference, &artifact)?;

            let slot = code_slots.len();
            code_slots_by_build.insert(reference.package_build_id.clone(), slot);
            code_slots.push(HydratedPackageCodeSlot {
                reference,
                artifact,
                files,
                file_slots,
                resources,
                resource_slots,
            });
        }
        Ok((code_slots, code_slots_by_build))
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

#[cfg(test)]
mod tests;
