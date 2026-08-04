use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_identity::{service_contract_ref, service_deployment_ref};
use skiff_artifact_model::{
    AssemblyIdentity, PackageArtifact, PackageArtifactRef, RuntimeAssembly, RuntimeAssemblyRef,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceContract, ServiceDeployment,
};
use skiff_compiler::{authoring::publish_package_artifact_records, PublishedPackageArtifact};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_deployment::storage::PackageArtifactAdmissionCache;
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};

use crate::canonical_fixture::CanonicalFixtureError;

/// Exact, hydrated closure selected by `--base-assembly`.
#[derive(Debug, Clone, Default)]
pub struct CanonicalBaseAssembly {
    pub assembly: Option<RuntimeAssembly>,
    pub config_snapshot: Option<RuntimeConfigSnapshot>,
    pub packages: Vec<PackageArtifact>,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
}

impl CanonicalBaseAssembly {
    pub fn load(
        artifact_root: &Path,
        identity: Option<&str>,
        config_snapshot_id: Option<&str>,
        target_profile: &str,
    ) -> Result<Self, CanonicalFixtureError> {
        let (Some(identity), Some(config_snapshot_id)) = (identity, config_snapshot_id) else {
            if identity.is_none() && config_snapshot_id.is_none() {
                return Ok(Self::default());
            }
            return Err(CanonicalFixtureError::InvalidInput(
                "base assembly and base config snapshot must be provided together".to_string(),
            ));
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
        let config_snapshot_ref = RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(config_snapshot_id).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "base config snapshot identity is invalid: {error}"
                ))
            })?,
        };
        let config_snapshot =
            RuntimeConfigSnapshotStore::open(artifact_root.join("runtime-config"))
                .and_then(|store| store.read(&config_snapshot_ref))
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        if config_snapshot.profile() != target_profile {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "base config snapshot profile {:?} does not match target profile {:?}",
                config_snapshot.profile(),
                target_profile
            )));
        }
        let expected = assembly
            .resolved_deployments
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual = config_snapshot
            .deployments()
            .iter()
            .map(|deployment| deployment.deployment().clone())
            .collect::<BTreeSet<_>>();
        if expected != actual || actual.len() != config_snapshot.deployments().len() {
            return Err(CanonicalFixtureError::InvalidInput(
                "base config snapshot deployments do not exactly match the base assembly"
                    .to_string(),
            ));
        }
        Ok(Self {
            assembly: Some(assembly),
            config_snapshot: Some(config_snapshot),
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
    pub config_snapshot: RuntimeConfigSnapshot,
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
        written.push(
            RuntimeConfigSnapshotStore::create(runtime_artifact_root.join("runtime-config"))
                .and_then(|store| store.publish(&self.config_snapshot))
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?,
        );
        Ok(written)
    }
}

#[derive(Debug, Default)]
pub(crate) struct CanonicalPublishSession {
    package_admissions: PackageArtifactAdmissionCache,
    owned_package_publications: BTreeMap<(PathBuf, PackageArtifactRef), PublishedPackageArtifact>,
    trusted_source: bool,
}

impl CanonicalPublishSession {
    pub(crate) fn with_trusted_source(mut self, trusted: bool) -> Self {
        self.trusted_source = trusted;
        self
    }

    pub(crate) fn owned_package_publication_count(&self) -> usize {
        self.owned_package_publications.len()
    }
}

fn copy_package(
    source: &CanonicalArtifactStore,
    target: &CanonicalArtifactStore,
    reference: &PackageArtifactRef,
    session: &mut CanonicalPublishSession,
    written: &mut Vec<PathBuf>,
) -> Result<(), CanonicalFixtureError> {
    if session.trusted_source {
        let records = source.read_package_copy_records_raw(reference)?;
        written.extend(target.write_package_copy_records_raw(&records)?);
    } else {
        let admitted = session.package_admissions.admit(source, reference)?;
        written.extend(target.write_validated_package_copy_records(admitted)?);
    }
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
