use crate::emission::artifact::{
    ArtifactUnit, ArtifactUnitSet, FileIrRef, PublishedFileIrArtifact, PublishedJsonArtifact,
    ARTIFACT_INDEX_SCHEMA_VERSION, BUNDLE_SCHEMA_VERSION, CONTRACT_SCHEMA_ARTIFACT_VERSION,
    FILE_IR_SCHEMA_VERSION, PACKAGE_UNIT_SCHEMA_VERSION, SERVICE_ASSEMBLY_KIND,
    SERVICE_ASSEMBLY_SCHEMA_VERSION, SERVICE_UNIT_SCHEMA_VERSION,
};
use crate::emission::identity::identity;
use crate::error::{EmissionError, Result};
use crate::projection::context::{PackageApiSourceProjection, ProjectedServiceDependencyLockEntry};
use crate::projection::contract::{CanonicalContractProjectionSchema, ContractProjection};
use crate::projection::prelude_metadata::PreludeMetadata;
use crate::projection::runtime::{GatewayEntry, OperationEntryIr, TimeoutEntry};
use crate::projection::runtime_manifest_model::{
    RuntimeServiceAccessManifest, SkiffRuntimeManifest,
};
use crate::projection::service::artifacts::ServiceArtifactProjection;
use crate::projection::service::service_unit::ServicePackageConfigEntry;
use crate::projection::source_map::PublicationSourceMap;
use crate::projection::typed_artifacts::ServiceUnit;
use crate::projection::{
    ConfigActivation, ConfigRequirementsProjection, ConfigShape, ConfigUseEntry,
};
use serde::Serialize;
use skiff_artifact_model::DbMetadataIr;
use skiff_compiler_core::{id::PublicationId, json_utils::value_sha256};

use super::identity::{service_unit_hash, service_unit_identity};
pub use super::identity::{BUNDLE_IDENTITY_PREFIX, SERVICE_ASSEMBLY_IDENTITY_PREFIX};

pub(crate) fn identity_sha256_hash(identity: &str) -> &str {
    identity
        .rsplit_once(":sha256:")
        .map(|(_, hash)| hash)
        .unwrap_or(identity)
}

pub(crate) fn service_id_artifact_path(service_id: &str) -> String {
    PublicationId::parse(service_id)
        .expect("service id was validated before artifact projection")
        .artifact_path()
}

pub(crate) struct ServiceArtifactEmissionInput<'a> {
    pub(crate) manifest: &'a SkiffRuntimeManifest,
    pub(crate) api_source: Option<&'a PackageApiSourceProjection>,
    pub(crate) service_http_response_max_bytes: Option<u64>,
    pub(crate) contract: &'a ContractProjection,
    pub(crate) prelude_metadata: &'a PreludeMetadata,
    pub(crate) config_shape: &'a ConfigShape,
    pub(crate) config_uses: &'a [ConfigUseEntry],
    pub(crate) config_activation: &'a ConfigActivation,
    pub(crate) config_requirements: &'a ConfigRequirementsProjection,
    pub(crate) canonical_contract_schema: &'a CanonicalContractProjectionSchema,
    pub(crate) artifact_projection: &'a ServiceArtifactProjection,
}

#[derive(Clone, Copy)]
struct ServiceObjectEmissionView<'a> {
    manifest: &'a SkiffRuntimeManifest,
    max_response_bytes: Option<u64>,
    contract: &'a ContractProjection,
}

struct ServiceAssemblyEmissionView<'a> {
    service: ServiceObjectEmissionView<'a>,
    api_source: Option<&'a PackageApiSourceProjection>,
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    prelude_metadata: &'a PreludeMetadata,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
    db_metadata: &'a [DbMetadataIr],
    operation_entries: &'a [OperationEntryIr],
    gateway: &'a GatewayEntry,
    timeout: &'a Option<TimeoutEntry>,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
    source_map: &'a PublicationSourceMap,
}

struct ContractSchemaEmissionView<'a> {
    protocol_identity: &'a str,
    schema: &'a CanonicalContractProjectionSchema,
}

