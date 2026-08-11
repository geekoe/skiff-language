use std::{
    borrow::Borrow,
    path::Path,
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BytecodeArtifactRef, PackageArtifact, PackageArtifactRef, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{
    DeploymentBytecodeContentResolver, DeploymentBytecodeHydrationError, DeploymentBytecodeLoader,
    HydratedDeploymentBytecode,
};

/// Production filesystem resolver for exact deployment bytecode hydration.
///
/// Reads are deliberately limited to deployment, contract, package and
/// bytecode records; File IR, schemas and resources are not resolvable from
/// this boundary.
#[derive(Debug, Clone)]
pub struct FilesystemDeploymentBytecodeContentResolver {
    store: CanonicalArtifactStore,
}

impl FilesystemDeploymentBytecodeContentResolver {
    pub fn open(artifact_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            store: CanonicalArtifactStore::open(artifact_root)?,
        })
    }

    pub fn from_store(store: CanonicalArtifactStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &CanonicalArtifactStore {
        &self.store
    }

    pub fn load_deployment_bytecode(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
        DeploymentBytecodeLoader::new(self).load(reference)
    }
}

impl DeploymentBytecodeContentResolver for FilesystemDeploymentBytecodeContentResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        Ok(self.store.read_service_deployment(reference)?)
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        Ok(self.store.read_service_contract(reference)?)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        Ok(self.store.read_package_artifact(reference)?)
    }

    fn resolve_package_bytecode(
        &self,
        package: &PackageArtifactRef,
        reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        Ok(self.store.read_package_bytecode(package, reference)?)
    }
}

/// Loads and hydrates one exact deployment from a canonical artifact store.
pub fn load_deployment_bytecode_from_store(
    store: impl Borrow<CanonicalArtifactStore>,
    reference: &ServiceDeploymentRef,
) -> Result<HydratedDeploymentBytecode, DeploymentBytecodeHydrationError> {
    FilesystemDeploymentBytecodeContentResolver::from_store(store.borrow().clone())
        .load_deployment_bytecode(reference)
}
