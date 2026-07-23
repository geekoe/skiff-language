use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use skiff_artifact_identity::{package_artifact_ref, service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    AssemblyIdentity, PackageArtifact, PackageArtifactRef, RuntimeAssembly, RuntimeAssemblyRef,
    ServiceContract, ServiceDeployment,
};
use skiff_compiler::{authoring::publish_package_artifact_records, PublishedPackageArtifact};
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
    let receipt = publish_package_artifact_records(store, package)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    written.extend(
        receipt
            .file_ir_record_paths
            .iter()
            .chain(&receipt.resource_record_paths)
            .chain(std::iter::once(&receipt.record_path))
            .map(|path| store.root().join(path)),
    );
    Ok(())
}