struct ServiceBundleEmissionView<'a> {
    prelude_metadata: &'a PreludeMetadata,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
}

struct ServiceIndexEmissionView<'a> {
    service_id: &'a str,
    contract_identity: &'a str,
    service: ServiceObjectEmissionView<'a>,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    prelude_metadata: &'a PreludeMetadata,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
}

/// The `service` object embedded in a service assembly. Optional members are
/// skipped when absent, matching the former conditional `json!` map.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceAssemblyServiceObject {
    id: String,
    revision_id: String,
    protocol_identity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<RuntimeServiceAccessManifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assembly_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    http: Option<ServiceHttpObject>,
    api: ServiceApiObject,
}

#[derive(Debug, Clone, Serialize)]
struct ServiceHttpObject {
    response: ServiceHttpResponseObject,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceHttpResponseObject {
    max_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceApiObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_source: Option<ServiceApiSourceObject>,
    bindings: std::collections::BTreeMap<String, ServiceApiBinding>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceApiSourceObject {
    relative_path: String,
    content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceApiBinding {
    source_module: String,
    source_symbol: String,
}

fn service_assembly_service_object(
    manifest: &SkiffRuntimeManifest,
    assembly_identity: Option<&str>,
    max_response_bytes: Option<u64>,
    contract: &ContractProjection,
    api_source: Option<&PackageApiSourceProjection>,
) -> ServiceAssemblyServiceObject {
    ServiceAssemblyServiceObject {
        id: manifest.service.id.clone(),
        revision_id: manifest.service.revision_id.clone(),
        protocol_identity: manifest.service.protocol_identity.clone(),
        access: manifest.service.access.clone(),
        assembly_identity: assembly_identity.map(ToString::to_string),
        http: max_response_bytes.map(|max_bytes| ServiceHttpObject {
            response: ServiceHttpResponseObject { max_bytes },
        }),
        api: service_api_object(contract, api_source),
    }
}

fn service_api_object(
    contract: &ContractProjection,
    api_source: Option<&PackageApiSourceProjection>,
) -> ServiceApiObject {
    let bindings = contract
        .api_bindings
        .iter()
        .map(|(alias, binding)| {
            (
                alias.clone(),
                ServiceApiBinding {
                    source_module: binding.source_module.clone(),
                    source_symbol: binding.source_symbol.clone(),
                },
            )
        })
        .collect();
    ServiceApiObject {
        api_source: api_source.map(service_api_source_object),
        bindings,
    }
}

fn service_api_source_object(source: &PackageApiSourceProjection) -> ServiceApiSourceObject {
    ServiceApiSourceObject {
        relative_path: source.relative_path.to_string_lossy().into_owned(),
        content_hash: source.content_hash.clone(),
    }
}

fn service_object_artifact_model(
    projection: ServiceObjectEmissionView<'_>,
    assembly_identity: Option<&str>,
    api_source: Option<&PackageApiSourceProjection>,
) -> ServiceAssemblyServiceObject {
    service_assembly_service_object(
        projection.manifest,
        assembly_identity,
        projection.max_response_bytes,
        projection.contract,
        api_source,
    )
}

fn service_assembly_artifact_model<'a>(
    projection: &ServiceAssemblyEmissionView<'a>,
    files: Vec<FileIrArtifactPointer>,
    assembly_identity: Option<&str>,
    service_unit: ServiceUnitArtifactPointer,
) -> ServiceAssemblyArtifact<'a> {
    ServiceAssemblyArtifact {
        schema_version: SERVICE_ASSEMBLY_SCHEMA_VERSION,
        kind: SERVICE_ASSEMBLY_KIND,
        service: service_object_artifact_model(
            projection.service,
            assembly_identity,
            projection.api_source,
        ),
        files,
        package_configs: projection.package_configs,
        prelude_identity: projection.prelude_metadata.identity.as_str(),
        prelude: projection.prelude_metadata,
        config_shape: projection.config_shape,
        config_uses: projection.config_uses,
        config_activation: projection.config_activation,
        config_requirements: projection.config_requirements,
        db: projection.db_metadata,
        operations: projection.operation_entries,
        gateway: projection.gateway,
        timeout: projection.timeout,
        dependency_lock: projection.dependency_lock,
        service_unit,
        source_map: projection.source_map,
    }
}

fn service_bundle_artifact_model<'a>(
    projection: &ServiceBundleEmissionView<'a>,
    bundle_identity: Option<String>,
    assemblies: Vec<ServiceBundleAssemblyPointer>,
    service_unit: ServiceUnitArtifactPointer,
    files: Vec<FileIrArtifactPointer>,
    file_ir_units: Vec<FileIrArtifactPointer>,
    package_units: Vec<PackageUnitArtifactPointer>,
) -> ServiceBundleArtifact<'a> {
    ServiceBundleArtifact {
        schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_identity,
        assemblies,
        prelude_identity: projection.prelude_metadata.identity.as_str(),
        config_shape: projection.config_shape,
        config_uses: projection.config_uses,
        config_activation: projection.config_activation,
        config_requirements: projection.config_requirements,
        service_unit,
        files,
        file_ir_units,
        package_units,
        package_configs: projection.package_configs,
        dependency_lock: projection.dependency_lock,
    }
}

