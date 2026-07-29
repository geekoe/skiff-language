use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_identity::{service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    AssemblyIdentity, PackageArtifact, PackageArtifactRef, RuntimeAssembly, RuntimeAssemblyRef,
    ServiceContract, ServiceDeployment,
};
use skiff_compiler::{authoring::publish_package_artifact_records, PublishedPackageArtifact};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_deployment::storage::PackageArtifactAdmissionCache;

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
        self.publish_with_session(
            source_artifact_root,
            runtime_artifact_root,
            &mut CanonicalPublishSession::default(),
        )
    }

    pub(crate) fn publish_with_session(
        &self,
        source_artifact_root: &Path,
        runtime_artifact_root: &Path,
        session: &mut CanonicalPublishSession,
    ) -> Result<Vec<PathBuf>, CanonicalFixtureError> {
        let source = CanonicalArtifactStore::open(source_artifact_root)?;
        let target = CanonicalArtifactStore::create(runtime_artifact_root)?;
        let owned_packages = self
            .packages
            .iter()
            .map(|package| declared_package_ref(&package.artifact))
            .collect::<BTreeSet<_>>();
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
                copy_package(&source, &target, package, session, &mut written)?;
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
            publish_package(&target, package, session, &mut written)?;
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
}

#[derive(Debug, Default)]
pub(crate) struct CanonicalPublishSession {
    package_admissions: PackageArtifactAdmissionCache,
    owned_package_publications: BTreeMap<(PathBuf, PackageArtifactRef), PublishedPackageArtifact>,
}

fn copy_package(
    source: &CanonicalArtifactStore,
    target: &CanonicalArtifactStore,
    reference: &PackageArtifactRef,
    session: &mut CanonicalPublishSession,
    written: &mut Vec<PathBuf>,
) -> Result<(), CanonicalFixtureError> {
    let admitted = session.package_admissions.admit(source, reference)?;
    written.extend(target.write_validated_package_copy_records(admitted)?);
    Ok(())
}

fn publish_package(
    store: &CanonicalArtifactStore,
    package: &PublishedPackageArtifact,
    session: &mut CanonicalPublishSession,
    written: &mut Vec<PathBuf>,
) -> Result<(), CanonicalFixtureError> {
    let reference = declared_package_ref(&package.artifact);
    let key = (store.root().to_path_buf(), reference.clone());
    if let Some(previous) = session.owned_package_publications.get(&key) {
        if previous != package {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "test-owned package {} reused one exact reference with different emitted content",
                reference.package_build_id
            )));
        }
        session.package_admissions.admit(store, &reference)?;
        return Ok(());
    }
    let receipt = publish_package_artifact_records(store.root(), package)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    if receipt.artifact != reference {
        return Err(CanonicalFixtureError::InvalidInput(
            "test-owned package publication receipt changed its declared exact reference"
                .to_string(),
        ));
    }
    written.extend(
        receipt
            .file_ir_record_paths
            .iter()
            .chain(&receipt.resource_record_paths)
            .chain(std::iter::once(&receipt.record_path))
            .map(|path| store.root().join(path)),
    );
    session
        .owned_package_publications
        .insert(key, package.clone());
    Ok(())
}

fn declared_package_ref(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}
