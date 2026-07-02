use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, ConfigAndEffectMetadata, ConstIr, ExecutableIr,
    ExternalRefTable, FileDeclarations, FileIrRef, FileIrUnit, FileLinkTargets,
    InterfaceInstantiationRef, MetadataValue, OperationAbiRef, OperationTargetRef,
    PackageDependencyConstraint, PackageImplementationLinks, PackageTestAssembly,
    PackageTestEntrypoint, PackageUnit, PackageUsedSymbol, PublicationAbiUnit,
    PublicationConformanceFact, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, PublicationSchemaType, ServiceDependencyConstraint,
    ServiceMeta, ServiceOperation, ServiceTimeoutConfig, ServiceUnit, SourceMapSource,
    SourceMapSpan, TypeDeclIr,
};
use thiserror::Error;

pub mod package_resolver;

pub use package_resolver::{
    ordered_package_build_identities_from_artifact_root, ordered_package_units_from_artifact_root,
    runtime_program_dynamic_build_id_from_artifact_root,
};

pub const RUNTIME_PROGRAM_BUILD_SCHEMA_MARKER: &str = "skiff-runtime-program-link-v1";
pub const SERVICE_BUILD_IDENTITY_PREFIX: &str = "skiff-service-build-v1";
pub const FILE_IR_IDENTITY_PREFIX: &str = "skiff-file-ir-v3:sha256";
pub const SERVICE_UNIT_IDENTITY_PREFIX: &str = "skiff-service-unit-v1:sha256";
pub const PACKAGE_BUILD_IDENTITY_PREFIX: &str = "skiff-package-build-v1:sha256";
pub const PACKAGE_ABI_IDENTITY_PREFIX: &str = "skiff-package-abi-v1:sha256";
pub const OPERATION_ABI_IDENTITY_PREFIX: &str = "skiff-operation-abi-v1:sha256";
pub const PUBLICATION_ABI_IDENTITY_PREFIX: &str = "skiff-publication-abi-v1:sha256";
pub const PACKAGE_ASSEMBLY_IDENTITY_PREFIX: &str = "skiff-package-assembly-v1:sha256";
pub const SERVICE_ASSEMBLY_IDENTITY_PREFIX: &str = "skiff-service-assembly-v1:sha256";
pub const BUNDLE_IDENTITY_PREFIX: &str = "skiff-bundle-v1:sha256";
pub const PACKAGE_TEST_BUILD_IDENTITY_PREFIX: &str = "skiff-package-test-build-v1:sha256";
pub const PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX: &str =
    "skiff-package-test-entrypoint-local-v1:sha256";
pub const PACKAGE_TEST_ENTRYPOINT_ID_PREFIX: &str = "skiff-package-test-entrypoint-v1:sha256";

#[derive(Debug, Error)]
pub enum ArtifactIdentityError {
    #[error("service unit must be a JSON object")]
    ServiceUnitMustBeObject,
    #[error("failed to serialize File IR identity payload: {0}")]
    SerializeFileIrIdentity(serde_json::Error),
    #[error("File IR unit declared fileIrIdentity {declared} but content identity is {computed}")]
    FileIrIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize package build identity payload: {0}")]
    SerializePackageBuildIdentity(serde_json::Error),
    #[error("failed to serialize package ABI identity payload: {0}")]
    SerializePackageAbiIdentity(serde_json::Error),
    #[error(
        "package unit declared buildIdentity {declared} but content build identity is {computed}"
    )]
    PackageBuildIdentityMismatch { declared: String, computed: String },
    #[error("package unit declared abiIdentity {declared} but content ABI identity is {computed}")]
    PackageAbiIdentityMismatch { declared: String, computed: String },
    #[error("failed to serialize service unit for runtime program identity: {0}")]
    SerializeServiceUnit(serde_json::Error),
    #[error("failed to serialize service unit storage identity payload: {0}")]
    SerializeServiceUnitStorageIdentity(serde_json::Error),
    #[error("failed to serialize operation ABI identity payload: {0}")]
    SerializeOperationAbiIdentity(serde_json::Error),
    #[error("service unit is invalid: {0}")]
    InvalidServiceUnit(serde_json::Error),
    #[error("package unit {path} is invalid: {source}")]
    InvalidPackageUnit {
        path: String,
        source: serde_json::Error,
    },
    #[error("package unit {path} schemaVersion must be {expected}, got {actual}")]
    PackageUnitSchemaVersionMismatch {
        path: String,
        expected: &'static str,
        actual: String,
    },
    #[error("failed to serialize runtime program service unit identity: {0}")]
    SerializeRuntimeProgramServiceUnitIdentity(serde_json::Error),
    #[error("{label} missing publicationAbi")]
    MissingPublicationAbi { label: String },
    #[error("{label} publicationAbi is invalid: {source}")]
    InvalidPublicationAbi {
        label: String,
        source: serde_json::Error,
    },
    #[error("failed to serialize publicationAbi identity payload: {0}")]
    SerializePublicationAbiIdentity(serde_json::Error),
    #[error("failed to serialize package test build identity payload: {0}")]
    SerializePackageTestBuildIdentity(serde_json::Error),
    #[error(
        "package test assembly declared testBuildIdentity {declared} but content identity is {computed}"
    )]
    PackageTestBuildIdentityMismatch { declared: String, computed: String },
    #[error(
        "package test entrypoint {entrypoint_local_id} declared entrypointId {declared} but derived id is {computed}"
    )]
    PackageTestEntrypointIdMismatch {
        entrypoint_local_id: String,
        declared: String,
        computed: String,
    },
    #[error("artifact path {path} for {label} must be relative and stay inside artifacts root")]
    PathEscape { label: String, path: String },
    #[error("artifact {path} was not found")]
    ArtifactNotFound { path: String },
    #[error("failed to read artifact {path}: {source}")]
    ReadArtifact {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse artifact {path}: {source}")]
    ParseArtifactJson {
        path: String,
        source: serde_json::Error,
    },
    #[error("failed to resolve artifact root {path}: {source}")]
    ResolveArtifactRoot {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to resolve artifact {path}: {source}")]
    ResolveArtifactPath {
        path: String,
        source: std::io::Error,
    },
    #[error("{label} path {path} escapes artifacts root {root}")]
    ArtifactPathEscapesRoot {
        label: String,
        path: String,
        root: String,
    },
    #[error("package dependency cycle includes {package_id}")]
    PackageDependencyCycle { package_id: String },
    #[error("package {package_id} is resolved to both {existing_build} and {new_build}")]
    PackageDependencyConflict {
        package_id: String,
        existing_build: String,
        new_build: String,
    },
    #[error("{message}")]
    InvalidPackageIndex { message: String },
    #[error("{label} {value} must be a publication id")]
    InvalidPublicationId { label: String, value: String },
    #[error("{label} {value} is not a safe artifact path segment")]
    InvalidArtifactSegment { label: String, value: String },
}

pub type Result<T> = std::result::Result<T, ArtifactIdentityError>;

pub fn file_ir_hash(unit: &FileIrUnit) -> Result<String> {
    Ok(sha256_hex(&canonical_file_ir_identity_bytes(unit)?))
}

pub fn file_ir_identity(unit: &FileIrUnit) -> Result<String> {
    Ok(identity(FILE_IR_IDENTITY_PREFIX, &file_ir_hash(unit)?))
}

pub fn canonical_file_ir_identity_value(unit: &FileIrUnit) -> Result<Value> {
    let value = serde_json::to_value(FileIrIdentityPayload::from_unit(unit))
        .map_err(ArtifactIdentityError::SerializeFileIrIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn canonical_file_ir_identity_bytes(unit: &FileIrUnit) -> Result<Vec<u8>> {
    let value = canonical_file_ir_identity_value(unit)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializeFileIrIdentity)
}

pub fn validate_file_ir_identity(unit: &FileIrUnit) -> Result<()> {
    let computed = file_ir_identity(unit)?;
    if unit.file_ir_identity != computed {
        return Err(ArtifactIdentityError::FileIrIdentityMismatch {
            declared: unit.file_ir_identity.clone(),
            computed,
        });
    }
    Ok(())
}

pub fn assign_file_ir_identity(unit: &mut FileIrUnit) -> Result<String> {
    let computed = file_ir_identity(unit)?;
    unit.file_ir_identity = computed.clone();
    Ok(computed)
}

pub fn file_ir_with_identity(mut unit: FileIrUnit) -> Result<FileIrUnit> {
    assign_file_ir_identity(&mut unit)?;
    Ok(unit)
}

pub fn service_unit_hash(unit: &ServiceUnit) -> Result<String> {
    Ok(sha256_hex(&service_unit_identity_bytes(unit)?))
}

pub fn service_unit_identity(unit: &ServiceUnit) -> Result<String> {
    Ok(identity(
        SERVICE_UNIT_IDENTITY_PREFIX,
        &service_unit_hash(unit)?,
    ))
}

pub fn service_unit_identity_value(unit: &ServiceUnit) -> Result<Value> {
    let value = serde_json::to_value(ServiceUnitStorageIdentityPayload {
        identity_schema: "skiff-service-unit-identity-v1",
        unit,
    })
    .map_err(ArtifactIdentityError::SerializeServiceUnitStorageIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn service_unit_identity_bytes(unit: &ServiceUnit) -> Result<Vec<u8>> {
    let value = service_unit_identity_value(unit)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializeServiceUnitStorageIdentity)
}

pub fn package_build_hash(unit: &PackageUnit) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        &PackageBuildIdentityPayload {
            schema_version: &unit.schema_version,
            package_id: &unit.package_id,
            version: &unit.version,
            publication_abi: &unit.publication_abi,
            files: &unit.files,
            dependencies: &unit.dependencies,
            implementation_links: &unit.implementation_links,
            config_and_effect_metadata: &unit.config_and_effect_metadata,
        },
        ArtifactIdentityError::SerializePackageBuildIdentity,
    )?))
}