fn service_index_artifact_model<'a>(
    projection: &ServiceIndexEmissionView<'a>,
    contract: ContractArtifactPointer,
    service_assembly: ServiceAssemblyArtifactPointer,
    service_unit: ServiceUnitArtifactPointer,
    bundle: ServiceBundleArtifactPointer,
    files: Vec<FileIrArtifactPointer>,
    file_ir_units: Vec<FileIrArtifactPointer>,
    package_units: Vec<PackageUnitArtifactPointer>,
) -> ServiceIndexArtifact<'a> {
    ServiceIndexArtifact {
        schema_version: ARTIFACT_INDEX_SCHEMA_VERSION,
        service_id: projection.service_id,
        contract_identity: projection.contract_identity,
        contract,
        service: service_object_artifact_model(projection.service, None, None),
        service_assembly,
        service_unit,
        dependency_lock: projection.dependency_lock,
        package_configs: projection.package_configs,
        prelude_identity: projection.prelude_metadata.identity.as_str(),
        prelude: projection.prelude_metadata,
        config_shape: projection.config_shape,
        config_uses: projection.config_uses,
        config_activation: projection.config_activation,
        config_requirements: projection.config_requirements,
        bundle,
        files,
        file_ir_units,
        package_units,
    }
}

/// Explicit emission context for path and identity decisions that must not be
/// hidden in projection helpers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EmissionContext<'a> {
    service_id: &'a str,
    protocol_identity: &'a str,
    file_ir_units: &'a [PublishedFileIrArtifact],
    package_units: &'a [PublishedJsonArtifact],
}

impl<'a> EmissionContext<'a> {
    pub(crate) fn for_manifest(
        manifest: &'a SkiffRuntimeManifest,
        file_ir_units: &'a [PublishedFileIrArtifact],
        package_units: &'a [PublishedJsonArtifact],
    ) -> Self {
        Self {
            service_id: manifest.service.id.as_str(),
            protocol_identity: manifest.service.protocol_identity.as_str(),
            file_ir_units,
            package_units,
        }
    }
}

pub(crate) struct PublishedArtifacts {
    pub(crate) service_assembly: PublishedJsonArtifact,
    pub(crate) service_unit: PublishedJsonArtifact,
    pub(crate) contract_schema: PublishedJsonArtifact,
    pub(crate) bundle: PublishedJsonArtifact,
    pub(crate) index: PublishedJsonArtifact,
}

impl PublishedArtifacts {
    pub(crate) fn emit<'a>(
        input: ServiceArtifactEmissionInput<'a>,
        context: EmissionContext<'a>,
    ) -> Result<Self> {
        emit_service_artifacts(input, context)
    }
}

