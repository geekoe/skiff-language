use std::sync::Arc;

use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef, PublicationResourceRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

use super::super::super::*;

pub(super) struct TypedResolver {
    pub(super) deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    pub(super) contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    pub(super) packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    pub(super) files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
}

impl RuntimeAssemblyContentResolver for TypedResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, deployment)| Arc::clone(deployment))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, contract)| Arc::clone(contract))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing contract"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, package)| Arc::clone(package))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.files
            .iter()
            .find(|(candidate_package, candidate_file, _)| {
                candidate_package == package && candidate_file == reference
            })
            .map(|(_, _, file)| Arc::clone(file))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("typed execution fixture has no static resources")
    }
}
