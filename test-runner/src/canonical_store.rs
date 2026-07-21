use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    AssemblyIdentity, PackageArtifact, PackageArtifactRef, PackageOperationTarget, RuntimeAssembly,
    RuntimeAssemblyRef, ServiceContract, ServiceDeployment,
};
use skiff_compiler::PublishedPackageArtifact;
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::canonical_fixture::CanonicalFixtureError;

/// Exact, hydrated closure selected by `--base-assembly`.
#[derive(Debug, Clone, Default)]
pub struct CanonicalBaseAssembly {
    pub assembly: Option<RuntimeAssembly>,
    pub packages: Vec<PackageArtifact>,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
}

impl CanonicalBaseAssembly {
    pub fn load(
        artifact_root: &Path,
        identity: Option<&str>,
    ) -> Result<Self, CanonicalFixtureError> {
        let Some(identity) = identity else {
            return Ok(Self::default());
        };
        let store = CanonicalArtifactStore::open(artifact_root)?;
        let reference = RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(identity),
        };
        let assembly = store.read_runtime_assembly(&reference)?.as_ref().clone();
        let deployments = assembly
            .resolved_deployments
            .iter()
            .map(|reference| Ok(store.read_service_deployment(reference)?.as_ref().clone()))
            .collect::<Result<Vec<_>, skiff_deployment::storage::EcosystemStorageError>>()?;
        let contracts = assembly
            .resolved_contracts
            .iter()
            .map(|reference| Ok(store.read_service_contract(reference)?.as_ref().clone()))
            .collect::<Result<Vec<_>, skiff_deployment::storage::EcosystemStorageError>>()?;
        let packages = assembly
            .resolved_packages
            .iter()
            .map(|reference| Ok(store.read_package_artifact(reference)?.as_ref().clone()))
            .collect::<Result<Vec<_>, skiff_deployment::storage::EcosystemStorageError>>()?;
        Ok(Self {
            assembly: Some(assembly),
            packages,
            contracts,
            deployments,
        })
    }
}

/// Test-owned records plus the final immutable assembly.
#[derive(Debug, Clone)]
pub struct CanonicalTestRecords {
    pub packages: Vec<PublishedPackageArtifact>,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
    pub assembly: RuntimeAssembly,
    pub base_assembly: Option<RuntimeAssembly>,
}

impl CanonicalTestRecords {
    /// Copies the exact external closure into the writable runtime root, then
    /// publishes only test-owned records and the projected test assembly.
    pub fn publish(
        &self,
        source_artifact_root: &Path,
        runtime_artifact_root: &Path,
    ) -> Result<Vec<PathBuf>, CanonicalFixtureError> {
        let source = CanonicalArtifactStore::open(source_artifact_root)?;
        let target = CanonicalArtifactStore::create(runtime_artifact_root)?;
        let owned_packages = self
            .packages
            .iter()
            .map(|package| package_artifact_ref(&package.artifact))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let owned_contracts = self
            .contracts
            .iter()
            .map(service_contract_ref)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        let owned_deployments = self
            .deployments
            .iter()
            .map(service_deployment_ref)
            .collect::<BTreeSet<_>>();

        let mut written = Vec::new();
        for package in &self.assembly.resolved_packages {
            if !owned_packages.contains(package) {
                copy_package(&source, &target, package, &mut written)?;
            }
        }
        for contract in &self.assembly.resolved_contracts {
            if !owned_contracts.contains(contract) {
                let value = source.read_service_contract(contract)?;
                written.push(target.write_service_contract(&value)?);
            }
        }
        for deployment in &self.assembly.resolved_deployments {
            if !owned_deployments.contains(deployment) {
                let value = source.read_service_deployment(deployment)?;
                written.push(target.write_service_deployment(&value)?);
            }
        }
        if let Some(base) = &self.base_assembly {
            written.push(target.write_runtime_assembly(base)?);
        }

        for package in &self.packages {
            publish_package(&target, package, &mut written)?;
        }
        for contract in &self.contracts {
            written.push(target.write_service_contract(contract)?);
        }
        for deployment in &self.deployments {
            written.push(target.write_service_deployment(deployment)?);
        }
        written.push(target.write_runtime_assembly(&self.assembly)?);
        Ok(written)
    }