struct PublishedArtifactUnits<'a> {
    service_unit: ArtifactUnit<ServiceUnit>,
    service_assembly: ArtifactUnit<ServiceAssemblyArtifact<'a>>,
    contract_schema: ArtifactUnit<ContractSchemaArtifact<'a>>,
    bundle: ArtifactUnit<ServiceBundleArtifact<'a>>,
    index: ArtifactUnit<ServiceIndexArtifact<'a>>,
}

impl<'a> ArtifactUnitSet<PublishedArtifactUnits<'a>> {
    fn emit(&self) -> PublishedArtifacts {
        let units = self.units();

        PublishedArtifacts {
            service_assembly: units.service_assembly.to_published_json(),
            service_unit: units.service_unit.to_published_json(),
            contract_schema: units.contract_schema.to_published_json(),
            bundle: units.bundle.to_published_json(),
            index: units.index.to_published_json(),
        }
    }
}

fn emit_service_artifacts<'a>(
    input: ServiceArtifactEmissionInput<'a>,
    context: EmissionContext<'a>,
) -> Result<PublishedArtifacts> {
    Ok(build_published_artifact_units(input, context)?.emit())
}

fn build_published_artifact_units<'a>(
    input: ServiceArtifactEmissionInput<'a>,
    context: EmissionContext<'a>,
) -> Result<ArtifactUnitSet<PublishedArtifactUnits<'a>>> {
    let artifact_projection = input.artifact_projection;
    debug_assert_eq!(
        artifact_projection.file_ir_units.len(),
        context.file_ir_units.len()
    );
    debug_assert_eq!(
        artifact_projection.package_units_typed.len(),
        context.package_units.len()
    );

    let service_object = ServiceObjectEmissionView {
        manifest: input.manifest,
        max_response_bytes: input.service_http_response_max_bytes,
        contract: input.contract,
    };
    let service_assembly = ServiceAssemblyEmissionView {
        service: service_object,
        api_source: input.api_source,
        package_configs: &artifact_projection.package_configs,
        prelude_metadata: input.prelude_metadata,
        config_shape: input.config_shape,
        config_uses: input.config_uses,
        config_activation: input.config_activation,
        config_requirements: input.config_requirements,
        db_metadata: &artifact_projection.db_metadata,
        operation_entries: &artifact_projection.operation_entries,
        gateway: &artifact_projection.gateway,
        timeout: &artifact_projection.timeout,
        dependency_lock: &artifact_projection.dependency_lock,
        source_map: &artifact_projection.source_map,
    };
    let service_bundle = ServiceBundleEmissionView {
        prelude_metadata: input.prelude_metadata,
        config_shape: input.config_shape,
        config_uses: input.config_uses,
        config_activation: input.config_activation,
        config_requirements: input.config_requirements,
        package_configs: &artifact_projection.package_configs,
        dependency_lock: &artifact_projection.dependency_lock,
    };
    let service_index = ServiceIndexEmissionView {
        service_id: input.manifest.service.id.as_str(),
        contract_identity: input.manifest.service.protocol_identity.as_str(),
        service: service_object,
        dependency_lock: &artifact_projection.dependency_lock,
        package_configs: &artifact_projection.package_configs,
        prelude_metadata: input.prelude_metadata,
        config_shape: input.config_shape,
        config_uses: input.config_uses,
        config_activation: input.config_activation,
        config_requirements: input.config_requirements,
    };
    let contract_schema = ContractSchemaEmissionView {
        protocol_identity: input.manifest.service.protocol_identity.as_str(),
        schema: input.canonical_contract_schema,
    };

    let mut service_unit_model = artifact_projection.service_unit.clone();
    attach_published_file_paths_to_service_unit(
        &mut service_unit_model.files,
        context.file_ir_units,
    );
    let service_unit_hash = service_unit_hash(&service_unit_model)?;
    let service_unit_identity = service_unit_identity(&service_unit_model)?;
    let service_id_path = service_id_artifact_path(context.service_id);
    let service_unit_path = format!("units/services/{service_id_path}/{service_unit_hash}.json");
    let service_unit = ArtifactUnit {
        model: service_unit_model,
        identity: service_unit_identity.clone(),
        hash: service_unit_hash.clone(),
        path: service_unit_path.clone(),
    };
    let service_unit_pointer = ServiceUnitArtifactPointer {
        schema_version: SERVICE_UNIT_SCHEMA_VERSION,
        unit_identity: service_unit_identity.clone(),
        unit_hash: service_unit_hash.clone(),
        unit_path: service_unit_path.clone(),
    };
    let package_unit_pointers = context
        .package_units
        .iter()
        .map(package_unit_pointer)
        .collect::<Result<Vec<_>>>()?;
    let file_ir_unit_pointers = context
        .file_ir_units
        .iter()
        .map(file_ir_artifact_pointer)
        .collect::<Vec<_>>();
    let service_assembly_hash_model = service_assembly_artifact_model(
        &service_assembly,
        file_ir_unit_pointers.clone(),
        None,
        service_unit_pointer.clone(),
    );
    let service_assembly_hash = value_sha256(&artifact_model_value(&service_assembly_hash_model));
    let service_assembly_identity =
        identity(SERVICE_ASSEMBLY_IDENTITY_PREFIX, &service_assembly_hash);
    let service_id_path = service_id_artifact_path(context.service_id);
    let service_assembly_path =
        format!("assemblies/services/{service_id_path}/{service_assembly_hash}.json");
    let service_assembly = ArtifactUnit {
        model: service_assembly_artifact_model(
            &service_assembly,
            file_ir_unit_pointers.clone(),
            Some(service_assembly_identity.as_str()),
            service_unit_pointer.clone(),
        ),
        identity: service_assembly_identity.clone(),
        hash: service_assembly_hash,
        path: service_assembly_path.clone(),
    };

    let bundle_assemblies = vec![ServiceBundleAssemblyPointer {
        kind: SERVICE_ASSEMBLY_KIND,
        identity: service_assembly_identity.clone(),
        path: service_assembly_path.clone(),
    }];
    let bundle_hash_model = service_bundle_artifact_model(
        &service_bundle,
        None,
        bundle_assemblies.clone(),
        service_unit_pointer.clone(),
        file_ir_unit_pointers.clone(),
        file_ir_unit_pointers.clone(),
        package_unit_pointers.clone(),
    );
    let bundle_hash = value_sha256(&artifact_model_value(&bundle_hash_model));
    let bundle_identity = identity(BUNDLE_IDENTITY_PREFIX, &bundle_hash);
    let bundle_path = format!("bundles/{bundle_hash}.json");
    let service_bundle = ArtifactUnit {
        model: service_bundle_artifact_model(
            &service_bundle,
            Some(bundle_identity.clone()),
            bundle_assemblies,
            service_unit_pointer.clone(),
            file_ir_unit_pointers.clone(),
            file_ir_unit_pointers.clone(),
            package_unit_pointers.clone(),
        ),
        identity: bundle_identity.clone(),
        hash: bundle_hash,
        path: bundle_path.clone(),
    };

    let protocol_hash = identity_sha256_hash(context.protocol_identity).to_string();
    let contract_schema_path = format!("contracts/{protocol_hash}.json");
    let contract_schema = ArtifactUnit {
        model: ContractSchemaArtifact {
            schema_version: CONTRACT_SCHEMA_ARTIFACT_VERSION,
            contract_hash: protocol_hash.clone(),
            protocol_identity: contract_schema.protocol_identity,
            schema: contract_schema.schema,
        },
        identity: context.protocol_identity.to_string(),
        hash: protocol_hash.clone(),
        path: contract_schema_path.clone(),
    };
    let index_path = format!("indexes/services/{service_id_path}/{protocol_hash}.json");
    let index = ArtifactUnit {
        model: service_index_artifact_model(
            &service_index,
            ContractArtifactPointer {
                contract_hash: protocol_hash.clone(),
                protocol_identity: context.protocol_identity.to_string(),
                schema_path: contract_schema_path.clone(),
            },
            ServiceAssemblyArtifactPointer {
                assembly_identity: service_assembly_identity.clone(),
                assembly_path: service_assembly_path.clone(),
            },
            service_unit_pointer,
            ServiceBundleArtifactPointer {
                bundle_identity: bundle_identity.clone(),
                bundle_path: bundle_path.clone(),
            },
            file_ir_unit_pointers.clone(),
            file_ir_unit_pointers,
            package_unit_pointers,
        ),
        identity: String::new(),
        hash: protocol_hash,
        path: index_path,
    };

    Ok(ArtifactUnitSet::new(PublishedArtifactUnits {
        service_unit,
        service_assembly,
        contract_schema,
        bundle: service_bundle,
        index,
    }))
}