pub fn package_build_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_BUILD_IDENTITY_PREFIX,
        &package_build_hash(unit)?,
    ))
}

pub fn package_abi_hash(unit: &PackageUnit) -> Result<String> {
    publication_abi_hash(&unit.publication_abi)
}

pub fn package_abi_identity(unit: &PackageUnit) -> Result<String> {
    Ok(identity(
        PACKAGE_ABI_IDENTITY_PREFIX,
        &package_abi_hash(unit)?,
    ))
}

pub fn publication_abi_hash(unit: &PublicationAbiUnit) -> Result<String> {
    Ok(sha256_hex(&publication_abi_identity_bytes(unit)?))
}

pub fn publication_abi_identity(unit: &PublicationAbiUnit) -> Result<String> {
    Ok(identity(
        PUBLICATION_ABI_IDENTITY_PREFIX,
        &publication_abi_hash(unit)?,
    ))
}

pub fn operation_abi_hash(input: &OperationAbiIdentityInput<'_>) -> Result<String> {
    Ok(sha256_hex(&canonical_ir_bytes(
        input,
        ArtifactIdentityError::SerializeOperationAbiIdentity,
    )?))
}

pub fn operation_abi_identity(input: &OperationAbiIdentityInput<'_>) -> Result<String> {
    Ok(identity(
        OPERATION_ABI_IDENTITY_PREFIX,
        &operation_abi_hash(input)?,
    ))
}

