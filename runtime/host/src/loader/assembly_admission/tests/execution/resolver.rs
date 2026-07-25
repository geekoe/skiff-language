use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ContractTypeNameability, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef,
    PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaIndexRef, PublicationResourceRef,
    ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
};

use super::super::super::*;

pub(super) struct TypedResolver {
    pub(super) deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    pub(super) contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    pub(super) packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    pub(super) files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
    pub(super) package_schema_records: Vec<(
        skiff_artifact_model::PackageSchemaTypeRecordRef,
        Arc<skiff_artifact_model::PackageSchemaTypeRecord>,
    )>,
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

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        let package = self
            .packages
            .iter()
            .find(|(_, package)| package.package_schema_index == *reference)
            .map(|(_, package)| package)
            .ok_or_else(|| {
                anyhow::anyhow!("typed execution fixture missing package schema index")
            })?;
        let mut types = BTreeMap::new();
        for record_ref in package.package_schema_type_records.values() {
            let record = self
                .package_schema_records
                .iter()
                .find(|(candidate, _)| candidate == record_ref)
                .map(|(_, record)| record)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "typed execution fixture package schema index is missing record {record_ref:?}"
                    )
                })?;
            let entry = PackageSchemaIndexEntry {
                package_schema_type_id: record.package_schema_type_id.clone(),
                public_path: Some(record.stable_schema_key.clone()),
                nameability: ContractTypeNameability::PublicNameable,
            };
            if types
                .insert(record.stable_schema_key.clone(), entry)
                .is_some()
            {
                anyhow::bail!(
                    "typed execution fixture has duplicate public schema key {}",
                    record.stable_schema_key
                );
            }
        }
        let identity =
            skiff_artifact_identity::package_schema_index_identity(&reference.package_id, &types)?;
        if identity != reference.package_schema_index_identity {
            anyhow::bail!("typed execution fixture package schema index identity mismatch");
        }
        Ok(Arc::new(PackageSchemaIndex {
            package_id: reference.package_id.clone(),
            package_schema_index_identity: identity,
            types,
        }))
    }

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        self.package_schema_records
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, record)| Arc::clone(record))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing package schema record"))
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