fn artifact_model_value<T>(model: &T) -> serde_json::Value
where
    T: Serialize,
{
    serde_json::to_value(model).expect("service artifact model must serialize")
}

fn attach_published_file_paths_to_service_unit(
    refs: &mut [FileIrRef],
    artifacts: &[PublishedFileIrArtifact],
) {
    let by_identity = artifacts
        .iter()
        .map(|artifact| (artifact.identity.as_str(), artifact))
        .collect::<std::collections::BTreeMap<_, _>>();
    for file_ref in refs {
        if let Some(artifact) = by_identity.get(file_ref.file_ir_identity.as_str()) {
            file_ref.artifact_path = Some(artifact.path.clone());
            file_ref.source_ast_hash = Some(artifact.unit.source_ast_hash.clone());
        }
    }
}

/// Pointer to a published File IR unit, embedded in service assemblies,
/// bundles, and indexes. Identity, hash, and path are emission facts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIrArtifactPointer {
    schema_version: &'static str,
    file_ir_identity: String,
    file_ir_hash: String,
    file_ir_path: String,
    source_path: String,
    module_path: String,
    role: String,
}

fn file_ir_artifact_pointer(artifact: &PublishedFileIrArtifact) -> FileIrArtifactPointer {
    FileIrArtifactPointer {
        schema_version: FILE_IR_SCHEMA_VERSION,
        file_ir_identity: artifact.identity.clone(),
        file_ir_hash: artifact.hash.clone(),
        file_ir_path: artifact.path.clone(),
        source_path: artifact.source_path.clone(),
        module_path: artifact.module_path.clone(),
        role: artifact.role.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceUnitArtifactPointer {
    schema_version: &'static str,
    unit_identity: String,
    unit_hash: String,
    unit_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceAssemblyArtifact<'a> {
    schema_version: &'static str,
    kind: &'static str,
    service: ServiceAssemblyServiceObject,
    files: Vec<FileIrArtifactPointer>,
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    prelude_identity: &'a str,
    prelude: &'a PreludeMetadata,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
    #[serde(rename = "db")]
    db: &'a [DbMetadataIr],
    operations: &'a [OperationEntryIr],
    gateway: &'a GatewayEntry,
    timeout: &'a Option<TimeoutEntry>,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
    service_unit: ServiceUnitArtifactPointer,
    source_map: &'a PublicationSourceMap,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceBundleAssemblyPointer {
    kind: &'static str,
    identity: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceBundleArtifact<'a> {
    schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_identity: Option<String>,
    assemblies: Vec<ServiceBundleAssemblyPointer>,
    prelude_identity: &'a str,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
    service_unit: ServiceUnitArtifactPointer,
    files: Vec<FileIrArtifactPointer>,
    file_ir_units: Vec<FileIrArtifactPointer>,
    package_units: Vec<PackageUnitArtifactPointer>,
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractSchemaArtifact<'a> {
    schema_version: &'static str,
    contract_hash: String,
    protocol_identity: &'a str,
    schema: &'a CanonicalContractProjectionSchema,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractArtifactPointer {
    contract_hash: String,
    protocol_identity: String,
    schema_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceAssemblyArtifactPointer {
    assembly_identity: String,
    assembly_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceBundleArtifactPointer {
    bundle_identity: String,
    bundle_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceIndexArtifact<'a> {
    schema_version: &'static str,
    service_id: &'a str,
    contract_identity: &'a str,
    contract: ContractArtifactPointer,
    service: ServiceAssemblyServiceObject,
    service_assembly: ServiceAssemblyArtifactPointer,
    service_unit: ServiceUnitArtifactPointer,
    dependency_lock: &'a [ProjectedServiceDependencyLockEntry],
    package_configs: &'a std::collections::BTreeMap<String, ServicePackageConfigEntry>,
    prelude_identity: &'a str,
    prelude: &'a PreludeMetadata,
    config_shape: &'a ConfigShape,
    config_uses: &'a [ConfigUseEntry],
    config_activation: &'a ConfigActivation,
    config_requirements: &'a ConfigRequirementsProjection,
    bundle: ServiceBundleArtifactPointer,
    files: Vec<FileIrArtifactPointer>,
    file_ir_units: Vec<FileIrArtifactPointer>,
    package_units: Vec<PackageUnitArtifactPointer>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageUnitArtifactPointer {
    schema_version: &'static str,
    package_id: String,
    version: String,
    build_identity: String,
    abi_identity: String,
    unit_hash: String,
    unit_path: String,
}

pub(crate) fn package_unit_pointer(
    artifact: &PublishedJsonArtifact,
) -> Result<PackageUnitArtifactPointer> {
    let object = artifact
        .value
        .as_object()
        .ok_or_else(|| EmissionError::ContractValidation {
            message: format!("package unit artifact {} must be an object", artifact.path),
        })?;
    let schema_version = package_unit_string_field(object, "schemaVersion", &artifact.path)?;
    if schema_version != PACKAGE_UNIT_SCHEMA_VERSION {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "package unit artifact {} schemaVersion must be {}",
                artifact.path, PACKAGE_UNIT_SCHEMA_VERSION
            ),
        });
    }
    let package_id = package_unit_string_field(object, "packageId", &artifact.path)?;
    let version = package_unit_string_field(object, "version", &artifact.path)?;
    let build_identity = package_unit_string_field(object, "buildIdentity", &artifact.path)?;
    let abi_identity = package_unit_string_field(object, "abiIdentity", &artifact.path)?;
    if build_identity != artifact.identity {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "package unit artifact {} buildIdentity {} does not match published identity {}",
                artifact.path, build_identity, artifact.identity
            ),
        });
    }
    Ok(PackageUnitArtifactPointer {
        schema_version: PACKAGE_UNIT_SCHEMA_VERSION,
        package_id,
        version,
        build_identity,
        abi_identity,
        unit_hash: artifact.hash.clone(),
        unit_path: artifact.path.clone(),
    })
}