pub fn public_function_operation_abi_id(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> Result<String> {
    operation_abi_identity(&OperationAbiIdentityInput {
        kind: PublicationOperationKind::PublicFunction,
        public_path,
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    })
}

pub fn public_instance_method_operation_abi_id(
    public_path: &str,
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_abi_id: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> Result<String> {
    operation_abi_identity(&OperationAbiIdentityInput {
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path,
        public_instance_key: Some(public_instance_key),
        interface: Some(interface),
        method_abi_id: Some(method_abi_id),
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    })
}

pub fn validate_package_unit_identities(unit: &PackageUnit) -> Result<()> {
    let computed_build = package_build_identity(unit)?;
    if unit.build_identity != computed_build {
        return Err(ArtifactIdentityError::PackageBuildIdentityMismatch {
            declared: unit.build_identity.clone(),
            computed: computed_build,
        });
    }

    let computed_abi = package_abi_identity(unit)?;
    if unit.abi_identity != computed_abi {
        return Err(ArtifactIdentityError::PackageAbiIdentityMismatch {
            declared: unit.abi_identity.clone(),
            computed: computed_abi,
        });
    }

    Ok(())
}

pub fn assign_publication_abi_identity(unit: &mut PublicationAbiUnit) -> Result<String> {
    let abi_identity = publication_abi_identity(unit)?;
    unit.abi_identity = abi_identity.clone();
    Ok(abi_identity)
}

pub fn assign_package_unit_identities(unit: &mut PackageUnit) -> Result<(String, String)> {
    unit.publication_abi.publication_id = unit.package_id.clone();
    unit.publication_abi.version = unit.version.clone();
    assign_publication_abi_identity(&mut unit.publication_abi)?;
    let abi_identity = package_abi_identity(unit)?;
    unit.abi_identity = abi_identity.clone();
    normalize_package_dependency_configs(unit);
    let build_identity = package_build_identity(unit)?;
    unit.build_identity = build_identity.clone();
    Ok((build_identity, abi_identity))
}

fn normalize_package_dependency_configs(unit: &mut PackageUnit) {
    for dependency in &mut unit.dependencies {
        if dependency.config.is_null() {
            dependency.config = Value::Object(Map::new());
        }
    }
}

pub fn package_test_build_hash(assembly: &PackageTestAssembly) -> Result<String> {
    Ok(sha256_hex(&canonical_package_test_build_identity_bytes(
        assembly,
    )?))
}

pub fn package_test_build_identity(assembly: &PackageTestAssembly) -> Result<String> {
    Ok(identity(
        PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
        &package_test_build_hash(assembly)?,
    ))
}

pub fn canonical_package_test_build_identity_value(
    assembly: &PackageTestAssembly,
) -> Result<Value> {
    let value = serde_json::to_value(PackageTestBuildIdentityPayload::from_assembly(assembly))
        .map_err(ArtifactIdentityError::SerializePackageTestBuildIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn canonical_package_test_build_identity_bytes(
    assembly: &PackageTestAssembly,
) -> Result<Vec<u8>> {
    let value = canonical_package_test_build_identity_value(assembly)?;
    serde_json::to_vec(&value).map_err(ArtifactIdentityError::SerializePackageTestBuildIdentity)
}

pub fn validate_package_test_assembly_identity(assembly: &PackageTestAssembly) -> Result<()> {
    let computed_build = package_test_build_identity(assembly)?;
    if assembly.test_build_identity != computed_build {
        return Err(ArtifactIdentityError::PackageTestBuildIdentityMismatch {
            declared: assembly.test_build_identity.clone(),
            computed: computed_build,
        });
    }

    for entrypoint in &assembly.test_entrypoints {
        let computed = derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &entrypoint.entrypoint_local_id,
        )?;
        if entrypoint.entrypoint_id != computed {
            return Err(ArtifactIdentityError::PackageTestEntrypointIdMismatch {
                entrypoint_local_id: entrypoint.entrypoint_local_id.clone(),
                declared: entrypoint.entrypoint_id.clone(),
                computed,
            });
        }
    }

    Ok(())
}

pub fn package_test_entrypoint_local_id(
    package_id: &str,
    package_version: &str,
    source_path: &str,
    test_ordinal: u32,
    normalized_test_name: &str,
) -> Result<String> {
    let hash = sha256_hex(&canonical_ir_bytes(
        &PackageTestEntrypointLocalIdPayload {
            schema: "skiff-package-test-entrypoint-local-v1",
            package_id,
            package_version,
            source_path,
            test_ordinal,
            normalized_test_name,
        },
        ArtifactIdentityError::SerializePackageTestBuildIdentity,
    )?);
    Ok(identity(PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX, &hash))
}

pub fn derive_package_test_entrypoint_id(
    test_build_identity: &str,
    entrypoint_local_id: &str,
) -> Result<String> {
    let hash = sha256_hex(&canonical_ir_bytes(
        &PackageTestEntrypointIdPayload {
            schema: "skiff-package-test-entrypoint-v1",
            test_build_identity,
            entrypoint_local_id,
        },
        ArtifactIdentityError::SerializePackageTestBuildIdentity,
    )?);
    Ok(identity(PACKAGE_TEST_ENTRYPOINT_ID_PREFIX, &hash))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceUnitStorageIdentityPayload<'a> {
    identity_schema: &'static str,
    unit: &'a ServiceUnit,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIrIdentityPayload<'a> {
    schema_version: &'a str,
    module_path: &'a str,
    ir_format_version: &'a str,
    opcode_table_version: &'a str,
    #[serde(skip_serializing_if = "is_zero_u32")]
    required_receiver_builtin_capability_version: u32,
    source_map: SourceMapIdentityPayload<'a>,
    declarations: &'a FileDeclarations,
    link_targets: &'a FileLinkTargets,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    type_table: &'a Vec<TypeDeclIr>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    constants: &'a Vec<ConstIr>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    executables: &'a Vec<ExecutableIr>,
    external_refs: &'a ExternalRefTable,
}

impl<'a> FileIrIdentityPayload<'a> {
    fn from_unit(unit: &'a FileIrUnit) -> Self {
        Self {
            schema_version: &unit.schema_version,
            module_path: &unit.module_path,
            ir_format_version: &unit.ir_format_version,
            opcode_table_version: &unit.opcode_table_version,
            required_receiver_builtin_capability_version: unit
                .required_receiver_builtin_capability_version,
            source_map: SourceMapIdentityPayload::from_unit(unit),
            declarations: &unit.declarations,
            link_targets: &unit.link_targets,
            type_table: &unit.type_table,
            constants: &unit.constants,
            executables: &unit.executables,
            external_refs: &unit.external_refs,
        }
    }
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapIdentityPayload<'a> {
    format: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<SourceMapSourceIdentityPayload<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    spans: &'a Vec<SourceMapSpan>,
}

impl<'a> SourceMapIdentityPayload<'a> {
    fn from_unit(unit: &'a FileIrUnit) -> Self {
        Self {
            format: &unit.source_map.format,
            sources: unit
                .source_map
                .sources
                .iter()
                .map(SourceMapSourceIdentityPayload::from_source)
                .collect(),
            spans: &unit.source_map.spans,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceMapSourceIdentityPayload<'a> {
    id: u64,
    path: &'a str,
    module_path: &'a str,
}

impl<'a> SourceMapSourceIdentityPayload<'a> {
    fn from_source(source: &'a SourceMapSource) -> Self {
        Self {
            id: source.id,
            path: &source.path,
            module_path: &source.module_path,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageBuildIdentityPayload<'a> {
    schema_version: &'a str,
    package_id: &'a str,
    version: &'a str,
    publication_abi: &'a PublicationAbiUnit,
    files: &'a [FileIrRef],
    dependencies: &'a [PackageDependencyConstraint],
    implementation_links: &'a PackageImplementationLinks,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestBuildIdentityPayload<'a> {
    schema_version: &'a str,
    kind: skiff_artifact_model::PackageTestAssemblyKind,
    package_id: &'a str,
    package_version: &'a str,
    production_package_unit: &'a skiff_artifact_model::PackageTestPackageUnitRef,
    dependency_package_units: &'a [skiff_artifact_model::PackageTestPackageUnitRef],
    test_file_identities: Vec<&'a str>,
    link_policy: &'a skiff_artifact_model::PackageTestLinkPolicy,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
    test_entrypoints: Vec<PackageTestEntrypointIdentityProjection<'a>>,
}

impl<'a> PackageTestBuildIdentityPayload<'a> {
    fn from_assembly(assembly: &'a PackageTestAssembly) -> Self {
        Self {
            schema_version: &assembly.schema_version,
            kind: assembly.kind,
            package_id: &assembly.package_id,
            package_version: &assembly.package_version,
            production_package_unit: &assembly.production_package_unit,
            dependency_package_units: &assembly.dependency_package_units,
            test_file_identities: assembly
                .test_files
                .iter()
                .map(|file| file.file_ir_identity.as_str())
                .collect(),
            link_policy: &assembly.link_policy,
            config_and_effect_metadata: &assembly.config_and_effect_metadata,
            test_entrypoints: assembly
                .test_entrypoints
                .iter()
                .map(PackageTestEntrypointIdentityProjection::from_entrypoint)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointIdentityProjection<'a> {
    entrypoint_local_id: &'a str,
    display_name: &'a str,
    source_path: &'a str,
    module_path: &'a str,
    owner_test_file_identity: &'a str,
    executable_ref: &'a skiff_artifact_model::PackageTestExecutableRef,
    default_run: bool,
    config_and_effect_metadata: &'a ConfigAndEffectMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_expected_error: Option<&'a skiff_artifact_model::PackageTestRuntimeExpectedError>,
}

impl<'a> PackageTestEntrypointIdentityProjection<'a> {
    fn from_entrypoint(entrypoint: &'a PackageTestEntrypoint) -> Self {
        Self {
            entrypoint_local_id: &entrypoint.entrypoint_local_id,
            display_name: &entrypoint.display_name,
            source_path: &entrypoint.source_path,
            module_path: &entrypoint.module_path,
            owner_test_file_identity: &entrypoint.owner_test_file.file_ir_identity,
            executable_ref: &entrypoint.executable_ref,
            default_run: entrypoint.default_run,
            config_and_effect_metadata: &entrypoint.config_and_effect_metadata,
            runtime_expected_error: entrypoint.runtime_expected_error.as_ref(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointLocalIdPayload<'a> {
    schema: &'static str,
    package_id: &'a str,
    package_version: &'a str,
    source_path: &'a str,
    test_ordinal: u32,
    normalized_test_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageTestEntrypointIdPayload<'a> {
    schema: &'static str,
    test_build_identity: &'a str,
    entrypoint_local_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationAbiIdentityInput<'a> {
    pub kind: PublicationOperationKind,
    pub public_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_instance_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<&'a InterfaceInstantiationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_abi_id: Option<&'a str>,
    pub public_signature: &'a CanonicalPublicCallableSignature,
    pub schema_closure: &'a [PublicationSchemaType],
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub stream_effect_throw_config: &'a BTreeMap<String, MetadataValue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicationAbiIdentityProjection {
    operation_exports: Vec<OperationAbiIdentityProjection>,
    operation_abi: Vec<PublicationOperationAbiIdentityProjection>,
    source_call_operation_index: Vec<SourceCallOperationIndexIdentityProjection>,
    public_instance_exports: Vec<PublicInstanceAbiIdentityProjection>,
    schema_closure: Vec<PublicationSchemaType>,
    public_conformance_facts: Vec<PublicationConformanceFact>,
    public_contract_effect_config: BTreeMap<String, MetadataValue>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationAbiIdentityProjection {
    operation_abi_id: String,
    kind: skiff_artifact_model::PublicationOperationKind,
    public_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_instance_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<InterfaceInstantiationRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method_abi_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicationOperationAbiIdentityProjection {
    operation: OperationAbiIdentityProjection,
    public_signature: skiff_artifact_model::CanonicalPublicCallableSignature,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    schema_closure: Vec<PublicationSchemaType>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    stream_effect_throw_config: BTreeMap<String, MetadataValue>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceCallOperationIndexIdentityProjection {
    source_call_path: String,
    operation_abi_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicInstanceAbiIdentityProjection {
    public_instance_key: String,
    interfaces: Vec<InterfaceInstantiationRef>,
    source_call_method_index: Vec<SourceCallMethodIndexIdentityProjection>,
    method_operations: Vec<OperationAbiIdentityProjection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceCallMethodIndexIdentityProjection {
    method_name: String,
    operation_abi_id: String,
}

pub fn publication_abi_identity_bytes(unit: &PublicationAbiUnit) -> Result<Vec<u8>> {
    canonical_ir_bytes(
        &publication_abi_identity_projection(unit),
        ArtifactIdentityError::SerializePublicationAbiIdentity,
    )
}

fn publication_abi_identity_projection(
    unit: &PublicationAbiUnit,
) -> PublicationAbiIdentityProjection {
    let mut operation_exports = unit
        .operation_exports
        .iter()
        .map(operation_identity_projection)
        .collect::<Vec<_>>();
    operation_exports.sort_by(|left, right| left.operation_abi_id.cmp(&right.operation_abi_id));

    let mut operation_abi = unit
        .operation_abi
        .iter()
        .map(publication_operation_abi_identity_projection)
        .collect::<Vec<_>>();
    operation_abi.sort_by(|left, right| {
        left.operation
            .operation_abi_id
            .cmp(&right.operation.operation_abi_id)
    });

    let mut source_call_operation_index = unit
        .source_call_operation_index
        .iter()
        .map(|entry| SourceCallOperationIndexIdentityProjection {
            source_call_path: entry.source_call_path.clone(),
            operation_abi_id: entry.operation.operation_abi_id.clone(),
        })
        .collect::<Vec<_>>();
    source_call_operation_index.sort_by(|left, right| {
        left.source_call_path
            .cmp(&right.source_call_path)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    let mut public_instance_exports = unit
        .public_instances
        .iter()
        .map(public_instance_identity_projection)
        .collect::<Vec<_>>();
    public_instance_exports
        .sort_by(|left, right| left.public_instance_key.cmp(&right.public_instance_key));

    let mut schema_closure = unit.schema_closure.clone();
    schema_closure.sort_by(|left, right| {
        left.abi_type_id
            .cmp(&right.abi_type_id)
            .then(schema_type_sort_key(left).cmp(&schema_type_sort_key(right)))
    });

    let mut public_conformance_facts = unit.public_conformance_facts.clone();
    public_conformance_facts.sort_by(|left, right| {
        left.type_abi_id
            .cmp(&right.type_abi_id)
            .then(interface_sort_key(&left.interface).cmp(&interface_sort_key(&right.interface)))
    });

    PublicationAbiIdentityProjection {
        operation_exports,
        operation_abi,
        source_call_operation_index,
        public_instance_exports,
        schema_closure,
        public_conformance_facts,
        public_contract_effect_config: unit.public_contract_effect_config.clone(),
    }
}

fn operation_identity_projection(operation: &OperationAbiRef) -> OperationAbiIdentityProjection {
    OperationAbiIdentityProjection {
        operation_abi_id: operation.operation_abi_id.clone(),
        kind: operation.kind,
        public_path: operation.public_path.clone(),
        public_instance_key: operation.public_instance_key.clone(),
        interface: operation.interface.clone(),
        method_abi_id: operation.method_abi_id.clone(),
    }
}

fn publication_operation_abi_identity_projection(
    operation: &PublicationOperationAbi,
) -> PublicationOperationAbiIdentityProjection {
    let mut schema_closure = operation.schema_closure.clone();
    schema_closure.sort_by(|left, right| left.abi_type_id.cmp(&right.abi_type_id));
    PublicationOperationAbiIdentityProjection {
        operation: operation_identity_projection(&operation.operation),
        public_signature: operation.public_signature.clone(),
        schema_closure,
        stream_effect_throw_config: operation.stream_effect_throw_config.clone(),
    }
}

fn public_instance_identity_projection(
    public_instance: &PublicationPublicInstanceExport,
) -> PublicInstanceAbiIdentityProjection {
    let mut interfaces = public_instance.interfaces.clone();
    interfaces.sort_by_key(interface_sort_key);

    let mut source_call_method_index = public_instance
        .source_call_method_index
        .iter()
        .map(|entry| SourceCallMethodIndexIdentityProjection {
            method_name: entry.method_name.clone(),
            operation_abi_id: entry.operation.operation_abi_id.clone(),
        })
        .collect::<Vec<_>>();
    source_call_method_index.sort_by(|left, right| {
        left.method_name
            .cmp(&right.method_name)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    let mut method_operations = public_instance
        .method_operations
        .iter()
        .map(operation_identity_projection)
        .collect::<Vec<_>>();
    method_operations.sort_by(|left, right| {
        left.method_abi_id
            .cmp(&right.method_abi_id)
            .then(left.operation_abi_id.cmp(&right.operation_abi_id))
    });

    PublicInstanceAbiIdentityProjection {
        public_instance_key: public_instance.public_instance_key.clone(),
        interfaces,
        source_call_method_index,
        method_operations,
    }
}

fn interface_sort_key(interface: &InterfaceInstantiationRef) -> Vec<u8> {
    canonical_ir_bytes(
        interface,
        ArtifactIdentityError::SerializePackageAbiIdentity,
    )
    .expect("interface instantiation must serialize for ABI identity sorting")
}

fn schema_type_sort_key(schema_type: &PublicationSchemaType) -> Vec<u8> {
    canonical_ir_bytes(
        schema_type,
        ArtifactIdentityError::SerializePackageAbiIdentity,
    )
    .expect("schema type must serialize for ABI identity sorting")
}

pub fn runtime_program_service_unit_identity_value(unit: &ServiceUnit) -> Result<Value> {
    let payload = RuntimeProgramServiceUnitIdentityPayload::from_service_unit(unit)?;
    let value = serde_json::to_value(payload)
        .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?;
    Ok(canonical_json_value(&value))
}

pub fn runtime_program_service_unit_identity_value_from_json(
    service_unit: &Value,
) -> Result<Value> {
    let unit: ServiceUnit = serde_json::from_value(service_unit.clone())
        .map_err(ArtifactIdentityError::InvalidServiceUnit)?;
    runtime_program_service_unit_identity_value(&unit)
}

pub fn runtime_program_service_unit_identity_bytes(unit: &ServiceUnit) -> Result<Vec<u8>> {
    let identity = runtime_program_service_unit_identity_value(unit)?;
    serialize_runtime_program_service_unit_identity_bytes(&identity)
}

pub fn runtime_program_service_unit_identity_bytes_from_json(
    service_unit: &Value,
) -> Result<Vec<u8>> {
    let identity = runtime_program_service_unit_identity_value_from_json(service_unit)?;
    serialize_runtime_program_service_unit_identity_bytes(&identity)
}

pub fn runtime_program_dynamic_build_id<'a>(
    service_unit_identity_bytes: &[u8],
    package_build_identities: impl IntoIterator<Item = &'a str>,
) -> String {
    let package_build_identities = package_build_identities.into_iter().collect::<Vec<_>>();
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, "schema", RUNTIME_PROGRAM_BUILD_SCHEMA_MARKER);
    hash_bytes(
        &mut hasher,
        "serviceUnitIdentity",
        service_unit_identity_bytes,
    );
    hash_field(
        &mut hasher,
        "packageCount",
        &package_build_identities.len().to_string(),
    );
    for build_identity in package_build_identities {
        hash_field(&mut hasher, "packageBuildIdentity", build_identity);
    }
    format!(
        "{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{}",
        hex::encode(hasher.finalize())
    )
}

pub fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonical_json_value).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            let mut sorted = Map::new();
            for key in keys {
                if let Some(nested) = object.get(key) {
                    sorted.insert(key.clone(), canonical_json_value(nested));
                }
            }
            Value::Object(sorted)
        }
        Value::Number(number) => canonical_json_number(number),
        _ => value.clone(),
    }
}

pub fn canonical_json_number(number: &Number) -> Value {
    if let Some(value) = number.as_i64() {
        return Value::Number(Number::from(value));
    }
    if let Some(value) = number.as_u64() {
        return Value::Number(Number::from(value));
    }
    if let Some(value) = number.as_f64() {
        if value.is_finite()
            && value.fract() == 0.0
            && value >= i64::MIN as f64
            && value <= i64::MAX as f64
        {
            return Value::Number(Number::from(value as i64));
        }
    }
    Value::Number(number.clone())
}

fn canonical_ir_bytes<T: Serialize>(
    value: &T,
    map_error: fn(serde_json::Error) -> ArtifactIdentityError,
) -> Result<Vec<u8>> {
    let value = serde_json::to_value(value).map_err(map_error)?;
    let canonical = canonical_json_value(&value);
    serde_json::to_vec(&canonical).map_err(map_error)
}

fn serialize_runtime_program_service_unit_identity_bytes(identity: &Value) -> Result<Vec<u8>> {
    let canonical = canonical_json_value(identity);
    serde_json::to_vec(&canonical)
        .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn identity(prefix: &str, hash: &str) -> String {
    format!("{prefix}:{hash}")
}

fn publication_abi_identity_value(unit: &PublicationAbiUnit) -> Result<Value> {
    let projection = publication_abi_identity_projection(unit);
    let value = serde_json::to_value(projection)
        .map_err(ArtifactIdentityError::SerializePublicationAbiIdentity)?;
    Ok(canonical_json_value(&value))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProgramServiceUnitIdentityPayload {
    schema_version: String,
    service: ServiceMetaIdentityPayload,
    version: String,
    protocol_identity: String,
    publication_abi: Value,
    files: Vec<FileIrRefIdentityPayload>,
    package_dependencies: Vec<PackageDependencyIdentityPayload>,
    service_dependencies: Vec<ServiceDependencyIdentityPayload>,
    package_abi_expectations: Vec<PackageAbiExpectationIdentityPayload>,
    operations: Vec<ServiceOperationIdentityPayload>,
    public_instances: Value,
    db: Value,
    processes: Value,
    spawn_targets: Value,
    actors: Value,
    gateway: Value,
    timeout: Value,
    config: Value,
}

impl RuntimeProgramServiceUnitIdentityPayload {
    fn from_service_unit(unit: &ServiceUnit) -> Result<Self> {
        Ok(Self {
            schema_version: unit.schema_version.clone(),
            service: ServiceMetaIdentityPayload::from_service_meta(&unit.service)?,
            version: unit.version.clone(),
            protocol_identity: unit.protocol_identity.clone(),
            publication_abi: publication_abi_identity_value(&unit.publication_abi)?,
            files: unit
                .files
                .iter()
                .map(FileIrRefIdentityPayload::from_ref)
                .collect(),
            package_dependencies: unit
                .package_dependencies
                .iter()
                .map(PackageDependencyIdentityPayload::from_constraint)
                .collect(),
            service_dependencies: unit
                .service_dependencies
                .iter()
                .map(ServiceDependencyIdentityPayload::from_constraint)
                .collect::<Result<Vec<_>>>()?,
            package_abi_expectations: unit
                .package_abi_expectations
                .iter()
                .map(PackageAbiExpectationIdentityPayload::from_expectation)
                .collect::<Result<Vec<_>>>()?,
            operations: unit
                .operations
                .iter()
                .map(ServiceOperationIdentityPayload::from_operation)
                .collect::<Result<Vec<_>>>()?,
            public_instances: non_empty_array_or_null(&unit.public_instances)?,
            db: non_empty_array_or_null(&unit.db)?,
            processes: Value::Null,
            spawn_targets: array_value(&unit.spawn_targets)?,
            actors: array_value(&unit.actors)?,
            gateway: serde_json::to_value(&unit.gateway)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
            timeout: service_timeout_identity_value(&unit.timeout)?,
            config: serde_json::to_value(&unit.config)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceMetaIdentityPayload {
    id: String,
    display_name: Value,
    revision_id: Value,
    metadata: Value,
}

impl ServiceMetaIdentityPayload {
    fn from_service_meta(service: &ServiceMeta) -> Result<Self> {
        Ok(Self {
            id: service.id.clone(),
            display_name: service
                .display_name
                .as_ref()
                .map(|value| Value::String(value.clone()))
                .unwrap_or(Value::Null),
            revision_id: Value::Null,
            metadata: serde_json::to_value(&service.metadata)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileIrRefIdentityPayload {
    file_ir_identity: String,
    module_path: String,
    artifact_path: Value,
    source_ast_hash: Value,
}

impl FileIrRefIdentityPayload {
    fn from_ref(file: &FileIrRef) -> Self {
        Self {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: option_string_value(&file.artifact_path),
            source_ast_hash: option_string_value(&file.source_ast_hash),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageDependencyIdentityPayload {
    id: String,
    version: String,
    alias: String,
    config: Value,
}

impl PackageDependencyIdentityPayload {
    fn from_constraint(dependency: &PackageDependencyConstraint) -> Self {
        Self {
            id: dependency.id.clone(),
            version: dependency.version.clone(),
            alias: dependency.alias.clone(),
            config: dependency.config.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDependencyIdentityPayload {
    id: String,
    version: String,
    alias: String,
    build_id: String,
    service_protocol_identity: String,
    publication_abi: Value,
}

impl ServiceDependencyIdentityPayload {
    fn from_constraint(dependency: &ServiceDependencyConstraint) -> Result<Self> {
        Ok(Self {
            id: dependency.id.clone(),
            version: dependency.version.clone(),
            alias: dependency.alias.clone(),
            build_id: dependency.build_id.clone(),
            service_protocol_identity: dependency.service_protocol_identity.clone(),
            publication_abi: publication_abi_identity_value(&dependency.publication_abi)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageAbiExpectationIdentityPayload {
    id: String,
    version: String,
    abi_identity: String,
    used_symbols: Vec<PackageUsedSymbolIdentityPayload>,
}

impl PackageAbiExpectationIdentityPayload {
    fn from_expectation(expectation: &skiff_artifact_model::PackageAbiExpectation) -> Result<Self> {
        Ok(Self {
            id: expectation.id.clone(),
            version: expectation.version.clone(),
            abi_identity: expectation.abi_identity.clone(),
            used_symbols: expectation
                .used_symbols
                .iter()
                .map(PackageUsedSymbolIdentityPayload::from_symbol)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageUsedSymbolIdentityPayload {
    kind: Value,
    symbol_path: String,
}

impl PackageUsedSymbolIdentityPayload {
    fn from_symbol(symbol: &PackageUsedSymbol) -> Result<Self> {
        Ok(Self {
            kind: serde_json::to_value(symbol.kind)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
            symbol_path: symbol.symbol_path.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceOperationIdentityPayload {
    kind: &'static str,
    operation: Value,
    executable: Value,
    receiver_executable: Value,
}

impl ServiceOperationIdentityPayload {
    fn from_operation(operation: &ServiceOperation) -> Result<Self> {
        match operation {
            ServiceOperation::LocalExecutable(target) => Ok(Self {
                kind: "localExecutable",
                operation: serde_json::to_value(&target.operation)
                    .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
                executable: operation_target_ref_identity_value(&target.executable)?,
                receiver_executable: Value::Null,
            }),
            ServiceOperation::LocalReceiverExecutable(target) => Ok(Self {
                kind: "localReceiverExecutable",
                operation: serde_json::to_value(&target.operation)
                    .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
                executable: Value::Null,
                receiver_executable: serde_json::to_value(
                    LocalReceiverExecutableIdentityPayload::from_ref(&target.receiver_executable)?,
                )
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalReceiverExecutableIdentityPayload {
    receiver: OperationConstReceiverIdentityPayload,
    executable_target: Value,
    method_abi_id: String,
    receiver_call_abi: Value,
}

impl LocalReceiverExecutableIdentityPayload {
    fn from_ref(value: &skiff_artifact_model::LocalReceiverExecutableRef) -> Result<Self> {
        Ok(Self {
            receiver: OperationConstReceiverIdentityPayload::from_ref(&value.receiver)?,
            executable_target: operation_target_ref_identity_value(&value.executable_target)?,
            method_abi_id: value.method_abi_id.clone(),
            receiver_call_abi: serde_json::to_value(value.receiver_call_abi)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationConstReceiverIdentityPayload {
    file_ref: FileRefIdentityPayload,
    const_index: u32,
    const_abi_id: String,
    const_type_abi_id: String,
}

impl OperationConstReceiverIdentityPayload {
    fn from_ref(value: &skiff_artifact_model::OperationConstReceiverRef) -> Result<Self> {
        Ok(Self {
            file_ref: FileRefIdentityPayload::from_ref(&value.file_ref),
            const_index: value.const_index,
            const_abi_id: value.const_abi_id.clone(),
            const_type_abi_id: value.const_type_abi_id.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationTargetRefIdentityPayload {
    file_ref: FileRefIdentityPayload,
    executable_index: u32,
    callable_abi_id: String,
    callable_kind: Value,
}

fn operation_target_ref_identity_value(target: &OperationTargetRef) -> Result<Value> {
    serde_json::to_value(OperationTargetRefIdentityPayload {
        file_ref: FileRefIdentityPayload::from_ref(&target.file_ref),
        executable_index: target.executable_index,
        callable_abi_id: target.callable_abi_id.clone(),
        callable_kind: serde_json::to_value(target.callable_kind)
            .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
    })
    .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileRefIdentityPayload {
    file_ir_identity: String,
    module_path: String,
}

impl FileRefIdentityPayload {
    fn from_ref(file: &FileIrRef) -> Self {
        Self {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
        }
    }
}

fn option_string_value(value: &Option<String>) -> Value {
    value
        .as_ref()
        .map(|value| Value::String(value.clone()))
        .unwrap_or(Value::Null)
}

fn array_value<T: Serialize>(value: &[T]) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)
}

fn non_empty_array_or_null<T: Serialize>(value: &[T]) -> Result<Value> {
    if value.is_empty() {
        return Ok(Value::Null);
    }
    array_value(value)
}

fn service_timeout_identity_value(timeout: &ServiceTimeoutConfig) -> Result<Value> {
    if timeout.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::to_value(timeout)
        .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)
}

fn hash_field(hasher: &mut Sha256, label: &str, value: &str) {
    hash_framed_bytes(hasher, label, value.as_bytes());
}

fn hash_bytes(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hash_framed_bytes(hasher, label, value);
}

fn hash_framed_bytes(hasher: &mut Sha256, label: &str, value: &[u8]) {
    hasher.update(label.as_bytes());
    hasher.update([0]);
    hasher.update(value.len().to_le_bytes());
    hasher.update(value);
    hasher.update([0xff]);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_artifact_model::{
        CanonicalPublicCallableSignature, ConfigAndEffectMetadata, FileIrUnit, MetadataValue,
        OperationAbiRef, PackageDependencyPublicLinkScope, PackageProductionLinkScope,
        PackageTestAssembly, PackageTestAssemblyKind, PackageTestEntrypoint,
        PackageTestEntrypointKind, PackageTestExecutableRef, PackageTestFileIrRef,
        PackageTestFileLinkScope, PackageTestLinkPolicy, PackageTestPackageUnitRef,
        PackageTestRuntimeExpectedError, PackageUnit, PublicationOperationAbi,
        PublicationOperationKind, SourceCallOperationIndexEntry, SourceMapSource, TypeRefIr,
    };

    use super::*;

    #[test]
    fn runtime_program_identity_value_keeps_legacy_projection_defaults() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let publication_abi_value =
            serde_json::to_value(&publication_abi).expect("publication ABI JSON");
        let service = json!({
            "schemaVersion": "skiff-service-unit-v1",
            "service": {
                "id": "example.com/svc",
                "displayName": "Typed Name"
            },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "publicationAbi": publication_abi_value,
            "files": [],
            "gateway": {},
            "config": {}
        });

        let identity =
            runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
        assert_eq!(identity["service"]["displayName"], "Typed Name");
        assert_eq!(identity["service"]["revisionId"], Value::Null);
        assert_eq!(identity["service"]["metadata"], json!({}));
        assert_eq!(identity["operations"], json!([]));
        assert_eq!(identity["packageDependencies"], json!([]));
        assert_eq!(identity["publicInstances"], Value::Null);
        assert_eq!(identity["bindingResolutions"], Value::Null);
        assert_eq!(identity["db"], Value::Null);
        assert_eq!(identity["processes"], Value::Null);
        assert_eq!(identity["spawnTargets"], json!([]));
        assert_eq!(identity["actors"], json!([]));
        assert_eq!(identity["timeout"], Value::Null);
    }

    #[test]
    fn runtime_program_timeout_participates_in_dynamic_build_id() {
        let mut base = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
        base.publication_abi.abi_identity =
            publication_abi_identity(&base.publication_abi).expect("publication ABI identity");
        let base_identity =
            runtime_program_service_unit_identity_value(&base).expect("base identity");
        assert_eq!(base_identity["timeout"], Value::Null);
        let base_bytes =
            runtime_program_service_unit_identity_bytes(&base).expect("base identity bytes");
        let base_build_id = runtime_program_dynamic_build_id(&base_bytes, []);

        let mut with_timeout = base.clone();
        with_timeout.timeout.default_ms = Some(5_000);
        with_timeout
            .timeout
            .methods
            .insert("run".to_string(), 1_500);
        let timeout_identity =
            runtime_program_service_unit_identity_value(&with_timeout).expect("timeout identity");
        assert_eq!(
            timeout_identity["timeout"],
            json!({
                "defaultMs": 5000,
                "methods": {
                    "run": 1500
                }
            })
        );
        let timeout_bytes = runtime_program_service_unit_identity_bytes(&with_timeout)
            .expect("timeout identity bytes");
        let timeout_build_id = runtime_program_dynamic_build_id(&timeout_bytes, []);

        assert_ne!(base_identity, timeout_identity);
        assert_ne!(base_build_id, timeout_build_id);
    }

    #[test]
    fn runtime_program_identity_rejects_snake_case_and_missing_dependency_fields() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let publication_abi_value =
            serde_json::to_value(&publication_abi).expect("publication ABI JSON");
        let service = json!({
            "schema_version": "skiff-service-unit-v1",
            "service": { "id": "example.com/svc" },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "publicationAbi": publication_abi_value,
            "files": [],
            "gateway": {},
            "config": {}
        });

        let error = runtime_program_service_unit_identity_value_from_json(&service)
            .expect_err("snake_case must fail closed")
            .to_string();
        assert!(
            error.contains("schema_version"),
            "unexpected snake_case error: {error}"
        );

        let mut service = service;
        service["schemaVersion"] = json!("skiff-service-unit-v1");
        service
            .as_object_mut()
            .expect("object")
            .remove("schema_version");
        service["packageDependencies"] = json!([{
            "id": "example.com/pkg",
            "version": "1.0.0"
        }]);
        let error = runtime_program_service_unit_identity_value_from_json(&service)
            .expect_err("missing package dependency alias must fail closed")
            .to_string();
        assert!(
            error.contains("alias"),
            "unexpected missing alias error: {error}"
        );
    }

    #[test]
    fn runtime_program_service_dependency_identity_uses_publication_abi_projection() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let publication_abi_value =
            serde_json::to_value(&publication_abi).expect("publication ABI JSON");
        let service = json!({
            "schemaVersion": "skiff-service-unit-v1",
            "service": { "id": "example.com/svc" },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "publicationAbi": publication_abi_value.clone(),
            "files": [],
            "serviceDependencies": [{
                "id": "example.com/upstream",
                "version": "1.0.0",
                "alias": "upstream",
                "buildId": "build:upstream",
                "serviceProtocolIdentity": "protocol:upstream",
                "publicationAbi": publication_abi_value
            }],
            "operations": [],
            "gateway": {},
            "config": {}
        });

        let identity =
            runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
        let dependency_publication_abi = &identity["serviceDependencies"][0]["publicationAbi"];
        assert_eq!(
            dependency_publication_abi["operationExports"][0]["operationAbiId"],
            "operation:run:string"
        );
        assert!(dependency_publication_abi
            .pointer("/operationExports/0/displayName")
            .is_none());
        assert!(dependency_publication_abi
            .pointer("/operationAbi/0/operation/displayName")
            .is_none());

        let mut renamed_display = service.clone();
        renamed_display["serviceDependencies"][0]["publicationAbi"]["operationAbi"][0]
            ["operation"]["displayName"] = json!("renamed");
        let renamed_identity =
            runtime_program_service_unit_identity_value_from_json(&renamed_display)
                .expect("renamed identity");
        assert_eq!(identity, renamed_identity);

        let mut signature_changed = service;
        signature_changed["serviceDependencies"][0]["publicationAbi"]["operationAbi"][0]
            ["publicSignature"]["returnType"]["name"] = json!("number");
        let signature_identity =
            runtime_program_service_unit_identity_value_from_json(&signature_changed)
                .expect("signature identity");
        assert_ne!(identity, signature_identity);
    }

    #[test]
    fn runtime_program_top_level_publication_abi_identity_uses_publication_abi_projection() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let mut publication_abi_value =
            serde_json::to_value(&publication_abi).expect("publication ABI JSON");
        publication_abi_value["operationAbi"][0]["publicSignature"]["params"] = json!([]);
        let service = json!({
            "schemaVersion": "skiff-service-unit-v1",
            "service": { "id": "example.com/svc" },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "publicationAbi": publication_abi_value,
            "files": [],
            "operations": [],
            "gateway": {},
            "config": {}
        });

        let identity =
            runtime_program_service_unit_identity_value_from_json(&service).expect("identity");
        let service_publication_abi = &identity["publicationAbi"];
        assert_eq!(
            service_publication_abi["operationExports"][0]["operationAbiId"],
            "operation:run:string"
        );
        assert!(service_publication_abi
            .pointer("/operationExports/0/displayName")
            .is_none());
        assert!(service_publication_abi
            .pointer("/operationAbi/0/operation/displayName")
            .is_none());
        assert!(service_publication_abi
            .pointer("/operationAbi/0/publicSignature/params")
            .is_none());

        let mut renamed_display = service.clone();
        renamed_display["publicationAbi"]["operationAbi"][0]["operation"]["displayName"] =
            json!("renamed");
        let renamed_identity =
            runtime_program_service_unit_identity_value_from_json(&renamed_display)
                .expect("renamed identity");
        assert_eq!(identity, renamed_identity);

        let mut signature_changed = service;
        signature_changed["publicationAbi"]["operationAbi"][0]["publicSignature"]["returnType"]
            ["name"] = json!("number");
        let signature_identity =
            runtime_program_service_unit_identity_value_from_json(&signature_changed)
                .expect("signature identity");
        assert_ne!(identity, signature_identity);
    }

    #[test]
    fn runtime_program_top_level_publication_abi_is_required() {
        let service = json!({
            "schemaVersion": "skiff-service-unit-v1",
            "service": { "id": "example.com/svc" },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "files": [],
            "operations": [],
            "gateway": {},
            "config": {}
        });

        let error = runtime_program_service_unit_identity_value_from_json(&service)
            .expect_err("missing top-level publicationAbi must fail closed")
            .to_string();
        assert!(
            error.contains("publicationAbi"),
            "unexpected missing publicationAbi error: {error}"
        );

        let mut null_publication_abi = service;
        null_publication_abi["publicationAbi"] = Value::Null;
        let error = runtime_program_service_unit_identity_value_from_json(&null_publication_abi)
            .expect_err("null top-level publicationAbi must fail closed")
            .to_string();
        assert!(
            error.contains("PublicationAbiUnit"),
            "unexpected null publicationAbi error: {error}"
        );
    }

    #[test]
    fn runtime_program_service_dependency_identity_requires_publication_abi() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let publication_abi_value =
            serde_json::to_value(&publication_abi).expect("publication ABI JSON");
        let service = json!({
            "schemaVersion": "skiff-service-unit-v1",
            "service": { "id": "example.com/svc" },
            "version": "1.0.0",
            "protocolIdentity": "protocol",
            "publicationAbi": publication_abi_value,
            "files": [],
            "serviceDependencies": [{
                "id": "example.com/upstream",
                "version": "1.0.0",
                "alias": "upstream",
                "buildId": "build:upstream",
                "serviceProtocolIdentity": "protocol:upstream"
            }],
            "operations": [],
            "gateway": {},
            "config": {}
        });

        let error = runtime_program_service_unit_identity_value_from_json(&service)
            .expect_err("missing dependency publicationAbi must fail closed")
            .to_string();
        assert!(
            error.contains("publicationAbi"),
            "unexpected missing publicationAbi error: {error}"
        );
    }

    #[test]
    fn file_ir_identity_omits_storage_identity_and_source_hashes() {
        let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
        unit.file_ir_identity = "stale-file-ir-identity".to_string();
        unit.source_map.sources.push(SourceMapSource {
            id: 0,
            path: "internal/example.skiff".to_string(),
            module_path: "internal.example".to_string(),
            source_ast_hash: Some("source-map-ast-hash-a".to_string()),
        });

        let value = canonical_file_ir_identity_value(&unit).expect("identity value");

        assert!(value.get("fileIrIdentity").is_none());
        assert!(value.get("sourceAstHash").is_none());
        assert!(value
            .pointer("/sourceMap/sources/0/sourceAstHash")
            .is_none());
        assert_eq!(value["modulePath"], "internal.example");
        assert_eq!(
            value.pointer("/sourceMap/sources/0/path"),
            Some(&json!("internal/example.skiff"))
        );
    }

    #[test]
    fn file_ir_identity_validation_rejects_stale_identity() {
        let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
        unit.file_ir_identity = "stale-file-ir-identity".to_string();

        let error = validate_file_ir_identity(&unit).expect_err("stale identity must fail");

        assert!(matches!(
            error,
            ArtifactIdentityError::FileIrIdentityMismatch { .. }
        ));
        let computed = file_ir_identity(&unit).expect("computed identity");
        unit.file_ir_identity = computed;
        validate_file_ir_identity(&unit).expect("computed identity should validate");
    }

    #[test]
    fn service_unit_storage_identity_wraps_canonical_service_unit() {
        let mut publication_abi = publication_abi_fixture();
        publication_abi.abi_identity =
            publication_abi_identity(&publication_abi).expect("publication ABI identity");
        let mut unit = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
        unit.publication_abi = publication_abi;
        unit.files.push(FileIrRef {
            file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            module_path: "svc.main".to_string(),
            artifact_path: Some("units/files/svc.json".to_string()),
            source_ast_hash: Some("source".to_string()),
        });

        let value = service_unit_identity_value(&unit).expect("service unit identity value");
        assert_eq!(
            value.pointer("/identitySchema"),
            Some(&json!("skiff-service-unit-identity-v1"))
        );
        assert_eq!(
            value.pointer("/unit/service/id"),
            Some(&json!("example.com/svc"))
        );
        assert_eq!(
            value.pointer("/unit/files/0/fileIrIdentity"),
            Some(&json!("skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))
        );

        let hash = service_unit_hash(&unit).expect("service unit hash");
        let identity = service_unit_identity(&unit).expect("service unit identity");
        assert_eq!(identity, format!("{SERVICE_UNIT_IDENTITY_PREFIX}:{hash}"));
        assert_eq!(
            service_unit_identity_bytes(&unit).expect("service unit identity bytes"),
            serde_json::to_vec(&value).expect("service unit identity value bytes")
        );

        let mut changed = unit;
        changed.protocol_identity = "protocol:changed".to_string();
        assert_ne!(
            identity,
            service_unit_identity(&changed).expect("changed service unit identity")
        );
    }

    #[test]
    fn operation_abi_helpers_share_the_canonical_operation_input() {
        let public_signature = CanonicalPublicCallableSignature {
            params: Vec::new(),
            return_type: TypeRefIr::native("string"),
            may_suspend: false,
        };
        let stream_effect_throw_config = BTreeMap::new();
        let input = OperationAbiIdentityInput {
            kind: PublicationOperationKind::PublicFunction,
            public_path: "run",
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            public_signature: &public_signature,
            schema_closure: &[],
            stream_effect_throw_config: &stream_effect_throw_config,
        };

        let identity = operation_abi_identity(&input).expect("operation ABI identity");
        assert!(identity.starts_with(OPERATION_ABI_IDENTITY_PREFIX));
        assert_eq!(
            identity,
            public_function_operation_abi_id("run", &public_signature, &[], &BTreeMap::new())
                .expect("public function ABI id")
        );

        let changed_signature = CanonicalPublicCallableSignature {
            params: Vec::new(),
            return_type: TypeRefIr::native("number"),
            may_suspend: false,
        };
        assert_ne!(
            identity,
            public_function_operation_abi_id("run", &changed_signature, &[], &BTreeMap::new())
                .expect("changed public function ABI id")
        );
    }

    #[test]
    fn package_body_change_changes_build_identity_not_abi_identity() {
        let base = package_fixture("hello");
        let body_changed = package_fixture("changed");

        assert_ne!(
            package_build_identity(&base).expect("base build identity"),
            package_build_identity(&body_changed).expect("changed build identity")
        );
        assert_eq!(
            package_abi_identity(&base).expect("base abi identity"),
            package_abi_identity(&body_changed).expect("changed abi identity")
        );
    }

    #[test]
    fn package_test_build_identity_excludes_persisted_entrypoint_id() {
        let mut assembly = package_test_assembly_fixture();
        let original_identity =
            package_test_build_identity(&assembly).expect("package test build identity");
        let original_projection = canonical_package_test_build_identity_value(&assembly)
            .expect("package test build identity projection");

        assert!(original_projection.get("testBuildIdentity").is_none());
        assert!(original_projection.get("sourceMap").is_none());
        assert!(original_projection
            .pointer("/testEntrypoints/0/entrypointId")
            .is_none());

        assembly.test_entrypoints[0].entrypoint_id =
            "skiff-package-test-entrypoint-v1:sha256:tampered".to_string();

        assert_eq!(
            original_identity,
            package_test_build_identity(&assembly)
                .expect("entrypoint id should not affect package test build identity")
        );
        assert_eq!(
            original_projection,
            canonical_package_test_build_identity_value(&assembly)
                .expect("entrypoint id should not affect package test build projection")
        );
    }

    #[test]
    fn package_test_build_identity_includes_entrypoint_config_and_effect_metadata() {
        let mut assembly = package_test_assembly_fixture();
        let original_identity =
            package_test_build_identity(&assembly).expect("package test build identity");

        assembly.test_entrypoints[0]
            .config_and_effect_metadata
            .config
            .insert("first.secret".to_string(), MetadataValue::Bool(true));

        assert_ne!(
            original_identity,
            package_test_build_identity(&assembly)
                .expect("entrypoint metadata should affect package test build identity")
        );
    }

    #[test]
    fn package_test_entrypoint_id_derivation_uses_build_identity_and_local_id() {
        let local_id = package_test_entrypoint_local_id(
            "example.com/pkg",
            "1.0.0",
            "tests/pkg.test.skiff",
            0,
            "runs internal helper",
        )
        .expect("entrypoint local id");
        let changed_local_id = package_test_entrypoint_local_id(
            "example.com/pkg",
            "1.0.0",
            "tests/pkg.test.skiff",
            1,
            "runs internal helper",
        )
        .expect("changed entrypoint local id");

        let entrypoint_id = derive_package_test_entrypoint_id(
            "skiff-package-test-build-v1:sha256:aaaaaaaa",
            &local_id,
        )
        .expect("entrypoint id");
        let changed_build_entrypoint_id = derive_package_test_entrypoint_id(
            "skiff-package-test-build-v1:sha256:bbbbbbbb",
            &local_id,
        )
        .expect("changed build entrypoint id");
        let changed_local_entrypoint_id = derive_package_test_entrypoint_id(
            "skiff-package-test-build-v1:sha256:aaaaaaaa",
            &changed_local_id,
        )
        .expect("changed local entrypoint id");

        assert!(local_id.starts_with(PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX));
        assert!(entrypoint_id.starts_with(PACKAGE_TEST_ENTRYPOINT_ID_PREFIX));
        assert_ne!(local_id, changed_local_id);
        assert_ne!(entrypoint_id, changed_build_entrypoint_id);
        assert_ne!(entrypoint_id, changed_local_entrypoint_id);
    }

    #[test]
    fn package_test_identity_validation_recomputes_entrypoint_ids() {
        let mut assembly = package_test_assembly_fixture();
        assembly.test_build_identity =
            package_test_build_identity(&assembly).expect("package test build identity");
        assembly.test_entrypoints[0].entrypoint_id = derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &assembly.test_entrypoints[0].entrypoint_local_id,
        )
        .expect("entrypoint id");

        validate_package_test_assembly_identity(&assembly)
            .expect("matching package test identities should validate");

        assembly.test_entrypoints[0].entrypoint_id =
            "skiff-package-test-entrypoint-v1:sha256:tampered".to_string();
        let error = validate_package_test_assembly_identity(&assembly)
            .expect_err("tampered entrypoint id must fail validation");
        assert!(matches!(
            error,
            ArtifactIdentityError::PackageTestEntrypointIdMismatch { .. }
        ));
    }

    #[test]
    fn package_abi_identity_uses_publication_abi_surface() {
        let mut unit = package_fixture("hello");
        let package_abi = package_abi_identity(&unit).expect("package abi identity");
        let package_hash = package_abi_hash(&unit).expect("package abi hash");
        let publication_hash =
            publication_abi_hash(&unit.publication_abi).expect("publication abi hash");
        let publication_abi =
            publication_abi_identity(&unit.publication_abi).expect("publication abi identity");

        assert_eq!(package_hash, publication_hash);
        assert_ne!(package_abi, publication_abi);
        assert_eq!(publication_abi, unit.publication_abi.abi_identity);
        assert!(package_abi.starts_with(PACKAGE_ABI_IDENTITY_PREFIX));
        assert!(publication_abi.starts_with(PUBLICATION_ABI_IDENTITY_PREFIX));

        let original = package_abi;
        let original_hash = package_hash;
        let original_publication = publication_abi;
        let link = unit
            .implementation_links
            .functions
            .get_mut("run")
            .expect("run implementation link");
        link.executable_index = 42;
        link.signature.return_type = TypeRefIr::native("number");

        assert_eq!(
            original,
            package_abi_identity(&unit).expect("implementation changed package abi identity")
        );
        assert_eq!(
            original_hash,
            package_abi_hash(&unit).expect("implementation changed package abi hash")
        );
        assert_eq!(
            original_publication,
            publication_abi_identity(&unit.publication_abi).expect("publication abi identity")
        );

        unit.publication_abi.operation_abi[0]
            .public_signature
            .return_type = TypeRefIr::native("number");
        let changed_publication_abi = publication_abi_identity(&unit.publication_abi)
            .expect("changed publication abi identity");
        assert_ne!(original_publication, changed_publication_abi);
        assert_ne!(
            original,
            package_abi_identity(&unit).expect("changed publication package abi identity")
        );
        assert_ne!(
            original_hash,
            package_abi_hash(&unit).expect("changed publication package abi hash")
        );
    }

    #[test]
    fn package_identity_validation_rejects_stale_build_or_abi_identity() {
        let mut unit = package_fixture("hello");
        unit.build_identity = package_build_identity(&unit).expect("build identity");
        unit.abi_identity = "stale-abi".to_string();

        let error = validate_package_unit_identities(&unit).expect_err("stale ABI must fail");
        assert!(matches!(
            error,
            ArtifactIdentityError::PackageAbiIdentityMismatch { .. }
        ));

        unit.abi_identity = package_abi_identity(&unit).expect("abi identity");
        validate_package_unit_identities(&unit).expect("computed identities should validate");
        unit.build_identity = "stale-build".to_string();
        let error = validate_package_unit_identities(&unit).expect_err("stale build must fail");
        assert!(matches!(
            error,
            ArtifactIdentityError::PackageBuildIdentityMismatch { .. }
        ));
    }

    #[test]
    fn assign_package_unit_identities_sets_publication_and_package_identities() {
        let mut unit = package_fixture("hello");
        unit.publication_abi.publication_id = "stale-publication".to_string();
        unit.publication_abi.version = "0.0.0".to_string();
        unit.publication_abi.abi_identity = "stale-publication-abi".to_string();
        unit.abi_identity = "stale-package-abi".to_string();
        unit.build_identity = "stale-build".to_string();

        let (build_identity, abi_identity) =
            assign_package_unit_identities(&mut unit).expect("assign package identities");

        assert_eq!(unit.publication_abi.publication_id, unit.package_id);
        assert_eq!(unit.publication_abi.version, unit.version);
        assert_eq!(unit.build_identity, build_identity);
        assert_eq!(unit.abi_identity, abi_identity);
        assert_eq!(
            unit.publication_abi.abi_identity,
            publication_abi_identity(&unit.publication_abi).expect("publication ABI identity")
        );
        validate_package_unit_identities(&unit).expect("assigned package identities validate");
    }

    fn package_test_assembly_fixture() -> PackageTestAssembly {
        let owner_test_file = PackageTestFileIrRef {
            file_ir_identity: "skiff-file-ir-v3:sha256:testfile".to_string(),
            file_ir_path: "units/files/test.json".to_string(),
            source_path: "tests/pkg.test.skiff".to_string(),
            module_path: "pkg.test".to_string(),
        };
        let entrypoint_local_id = package_test_entrypoint_local_id(
            "example.com/pkg",
            "1.0.0",
            "tests/pkg.test.skiff",
            0,
            "runs internal helper",
        )
        .expect("entrypoint local id");

        PackageTestAssembly {
            schema_version: "skiff-package-test-assembly-v1".to_string(),
            kind: PackageTestAssemblyKind::PackageTest,
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            test_build_identity: "skiff-package-test-build-v1:sha256:stale".to_string(),
            production_package_unit: PackageTestPackageUnitRef {
                package_id: "example.com/pkg".to_string(),
                version: "1.0.0".to_string(),
                build_identity: "skiff-package-build-v1:sha256:prod".to_string(),
                unit_path: "units/packages/example.com/pkg/prod.json".to_string(),
                public_abi_identity: "skiff-package-abi-v1:sha256:prodabi".to_string(),
                implementation_links_identity: "sha256:prodlinks".to_string(),
            },
            test_files: vec![owner_test_file.clone()],
            dependency_package_units: vec![PackageTestPackageUnitRef {
                package_id: "example.com/dep".to_string(),
                version: "1.0.0".to_string(),
                build_identity: "skiff-package-build-v1:sha256:dep".to_string(),
                unit_path: "units/packages/example.com/dep/dep.json".to_string(),
                public_abi_identity: "skiff-package-abi-v1:sha256:depabi".to_string(),
                implementation_links_identity: "sha256:deplinks".to_string(),
            }],
            test_entrypoints: vec![PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: entrypoint_local_id.clone(),
                entrypoint_id: "skiff-package-test-entrypoint-v1:sha256:stale".to_string(),
                display_name: "runs internal helper".to_string(),
                source_path: "tests/pkg.test.skiff".to_string(),
                module_path: "pkg.test".to_string(),
                owner_test_file: owner_test_file.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner_test_file.file_ir_identity.clone(),
                    executable_index: 0,
                    executable_local_id: "test-entrypoint-0".to_string(),
                    symbol: Some("__skiff_package_test_0".to_string()),
                },
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                runtime_expected_error: Some(PackageTestRuntimeExpectedError {
                    code: "ProviderUnavailableError".to_string(),
                    message_contains: Some("offline".to_string()),
                }),
            }],
            link_policy: PackageTestLinkPolicy {
                current_package_production: PackageProductionLinkScope {
                    package_id: "example.com/pkg".to_string(),
                    version: "1.0.0".to_string(),
                    build_identity: "skiff-package-build-v1:sha256:prod".to_string(),
                    files_digest: "sha256:prodfiles".to_string(),
                    implementation_links_digest: "sha256:prodlinks".to_string(),
                    allow_private: true,
                },
                test_file_scopes: vec![PackageTestFileLinkScope {
                    owner_test_file_identity: owner_test_file.file_ir_identity.clone(),
                    source_path: owner_test_file.source_path.clone(),
                    module_path: owner_test_file.module_path.clone(),
                    allowed_local_link_digest: "sha256:testlinks".to_string(),
                    entrypoint_local_ids: vec![entrypoint_local_id],
                }],
                dependency_public_scopes: vec![PackageDependencyPublicLinkScope {
                    package_id: "example.com/dep".to_string(),
                    version: "1.0.0".to_string(),
                    build_identity: "skiff-package-build-v1:sha256:dep".to_string(),
                    public_abi_identity: "skiff-package-abi-v1:sha256:depabi".to_string(),
                    public_export_digest: "sha256:depexports".to_string(),
                    implementation_links_digest: "sha256:deplinks".to_string(),
                    allow_private: false,
                }],
            },
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            source_map: json!({ "sources": [] }),
        }
    }

    fn package_fixture(body_seed: &str) -> PackageUnit {
        let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
        unit.config_and_effect_metadata
            .effects
            .entry("bodySeed".to_string())
            .or_default()
            .metadata
            .insert(
                "value".to_string(),
                MetadataValue::String(body_seed.to_string()),
            );
        unit.implementation_links.functions.insert(
            "run".to_string(),
            skiff_artifact_model::ExecutableExport {
                file: FileIrRef {
                    file_ir_identity: "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    module_path: "pkg.main".to_string(),
                    artifact_path: Some("units/files/pkg.json".into()),
                    source_ast_hash: Some("source".into()),
                },
                executable_index: 0,
                symbol: "run".to_string(),
                signature: skiff_artifact_model::ExecutableSignatureIr {
                    params: Vec::new(),
                    return_type: TypeRefIr::native("string"),
                    self_type: None,
                    may_suspend: false,
                },
            },
        );
        unit.publication_abi = publication_abi_fixture();
        unit.publication_abi.abi_identity =
            publication_abi_identity(&unit.publication_abi).expect("publication abi identity");
        unit
    }

    fn publication_abi_fixture() -> PublicationAbiUnit {
        let operation = OperationAbiRef {
            operation_abi_id: "operation:run:string".to_string(),
            kind: PublicationOperationKind::PublicFunction,
            public_path: "run".to_string(),
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            display_name: "run".to_string(),
        };
        let mut unit = PublicationAbiUnit::empty("example.com/pkg", "1.0.0", "");
        unit.operation_exports.push(operation.clone());
        unit.operation_abi.push(PublicationOperationAbi {
            operation: operation.clone(),
            public_signature: CanonicalPublicCallableSignature {
                params: Vec::new(),
                return_type: TypeRefIr::native("string"),
                may_suspend: false,
            },
            schema_closure: Vec::new(),
            stream_effect_throw_config: BTreeMap::new(),
        });
        unit.source_call_operation_index
            .push(SourceCallOperationIndexEntry {
                source_call_path: "run".to_string(),
                operation,
            });
        unit
    }
}
