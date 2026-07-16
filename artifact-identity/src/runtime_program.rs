use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    FileIrRef, OperationTargetRef, PackageDependencyConstraint, PackageUsedSymbol,
    ServiceDependencyConstraint, ServiceMeta, ServiceOperation, ServiceTimeoutConfig, ServiceUnit,
};

use crate::framing::{hash_bytes, hash_field};
use crate::publication::publication_abi_identity_value;
use crate::{
    ArtifactIdentityError, Result, RUNTIME_PROGRAM_BUILD_SCHEMA_MARKER,
    SERVICE_BUILD_IDENTITY_PREFIX,
};
use skiff_canonical_json::canonical_json_value;

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProgramServiceUnitIdentityPayload {
    schema_version: String,
    service: ServiceMetaIdentityPayload,
    version: String,
    protocol_identity: String,
    publication_abi: Value,
    files: Vec<FileIrRefIdentityPayload>,
    resources: Value,
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
            resources: serde_json::to_value(&unit.resources)
                .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)?,
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

fn serialize_runtime_program_service_unit_identity_bytes(identity: &Value) -> Result<Vec<u8>> {
    let canonical = canonical_json_value(identity);
    serde_json::to_vec(&canonical)
        .map_err(ArtifactIdentityError::SerializeRuntimeProgramServiceUnitIdentity)
}
