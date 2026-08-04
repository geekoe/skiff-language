use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
};

use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef, PackageSchemaIndex,
    PackageSchemaIndexRef, PackageSchemaTypeRecord, PackageSchemaTypeRecordRef,
    PublicationResourceRef, RuntimeAssemblyRef, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::{
    HydratedRuntimeAssembly, RuntimeAssemblyContentResolver, RuntimeAssemblyLoader,
    RuntimeAssemblyRecordResolver,
};

/// Production filesystem resolver for the typed canonical artifact store.
///
/// Every path comes from an exact typed reference. Raw coordinates are checked
/// before typed deserialization by the store; no legacy pointer/index or host
/// admission hook participates in hydration.
#[derive(Debug, Clone)]
pub struct FilesystemRuntimeAssemblyContentResolver {
    store: CanonicalArtifactStore,
    cache: Arc<FilesystemRuntimeAssemblyContentCache>,
}

#[derive(Debug, Default)]
struct FilesystemRuntimeAssemblyContentCache {
    packages: RwLock<HashMap<PackageArtifactRef, Arc<PackageArtifact>>>,
    file_ir: RwLock<HashMap<(PackageArtifactRef, FileIrRef), Arc<FileIrUnit>>>,
}

impl FilesystemRuntimeAssemblyContentResolver {
    pub fn open(artifact_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            store: CanonicalArtifactStore::open(artifact_root)?,
            cache: Arc::new(FilesystemRuntimeAssemblyContentCache::default()),
        })
    }

    pub fn from_store(store: CanonicalArtifactStore) -> Self {
        Self {
            store,
            cache: Arc::new(FilesystemRuntimeAssemblyContentCache::default()),
        }
    }

    pub fn store(&self) -> &CanonicalArtifactStore {
        &self.store
    }

    pub fn load_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<HydratedRuntimeAssembly> {
        RuntimeAssemblyLoader::new(self).load_ref(reference)
    }
}

impl RuntimeAssemblyContentResolver for FilesystemRuntimeAssemblyContentResolver {
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

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        Ok(self.store.read_package_schema_index(reference)?)
    }

    fn resolve_package_schema_type(
        &self,
        reference: &PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<PackageSchemaTypeRecord>> {
        Ok(self.store.read_package_schema_type_record(reference)?)
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        if let Some(artifact) = self
            .cache
            .packages
            .read()
            .expect("package artifact cache poisoned")
            .get(reference)
        {
            return Ok(artifact.clone());
        }
        let artifact = self.store.read_package_artifact(reference)?;
        self.cache
            .packages
            .write()
            .expect("package artifact cache poisoned")
            .insert(reference.clone(), artifact.clone());
        Ok(artifact)
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        let key = (package.clone(), reference.clone());
        if let Some(unit) = self
            .cache
            .file_ir
            .read()
            .expect("file IR cache poisoned")
            .get(&key)
        {
            return Ok(unit.clone());
        }
        let unit = self.store.read_file_ir(package, reference)?;
        self.cache
            .file_ir
            .write()
            .expect("file IR cache poisoned")
            .insert(key, unit.clone());
        Ok(unit)
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        Ok(self.store.read_static_resource(package, reference)?)
    }
}

impl RuntimeAssemblyRecordResolver for FilesystemRuntimeAssemblyContentResolver {
    fn resolve_runtime_assembly(
        &self,
        reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::RuntimeAssembly>> {
        Ok(self.store.read_runtime_assembly(reference)?)
    }
}