    pub fn assert_production_package_unchanged(
        before: &PackageArtifactRef,
        after: &PublishedPackageArtifact,
    ) -> Result<(), CanonicalFixtureError> {
        let after = package_artifact_ref(&after.artifact)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        if before != &after {
            return Err(CanonicalFixtureError::InvalidInput(
                "test overlay rewrote production PackageArtifact identity".to_string(),
            ));
        }
        Ok(())
    }
}

fn copy_package(
    source: &CanonicalArtifactStore,
    target: &CanonicalArtifactStore,
    reference: &PackageArtifactRef,
    written: &mut Vec<PathBuf>,
) -> Result<(), CanonicalFixtureError> {
    let artifact = source.read_package_artifact(reference)?;
    for file in &artifact.files {
        let unit = source.read_file_ir(reference, file)?;
        written.push(target.write_file_ir(reference, file, &unit)?);
    }
    for resource in &artifact.static_resources {
        let bytes = source.read_static_resource(reference, resource)?;
        written.push(target.write_static_resource(reference, resource, &bytes)?);
    }
    written.push(target.write_package_artifact(&artifact)?);
    Ok(())
}

fn publish_package(
    store: &CanonicalArtifactStore,
    package: &PublishedPackageArtifact,
    written: &mut Vec<PathBuf>,
) -> Result<(), CanonicalFixtureError> {
    let package = storage_canonical_package(package);
    let reference = package_artifact_ref(&package.artifact)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    for file in &package.file_ir_units {
        let file_ref = package
            .artifact
            .files
            .iter()
            .find(|candidate| candidate.file_ir_identity == file.unit.file_ir_identity)
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "package {} emitted File IR {} outside its canonical refs",
                    package.artifact.package_build_id, file.unit.file_ir_identity
                ))
            })?;
        written.push(store.write_file_ir(&reference, file_ref, &file.unit)?);
    }
    for resource_ref in &package.artifact.static_resources {
        let blob = package
            .resource_blobs
            .iter()
            .find(|candidate| {
                candidate.logical_path == resource_ref.path
                    && candidate.sha256 == resource_ref.sha256
                    && candidate.byte_len == resource_ref.byte_len
            })
            .ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(format!(
                    "package {} resource {} has no exact emitted blob",
                    package.artifact.package_build_id, resource_ref.path
                ))
            })?;
        written.push(store.write_static_resource(&reference, resource_ref, &blob.bytes)?);
    }
    written.push(store.write_package_artifact(&package.artifact)?);
    Ok(())
}

fn storage_canonical_package(package: &PublishedPackageArtifact) -> PublishedPackageArtifact {
    let mut package = package.clone();
    for file in &mut package.artifact.files {
        file.artifact_path = None;
    }
    for resource in &mut package.artifact.static_resources {
        resource.artifact_path = None;
    }
    for link in package.artifact.callable_links.values_mut() {
        link.target.file_ref.artifact_path = None;
    }
    for export in package.artifact.implementation_links.types.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package.artifact.implementation_links.constants.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package.artifact.implementation_links.functions.values_mut() {
        export.file.artifact_path = None;
    }
    for export in package
        .artifact
        .implementation_links
        .impl_methods
        .values_mut()
    {
        export.file.artifact_path = None;
    }
    for target in package
        .artifact
        .implementation_links
        .operation_targets
        .values_mut()
    {
        match target {
            PackageOperationTarget::LocalExecutable { target, .. } => {
                target.file_ref.artifact_path = None;
            }
            PackageOperationTarget::LocalConstReceiverExecutable { target, .. } => {
                target.receiver.file_ref.artifact_path = None;
                target.executable_target.file_ref.artifact_path = None;
            }
        }
    }
    package
}