fn package_unit_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
    path: &str,
) -> Result<String> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| EmissionError::ContractValidation {
            message: format!("package unit artifact {path} missing string {field}"),
        })
}

#[cfg(test)]
mod tests {
    use super::{package_unit_pointer, service_id_artifact_path};
    use crate::emission::artifact::PublishedJsonArtifact;
    use serde_json::json;

    #[test]
    fn service_id_artifact_path_projects_url_like_id_to_single_segment() {
        assert_eq!(
            service_id_artifact_path("skiff.run/account"),
            "skiff~run~~account"
        );
    }

    #[test]
    fn package_unit_pointer_uses_published_artifact_content_for_identity_fields() {
        let artifact = PublishedJsonArtifact {
            value: json!({
                "schemaVersion": "skiff-package-unit-v1",
                "packageId": "example.com/package-a",
                "version": "1.0.0",
                "buildIdentity": "skiff-package-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "abiIdentity": "skiff-package-abi-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }),
            identity: "skiff-package-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            hash: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_string(),
            path: "units/packages/example~com~~package-a/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.json"
                .to_string(),
        };

        let pointer = package_unit_pointer(&artifact).expect("package pointer should build");

        assert_eq!(pointer.package_id, "example.com/package-a");
        assert_eq!(pointer.version, "1.0.0");
        assert_eq!(pointer.build_identity, artifact.identity);
        assert_eq!(
            pointer.unit_path,
            "units/packages/example~com~~package-a/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.json"
        );
    }
}
