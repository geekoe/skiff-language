use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use skiff_artifact_identity::service_contract_ref;
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    ServiceContract, ServiceDeployment,
};
use skiff_compiler::{
    authoring::publish_package_artifact_records_with_bytecode, PackageBytecodeLane,
    PublishedPackageArtifact,
};
use skiff_deployment::storage::{CanonicalArtifactStore, PackageArtifactAdmissionCache};
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};

use crate::canonical_fixture::CanonicalFixtureError;

/// Exact base closure selected by the base config snapshot.
///
/// The legacy `RuntimeAssembly` is gone. The base closure is hydrated from
/// deployment records and their exact package/contract records, so service
/// requirements and config inheritance work from the same immutable closure
/// the runtime would load.
#[derive(Debug, Clone, Default)]
pub struct CanonicalBaseClosure {
    pub config_snapshot: Option<RuntimeConfigSnapshot>,
    pub packages: Vec<PackageArtifact>,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
}

impl CanonicalBaseClosure {
    pub fn load(
        artifact_root: &Path,
        _base_assembly: Option<&str>,
        config_snapshot_id: Option<&str>,
        target_profile: &str,
    ) -> Result<Self, CanonicalFixtureError> {
        let Some(config_snapshot_id) = config_snapshot_id else {
            if _base_assembly.is_none() {
                return Ok(Self::default());
            }
            return Err(CanonicalFixtureError::InvalidInput(
                "base assembly is retired; supply only the base config snapshot".to_string(),
            ));
        };
        let store = CanonicalArtifactStore::open(artifact_root)?;
        let snapshot_ref = RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(config_snapshot_id).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "base config snapshot identity is invalid: {error}"
                ))
            })?,
        };
        let config_snapshot =
            RuntimeConfigSnapshotStore::open(artifact_root.join("runtime-config"))
                .and_then(|store| store.read(&snapshot_ref))
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        if config_snapshot.profile() != target_profile {
            return Err(CanonicalFixtureError::InvalidInput(format!(
                "base config snapshot profile {:?} does not match target profile {:?}",
                config_snapshot.profile(),
                target_profile
            )));
        }

        let deployment_refs = config_snapshot
            .deployments()
            .iter()
            .map(|deployment| deployment.deployment().clone())
            .collect::<Vec<_>>();
        let mut deployments = Vec::with_capacity(deployment_refs.len());
        let mut package_refs = BTreeSet::new();
        let mut contract_refs = BTreeSet::new();
        for reference in deployment_refs {
            let deployment = store
                .read_service_deployment(&reference)
                .map_err(|error| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "base deployment {}@{} is missing from the artifact store: {error}",
                        reference.service_id, reference.contract_version
                    ))
                })?
                .as_ref()
                .clone();
            package_refs.insert(deployment.implementation.clone());
            package_refs.extend(
                deployment
                    .package_bindings
                    .iter()
                    .map(|binding| binding.package.clone()),
            );
            contract_refs.insert(deployment.contract.clone());
            contract_refs.extend(
                deployment
                    .service_selectors
                    .iter()
                    .map(|selector| selector.contract.clone()),
            );
            deployments.push(deployment);
        }
        let actual_deployments = deployments
            .iter()
            .map(skiff_artifact_identity::service_deployment_ref)
            .collect::<BTreeSet<_>>();
        let snapshot_deployments = config_snapshot
            .deployments()
            .iter()
            .map(|deployment| deployment.deployment().clone())
            .collect::<BTreeSet<_>>();
        if actual_deployments != snapshot_deployments
            || actual_deployments.len() != config_snapshot.deployments().len()
        {
            return Err(CanonicalFixtureError::InvalidInput(
                "base config snapshot deployments do not exactly match the base deployment closure"
                    .to_string(),
            ));
        }

        let mut packages = Vec::with_capacity(package_refs.len());
        for reference in package_refs {
            let package = store
                .read_package_artifact(&reference)
                .map_err(|error| {
                    CanonicalFixtureError::InvalidInput(format!(
                        "base package {}@{} is missing from the artifact store: {error}",
                        reference.package_id, reference.package_version
                    ))
                })?
                .as_ref()
                .clone();
            packages.push(package);
        }
        let mut contracts = Vec::with_capacity(contract_refs.len());
        for reference in contract_refs {
            let contract = store.read_service_contract(&reference).map_err(|error| {
                CanonicalFixtureError::InvalidInput(format!(
                    "base contract {}@{} is missing from the artifact store: {error}",
                    reference.service_id, reference.contract_version
                ))
            })?;
            contracts.push(contract.as_ref().clone());
        }

        Ok(Self {
            config_snapshot: Some(config_snapshot),
            packages,
            contracts,
            deployments,
        })
    }
}

/// Test-owned records plus the final immutable deployment closure.
#[derive(Debug, Clone)]
pub struct CanonicalTestRecords {
    pub packages: Vec<PublishedPackageArtifact>,
    pub bytecode: PackageBytecodeLane,
    pub contracts: Vec<ServiceContract>,
    pub deployments: Vec<ServiceDeployment>,
    pub config_snapshot: RuntimeConfigSnapshot,
    pub base: CanonicalBaseClosure,
}

impl CanonicalTestRecords {
    /// Copies the exact external closure into the writable runtime root, then
    /// publishes only test-owned records and the combined config snapshot.
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
            .map(skiff_artifact_identity::service_deployment_ref)
            .collect::<BTreeSet<_>>();

        let mut written = Vec::new();
        for package in &self.base.packages {
            if !owned_packages.contains(&declared_package_ref(package)) {
                copy_package(
                    &source,
                    &target,
                    &declared_package_ref(package),
                    session,
                    &mut written,
                )?;
            }
        }
        for contract in &self.base.contracts {
            let reference = service_contract_ref(contract)
                .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
            if !owned_contracts.contains(&reference) {
                let value = source.read_service_contract(&reference)?;
                written.push(target.write_service_contract(&value)?);
            }
        }
        for deployment in &self.base.deployments {
            let reference = skiff_artifact_identity::service_deployment_ref(deployment);
            if !owned_deployments.contains(&reference) {
                let value = source.read_service_deployment(&reference)?;
                written.push(target.write_service_deployment(&value)?);
            }
        }

        for package in &self.packages {
            publish_package(&target, package, &self.bytecode, session, &mut written)?;
        }
        for contract in &self.contracts {
            written.push(target.write_service_contract(contract)?);
        }
        for deployment in &self.deployments {
            written.push(target.write_service_deployment(deployment)?);
        }
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
    let artifact = source
        .read_package_artifact(reference)
        .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
    if let Some(bytecode_reference) = artifact.bytecode.as_ref() {
        let bytecode = source
            .read_package_bytecode(reference, bytecode_reference)
            .map_err(|error| CanonicalFixtureError::InvalidInput(error.to_string()))?;
        written.push(target.write_package_bytecode(reference, bytecode.artifact())?);
    }
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
    bytecode: &PackageBytecodeLane,
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
    let handoff = bytecode.handoff().ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(format!(
            "test-owned package {} has no bytecode handoff",
            reference.package_build_id
        ))
    })?;
    let receipt = publish_package_artifact_records_with_bytecode(store.root(), package, handoff)
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
