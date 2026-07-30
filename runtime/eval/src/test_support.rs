use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use skiff_artifact_model::{
    ActorMetadataIr, DbMetadataIr, FileIrRef, PackageArtifact, PackageBuildId,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndexRef, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, GatewayConfig, LinkOverlay, LinkedFileUnit, LinkedProgramImage,
    OperationRouteBinding, PackageCodeSlotIndex, PublicationResourceTable, RuntimeExecutionPackage,
    RuntimeProgramIdentity, RuntimeTypeContext, ServiceMeta, ServiceTimeoutConfig,
};

use crate::EvalRuntimeProgramSource;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn runtime_execution_package_fixture(
    package_id: &str,
    code_slot: usize,
    files: Vec<Arc<LinkedFileUnit>>,
    static_resources: PublicationResourceTable,
) -> Arc<RuntimeExecutionPackage> {
    runtime_execution_package_fixture_with_identity(
        package_id,
        "1.0.0",
        &format!("{package_id}:build"),
        &format!("{package_id}:abi"),
        code_slot,
        files,
        static_resources,
    )
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn runtime_execution_package_fixture_with_identity(
    package_id: &str,
    package_version: &str,
    package_build_id: &str,
    package_local_abi_identity: &str,
    code_slot: usize,
    files: Vec<Arc<LinkedFileUnit>>,
    static_resources: PublicationResourceTable,
) -> Arc<RuntimeExecutionPackage> {
    let artifact = runtime_execution_package_artifact_fixture(
        package_id,
        package_version,
        package_build_id,
        package_local_abi_identity,
        &files,
        &static_resources,
    );
    runtime_execution_package_from_artifact(code_slot, artifact, files, static_resources)
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn runtime_execution_package_artifact_fixture(
    package_id: &str,
    package_version: &str,
    package_build_id: &str,
    package_local_abi_identity: &str,
    files: &[Arc<LinkedFileUnit>],
    static_resources: &PublicationResourceTable,
) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: package_version.to_string(),
        package_build_id: PackageBuildId::new(package_build_id),
        files: files
            .iter()
            .map(|file| FileIrRef {
                file_ir_identity: file.file_ir_identity.clone(),
                module_path: file.module_path.clone(),
                artifact_path: None,
                source_ast_hash: Some(file.source_ast_hash.clone()),
            })
            .collect(),
        static_resources: static_resources
            .resources_by_path
            .values()
            .map(|resource| resource.meta.clone())
            .collect(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new(package_local_abi_identity),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn runtime_execution_package_from_artifact(
    code_slot: usize,
    artifact: PackageArtifact,
    files: Vec<Arc<LinkedFileUnit>>,
    static_resources: PublicationResourceTable,
) -> Arc<RuntimeExecutionPackage> {
    Arc::new(
        RuntimeExecutionPackage::try_new(
            PackageCodeSlotIndex::new(code_slot),
            Arc::new(artifact),
            files,
            static_resources,
        )
        .expect("test package execution context must be exact"),
    )
}

#[cfg(test)]
pub(crate) fn link_package_fixture(
    assembly: skiff_artifact_model::RuntimeAssembly,
    packages: Vec<(
        skiff_artifact_model::PackageArtifact,
        Vec<skiff_artifact_model::FileIrUnit>,
    )>,
) -> Arc<skiff_runtime_linked_program::AssemblyExecutionImage> {
    use skiff_artifact_model::PackageSchemaIndex;

    skiff_runtime_linker::link_package_fixture_from_runtime_assembly(
        &assembly,
        packages.into_iter().map(|(package, files)| {
            assert!(
                package.package_schema_type_records.is_empty(),
                "package fixture with public schema records must hydrate its real index and records"
            );
            let schema_index = PackageSchemaIndex {
                package_id: package.package_schema_index.package_id.clone(),
                package_schema_index_identity: package
                    .package_schema_index
                    .package_schema_index_identity
                    .clone(),
                types: BTreeMap::new(),
            };
            skiff_runtime_linked_program::HydratedPackageCode::new(
                Arc::new(package),
                files.into_iter().map(Arc::new).collect(),
                skiff_runtime_linked_program::PublicationResourceTable::default(),
            )
            .with_schema_index(Arc::new(schema_index))
        }),
    )
    .expect("package fixture should link through the canonical assembly projection")
}

#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    pub service: ServiceMeta,
    pub version: String,
    pub build_id: String,
    pub service_files: Vec<Arc<LinkedFileUnit>>,
    pub packages: Vec<Arc<RuntimeExecutionPackage>>,
    pub service_resources: skiff_runtime_linked_program::PublicationResourceTable,
    pub timeout: ServiceTimeoutConfig,
    pub operation_route_bindings: Vec<OperationRouteBinding>,
    pub routes: HashMap<String, ExecutableAddr>,
    pub spawn_routes: HashMap<String, ExecutableAddr>,
    pub operations: HashMap<String, ExecutableAddr>,
    pub operation_receivers: HashMap<String, ConstAddr>,
    pub db: Vec<DbMetadataIr>,
    pub actors: Vec<ActorMetadataIr>,
    pub link_overlay: LinkOverlay,
    pub gateway: GatewayConfig,
    pub types: RuntimeTypeContext,
}

impl RuntimeProgram {
    pub fn runtime_program_identity(&self) -> RuntimeProgramIdentity {
        RuntimeProgramIdentity::from_dynamic_build_id(self.build_id.clone())
    }

    pub fn linked_image(&self) -> LinkedProgramImage {
        LinkedProgramImage {
            service_files: self.service_files.clone(),
            packages: self.packages.clone(),
            service_resources: self.service_resources.clone(),
            routes: self.routes.clone(),
            spawn_routes: self.spawn_routes.clone(),
            operations: self.operations.clone(),
            operation_receivers: self.operation_receivers.clone(),
            link_overlay: self.link_overlay.clone(),
            types: self.types.clone(),
        }
    }
}

impl EvalRuntimeProgramSource for RuntimeProgram {
    fn service_id(&self) -> &str {
        &self.service.id
    }

    fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.service_files
    }

    fn packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        &self.packages
    }

    fn service_resources(&self) -> &skiff_runtime_linked_program::PublicationResourceTable {
        &self.service_resources
    }

    fn spawn_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.spawn_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }
}
