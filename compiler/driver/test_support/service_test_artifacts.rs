use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Map as JsonMap, Value};
use skiff_compiler_core::artifact::{
    CanonicalPublicCallableSignature, DbMetadataIndexIr, DbMetadataIr, FunctionTypeParamIr,
    OperationAbiRef, OperationCallableKind, OperationIngressKind, OperationRouteBinding,
    PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind, ServiceOperationTarget,
    SourceCallOperationIndexEntry,
};
use skiff_compiler_emission::identity::{
    runtime_program_dynamic_build_id, runtime_program_service_unit_identity_bytes_from_json,
    SERVICE_BUILD_IDENTITY_PREFIX,
};
use thiserror::Error;

use skiff_compiler_core::id::{PublicationId, SKIFF_STD_PUBLICATION_ID};
use skiff_compiler_core::json_utils::value_sha256;

use crate::{
    emission::artifact::{
        PublishedFileIrArtifact, PublishedJsonArtifact, FILE_IR_SCHEMA_VERSION,
        PACKAGE_UNIT_SCHEMA_VERSION, SERVICE_ASSEMBLY_KIND, SERVICE_ASSEMBLY_SCHEMA_VERSION,
        SERVICE_UNIT_SCHEMA_VERSION,
    },
    emission::{
        file_ir_artifacts::published_file_ir_artifact_from_unit, identity::identity,
        service_artifacts::SERVICE_ASSEMBLY_IDENTITY_PREFIX,
    },
    test_support::package_units::{file_ref_for_published, package_unit_path},
};
use skiff_compiler_lowering::file_ir::{
    assign_file_ir_identity, ExecutableBody, ExecutableIr, ExecutableSignatureIr, FileIrRef,
    FileIrUnit, MetadataValue, PackageRefIr, PackageSymbolRef, TypeRefIr,
};
use skiff_compiler_projection::recoverable_boundary::{
    recoverable_metadata_for_service_artifacts, RecoverableInputs,
};
use skiff_compiler_projection::typed_artifacts::{
    assign_package_unit_identities, assign_publication_abi_identity,
    public_function_operation_abi_id, service_unit_hash, service_unit_identity,
    ConfigAndEffectMetadata, ExecutableExport, GatewayConfig, OperationTargetRef,
    PackageDependencyConstraint, PackageExportIndex, PackageImplementationLinks, PackageUnit,
    RecoverableArtifactMetadata, ServiceConfigMetadata, ServiceMeta, ServiceOperation, ServiceUnit,
    TypeExport,
};

const TEST_REVISION_ID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

#[derive(Debug, Clone)]
pub struct TestRuntimeArtifact {
    pub source_path: String,
    pub module_path: String,
    pub role: String,
    pub package_id: Option<String>,
    pub file_ir: FileIrUnit,
}

pub type TestServiceFileIrArtifact = TestRuntimeArtifact;

#[derive(Debug)]
pub struct WriteTestServiceArtifactRootInput {
    pub artifact_root: PathBuf,
    pub service_id: String,
    pub version: String,
    pub artifacts: Vec<TestRuntimeArtifact>,
    pub package_aliases: BTreeMap<String, Vec<String>>,
    pub operation_name: String,
    pub operation_module: String,
    pub target: String,
    pub test_config: Value,
    pub service_db_mongo_url: Option<String>,
}

pub type TestServiceArtifactInput = WriteTestServiceArtifactRootInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenTestServiceArtifactRoot {
    pub artifact_root: PathBuf,
    pub service_id: String,
    pub version: String,
    pub build_id: String,
    pub pointer_build_id: String,
    pub service_protocol_identity: String,
    pub operation_name: String,
    pub operation_abi_id: String,
    pub target: String,
    pub service_assembly_path: String,
    pub service_unit_path: String,
    pub service_db_mongo_url: Option<String>,
}

pub type TestServiceArtifactOutput = WrittenTestServiceArtifactRoot;

#[derive(Debug, Error)]
pub enum TestArtifactError {
    #[error("invalid test artifact input: {message}")]
    InvalidInput { message: String },
    #[error("failed to write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize {path}: {source}")]
    SerializeJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize {path}: {source}")]
    SerializeYaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

pub type TestServiceArtifactError = TestArtifactError;

pub fn write_test_service_artifact_root(
    input: WriteTestServiceArtifactRootInput,
) -> Result<WrittenTestServiceArtifactRoot, TestArtifactError> {
    let service_id = PublicationId::parse(&input.service_id).map_err(|error| {
        TestArtifactError::InvalidInput {
            message: format!("service id {} is invalid: {error}", input.service_id),
        }
    })?;
    if input.version.is_empty() {
        return Err(TestArtifactError::InvalidInput {
            message: "version must not be empty".to_string(),
        });
    }
    let test_config_object =
        input
            .test_config
            .as_object()
            .ok_or_else(|| TestArtifactError::InvalidInput {
                message: "test_config must be a JSON object".to_string(),
            })?;

    let artifacts = input
        .artifacts
        .iter()
        .map(published_test_file_artifact)
        .collect::<Vec<_>>();
    let package_artifacts = package_artifacts(&input, &artifacts)?;
    let service_files = artifacts
        .iter()
        .map(file_ref_for_published)
        .collect::<Vec<_>>();
    let operation = service_operation(&input, &artifacts)?;
    let operation_abi_id = service_operation_ref(&operation).operation_abi_id.clone();
    let publication_abi = test_service_publication_abi(&input, &operation, &artifacts)?;
    let operation_route_bindings = test_service_operation_route_bindings(&publication_abi);
    let config_paths = config_leaf_paths(test_config_object);
    let config_activation = config_activation_value(&config_paths);
    let db = runtime_program_db_metadata(&input.artifacts, &artifacts);
    let recoverable_file_ir_units = artifacts
        .iter()
        .map(|artifact| artifact.unit.clone())
        .collect::<Vec<_>>();
    let recoverable_metadata = recoverable_metadata_for_service_artifacts(
        &input.service_id,
        &recoverable_file_ir_units,
        &db,
        &[],
        RecoverableInputs::default(),
    )
    .map_err(|error| TestArtifactError::InvalidInput {
        message: format!("failed to project recoverable test DB metadata: {error}"),
    })?;
    let mut service_unit = ServiceUnit {
        schema_version: SERVICE_UNIT_SCHEMA_VERSION.to_string(),
        service: ServiceMeta {
            id: input.service_id.clone(),
            display_name: Some("Skiff Test".to_string()),
            metadata: BTreeMap::new(),
        },
        version: input.version.clone(),
        protocol_identity: protocol_identity_for_test(&input, &artifacts),
        abi_identity_projection: Default::default(),
        publication_abi,
        files: service_files,
        package_dependencies: package_artifacts.dependencies.clone(),
        service_dependencies: Vec::new(),
        package_abi_expectations: Vec::new(),
        operations: vec![operation],
        operation_route_bindings,
        public_instances: Vec::new(),
        recoverable_metadata,
        db: db.clone(),
        spawn_targets: Vec::new(),
        actors: Vec::new(),
        gateway: GatewayConfig::default(),
        timeout: Default::default(),
        config: ServiceConfigMetadata::default(),
    };
    attach_service_file_paths(&mut service_unit, &artifacts);
    let service_unit_hash = service_unit_hash(&service_unit);
    let service_unit_identity = service_unit_identity(&service_unit);
    let service_path = service_id.artifact_path();
    let service_unit_path = format!("units/services/{service_path}/{service_unit_hash}.json");
    let service_unit_value =
        serde_json::to_value(&service_unit).expect("ServiceUnit must serialize");
    let build_id = dynamic_build_id(&service_unit_value, &package_artifacts.units)?;
    let service_unit_artifact = PublishedJsonArtifact {
        value: service_unit_value,
        identity: service_unit_identity,
        hash: service_unit_hash,
        path: service_unit_path.clone(),
    };
    let assembly = service_assembly_artifact(
        &input,
        &artifacts,
        &package_artifacts,
        &service_unit_artifact,
        &config_activation,
        &db,
    );
    let pointer_build_id = format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{}", assembly.hash);

    write_artifact_tree(
        &input.artifact_root,
        &artifacts,
        &package_artifacts,
        &service_unit_artifact,
        &assembly,
        &service_id,
        &input.version,
        &build_id,
        &pointer_build_id,
        &config_wrapped_for_router(
            &input.test_config,
            input.service_db_mongo_url.as_deref(),
            &package_artifacts.dependencies,
        ),
    )?;

    Ok(WrittenTestServiceArtifactRoot {
        artifact_root: input.artifact_root,
        service_id: input.service_id,
        version: input.version,
        build_id,
        pointer_build_id,
        service_protocol_identity: service_unit.protocol_identity,
        operation_name: input.operation_name,
        operation_abi_id,
        target: input.target,
        service_assembly_path: assembly.path,
        service_unit_path,
        service_db_mongo_url: input.service_db_mongo_url,
    })
}

struct PackageArtifacts {
    dependencies: Vec<PackageDependencyConstraint>,
    file_units: Vec<PublishedFileIrArtifact>,
    indexes: Vec<PublishedJsonArtifact>,
    units: Vec<PublishedJsonArtifact>,
}

fn published_test_file_artifact(artifact: &TestRuntimeArtifact) -> PublishedFileIrArtifact {
    let mut unit = artifact.file_ir.clone();
    assign_file_ir_identity(&mut unit);
    published_file_ir_artifact_from_unit(
        &unit,
        artifact.source_path.clone(),
        artifact.module_path.clone(),
        artifact.role.clone(),
    )
}

fn package_artifacts(
    input: &WriteTestServiceArtifactRootInput,
    files: &[PublishedFileIrArtifact],
) -> Result<PackageArtifacts, TestArtifactError> {
    let mut package_groups = BTreeMap::<String, Vec<&PublishedFileIrArtifact>>::new();
    for source in &input.artifacts {
        if let Some(package_id) = &source.package_id {
            if package_id == SKIFF_STD_PUBLICATION_ID {
                continue;
            }
            let Some(file) = files.iter().find(|file| {
                file.source_path == source.source_path && file.module_path == source.module_path
            }) else {
                continue;
            };
            if source.role == "package" {
                package_groups
                    .entry(package_id.clone())
                    .or_default()
                    .push(file);
            }
        }
    }

    let mut package_symbols_by_id = HashMap::<String, Vec<String>>::new();
    let mut units = Vec::new();
    let mut indexes = Vec::new();
    let mut package_files = Vec::new();
    for (package_id, package_files_for_id) in package_groups {
        let mut exports = PackageExportIndex::default();
        for file in &package_files_for_id {
            let unit = file_ir_from_published(file)?;
            let file_ref = file_ref_for_published(file);
            for (executable_index, executable) in unit.executables.iter().enumerate() {
                let Some(symbol_path) = package_symbol_for_file_executable(&unit, executable)
                else {
                    continue;
                };
                package_symbols_by_id
                    .entry(package_id.clone())
                    .or_default()
                    .push(symbol_path.clone());
                insert_package_function_export(
                    &mut exports,
                    &package_id,
                    &symbol_path,
                    file_ref.clone(),
                    executable_index,
                    executable,
                );
            }
            for (type_index, ty) in unit.type_table.iter().enumerate() {
                let Some(symbol_path) = package_symbol_for_file_type(&unit, &ty.name) else {
                    continue;
                };
                package_symbols_by_id
                    .entry(package_id.clone())
                    .or_default()
                    .push(symbol_path.clone());
                insert_package_type_export(
                    &mut exports,
                    &package_id,
                    &symbol_path,
                    file_ref.clone(),
                    type_index,
                    ty,
                );
            }
        }

        let config_and_effect_metadata = package_config_and_effect_metadata(&config_leaf_paths(
            input
                .test_config
                .as_object()
                .expect("test_config object was validated"),
        ));
        let (package_unit, unit) = service_test_package_unit_artifact(
            &package_id,
            &package_files_for_id,
            exports,
            config_and_effect_metadata,
        )?;
        let package_path = PublicationId::parse(&package_id)
            .map_err(|error| TestArtifactError::InvalidInput {
                message: format!("package id {package_id} is invalid: {error}"),
            })?
            .artifact_path();
        let index = package_index_with_unit(&package_unit, &unit, &package_path);
        package_files.extend(package_files_for_id.into_iter().cloned());
        units.push(unit);
        indexes.push(index);
    }

    Ok(PackageArtifacts {
        dependencies: package_dependencies(
            &input.artifacts,
            &package_symbols_by_id,
            &input.package_aliases,
        ),
        file_units: package_files,
        indexes,
        units,
    })
}

fn service_test_package_unit_artifact(
    package_id: &str,
    package_files: &[&PublishedFileIrArtifact],
    exports: PackageExportIndex,
    config_and_effect_metadata: ConfigAndEffectMetadata,
) -> Result<(PackageUnit, PublishedJsonArtifact), TestArtifactError> {
    let mut package_unit = PackageUnit {
        schema_version: PACKAGE_UNIT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        version: "test".to_string(),
        build_identity: String::new(),
        abi_identity: String::new(),
        abi_identity_projection: Default::default(),
        publication_abi: PublicationAbiUnit::empty(
            package_id.to_string(),
            "test".to_string(),
            String::new(),
        ),
        files: package_files
            .iter()
            .map(|file| file_ref_for_published(file))
            .collect(),
        implementation_links: PackageImplementationLinks::from_exports(&exports),
        dependencies: Vec::new(),
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        config_and_effect_metadata,
    };
    assign_package_unit_identities(&mut package_unit);
    let value = serde_json::to_value(&package_unit).expect("PackageUnit must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(package_id)
        .map_err(|error| TestArtifactError::InvalidInput {
            message: format!("package id {package_id} is invalid: {error}"),
        })?
        .artifact_path();
    let unit = PublishedJsonArtifact {
        value,
        identity: package_unit.build_identity.clone(),
        hash: hash.clone(),
        path: package_unit_path(&package_path, &hash),
    };
    Ok((package_unit, unit))
}

fn file_ir_from_published(
    artifact: &PublishedFileIrArtifact,
) -> Result<FileIrUnit, TestArtifactError> {
    Ok(artifact.unit.clone())
}

fn file_ref_for_unit(unit: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: unit.file_ir_identity.clone(),
        module_path: unit.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(unit.source_ast_hash.clone()),
    }
}

fn attach_service_file_paths(service_unit: &mut ServiceUnit, files: &[PublishedFileIrArtifact]) {
    let by_identity = files
        .iter()
        .map(|file| (file.identity.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    for file_ref in &mut service_unit.files {
        let Some(file) = by_identity.get(file_ref.file_ir_identity.as_str()) else {
            continue;
        };
        file_ref.artifact_path = Some(file.path.clone());
        file_ref.source_ast_hash = Some(file.unit.source_ast_hash.clone());
    }
}

fn service_operation(
    input: &WriteTestServiceArtifactRootInput,
    files: &[PublishedFileIrArtifact],
) -> Result<ServiceOperation, TestArtifactError> {
    let (file, executable_index, operation_symbol, executable) =
        test_executable(files, &input.operation_name, &input.operation_module)?;
    let public_signature = CanonicalPublicCallableSignature {
        params: executable
            .params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: param.ty.clone(),
            })
            .collect(),
        return_type: executable.return_type.clone(),
        may_suspend: false,
    };
    let operation_ref = OperationAbiRef {
        operation_abi_id: public_function_operation_abi_id(
            &input.operation_name,
            &public_signature,
            &[],
            &BTreeMap::new(),
        ),
        kind: PublicationOperationKind::PublicFunction,
        public_path: input.operation_name.clone(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: input.operation_name.clone(),
    };
    Ok(ServiceOperation::LocalExecutable(ServiceOperationTarget {
        operation: operation_ref,
        executable: OperationTargetRef {
            file_ref: file_ref_for_unit(&file),
            executable_index: executable_index as u32,
            callable_abi_id: format!("callable:{}.{}", file.module_path, operation_symbol),
            callable_kind: OperationCallableKind::PublicFunction,
        },
    }))
}

fn test_service_publication_abi(
    input: &WriteTestServiceArtifactRootInput,
    operation: &ServiceOperation,
    files: &[PublishedFileIrArtifact],
) -> Result<PublicationAbiUnit, TestArtifactError> {
    let operation_ref = service_operation_ref(operation).clone();
    let (_, _, _, executable) =
        test_executable(files, &input.operation_name, &input.operation_module)?;
    let mut publication_abi = PublicationAbiUnit::empty(
        input.service_id.clone(),
        input.version.clone(),
        String::new(),
    );
    publication_abi
        .operation_exports
        .push(operation_ref.clone());
    publication_abi.operation_abi.push(PublicationOperationAbi {
        operation: operation_ref.clone(),
        public_signature: CanonicalPublicCallableSignature {
            params: executable
                .params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: param.ty.clone(),
                })
                .collect(),
            return_type: executable.return_type.clone(),
            may_suspend: false,
        },
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    });
    publication_abi
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path: operation_ref.public_path.clone(),
            operation: operation_ref,
        });
    assign_publication_abi_identity(&mut publication_abi);
    Ok(publication_abi)
}

fn service_operation_ref(operation: &ServiceOperation) -> &OperationAbiRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.operation,
        ServiceOperation::LocalReceiverExecutable(target) => &target.operation,
    }
}

fn test_service_operation_route_bindings(
    publication_abi: &PublicationAbiUnit,
) -> Vec<OperationRouteBinding> {
    publication_abi
        .operation_exports
        .iter()
        .map(|operation| OperationRouteBinding {
            ingress_kind: OperationIngressKind::ServiceCall,
            selector: format!("operation:{}", operation.operation_abi_id),
            operation_abi_id: operation.operation_abi_id.clone(),
        })
        .collect()
}

fn test_executable(
    files: &[PublishedFileIrArtifact],
    operation: &str,
    operation_module: &str,
) -> Result<(FileIrUnit, usize, String, ExecutableIr), TestArtifactError> {
    let file_artifact = files
        .iter()
        .find(|file| file.module_path == operation_module)
        .ok_or_else(|| TestArtifactError::InvalidInput {
            message: format!("typed File IR did not include test module {operation_module}"),
        })?;
    let file = file_ir_from_published(file_artifact)?;
    let expected_symbol = format!("{operation_module}.{operation}");
    let executable_index = file
        .executables
        .iter()
        .position(|executable| {
            executable.symbol == expected_symbol
                || executable.symbol == operation
                || executable.symbol.ends_with(&format!(".{operation}"))
        })
        .ok_or_else(|| TestArtifactError::InvalidInput {
            message: format!(
                "typed File IR module {operation_module} did not include test operation {operation}"
            ),
        })?;
    let executable = file.executables[executable_index].clone();
    let operation_symbol = file
        .link_targets
        .executables
        .iter()
        .find_map(|(symbol, export)| {
            (export.executable_index as usize == executable_index).then(|| symbol.clone())
        })
        .unwrap_or_else(|| operation.to_string());
    Ok((file, executable_index, operation_symbol, executable))
}

fn package_index_with_unit(
    package_unit: &PackageUnit,
    unit: &PublishedJsonArtifact,
    package_path: &str,
) -> PublishedJsonArtifact {
    let value = json!({
        "schemaVersion": "skiff-package-index-v1",
        "packageId": package_unit.package_id,
        "version": package_unit.version,
        "packageUnit": {
            "schemaVersion": PACKAGE_UNIT_SCHEMA_VERSION,
            "packageId": package_unit.package_id,
            "version": package_unit.version,
            "buildIdentity": package_unit.build_identity,
            "abiIdentity": package_unit.abi_identity,
            "unitHash": unit.hash,
            "unitPath": unit.path,
        },
    });
    let hash = value_sha256(&value);
    PublishedJsonArtifact {
        value,
        identity: String::new(),
        hash,
        path: format!(
            "indexes/packages/{package_path}/versions/{}.json",
            package_unit.version
        ),
    }
}

fn service_assembly_artifact(
    input: &WriteTestServiceArtifactRootInput,
    files: &[PublishedFileIrArtifact],
    package_artifacts: &PackageArtifacts,
    service_unit: &PublishedJsonArtifact,
    config_activation: &Value,
    db_metadata: &[DbMetadataIr],
) -> PublishedJsonArtifact {
    let service_path = PublicationId::parse(&input.service_id)
        .expect("service id was already validated")
        .artifact_path();
    let package_configs = package_artifacts
        .dependencies
        .iter()
        .map(|dependency| {
            json!({
                "packageId": dependency.id,
                "alias": dependency.alias,
                "defaultConfig": {},
                "configShape": empty_config_shape_value(),
                "configActivation": config_activation,
            })
        })
        .collect::<Vec<_>>();
    let service = json!({
        "id": input.service_id,
        "revisionId": TEST_REVISION_ID,
        "protocolIdentity": protocol_identity_for_test(input, files),
        "api": Value::Null,
    });
    let operations = vec![assembly_operation(input, files)];
    let source_map = assembly_source_map(files);
    let service_unit_pointer = json!({
        "schemaVersion": SERVICE_UNIT_SCHEMA_VERSION,
        "unitIdentity": service_unit.identity,
        "unitHash": service_unit.hash,
        "unitPath": service_unit.path,
    });
    let hash_input = json!({
        "schemaVersion": SERVICE_ASSEMBLY_SCHEMA_VERSION,
        "kind": SERVICE_ASSEMBLY_KIND,
        "service": service,
        "files": files.iter().map(file_ir_artifact_pointer).collect::<Vec<_>>(),
        "packageConfigs": package_configs,
        "preludeIdentity": Value::Null,
        "prelude": Value::Null,
        "configShape": empty_config_shape_value(),
        "configUses": Value::Null,
        "configActivation": config_activation.clone(),
        "configRequirements": empty_config_requirements_value(),
        "db": serde_json::to_value(db_metadata).expect("DB metadata must serialize"),
        "operations": operations,
        "gateway": {},
        "timeout": Value::Null,
        "dependencyLock": [],
        "serviceUnit": service_unit_pointer.clone(),
        "sourceMap": source_map,
    });
    let hash = value_sha256(&hash_input);
    let assembly_identity = identity(SERVICE_ASSEMBLY_IDENTITY_PREFIX, &hash);
    let path = format!("assemblies/services/{service_path}/{hash}.json");
    let mut value = hash_input;
    value["service"]["assemblyIdentity"] = Value::String(assembly_identity.clone());
    value["serviceUnit"] = service_unit_pointer;
    PublishedJsonArtifact {
        value,
        identity: assembly_identity,
        hash,
        path,
    }
}

fn assembly_operation(
    input: &WriteTestServiceArtifactRootInput,
    files: &[PublishedFileIrArtifact],
) -> Value {
    let params = test_executable(files, &input.operation_name, &input.operation_module)
        .ok()
        .map(|(_, _, _, executable)| {
            executable
                .params
                .into_iter()
                .map(|param| {
                    json!({
                        "name": param.name,
                        "schema": { "type": "any" },
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "operation": input.operation_name,
        "entrypoint": input.target,
        "mode": "unary",
        "parameters": params,
        "response": { "type": "any" },
        "summary": Value::Null,
    })
}

fn file_ir_artifact_pointer(artifact: &PublishedFileIrArtifact) -> Value {
    json!({
        "schemaVersion": FILE_IR_SCHEMA_VERSION,
        "fileIrIdentity": artifact.identity,
        "fileIrHash": artifact.hash,
        "fileIrPath": artifact.path,
        "sourcePath": artifact.source_path,
        "modulePath": artifact.module_path,
        "role": artifact.role,
    })
}

fn assembly_source_map(files: &[PublishedFileIrArtifact]) -> Value {
    let mut sources = Vec::new();
    let mut spans = Vec::new();
    for file in files {
        let Ok(unit) = file_ir_from_published(file) else {
            continue;
        };
        for source in &unit.source_map.sources {
            sources.push(json!({
                "id": source.id,
                "path": source.path,
                "modulePath": source.module_path,
                "fileIrIdentity": unit.file_ir_identity,
                "sourceAstHash": source.source_ast_hash,
            }));
        }
        for span in &unit.source_map.spans {
            spans.push(json!({
                "id": span.id,
                "source": span.source,
                "kind": span.kind,
                "name": span.name,
                "span": span.span,
                "modulePath": unit.module_path,
                "fileIrIdentity": unit.file_ir_identity,
            }));
        }
    }
    json!({
        "format": "skiff-source-map-v1",
        "sources": sources,
        "spans": spans,
    })
}

fn protocol_identity_for_test(
    input: &WriteTestServiceArtifactRootInput,
    files: &[PublishedFileIrArtifact],
) -> String {
    let hash = value_sha256(&json!({
        "schemaVersion": "skiff-test-protocol-v1",
        "serviceId": input.service_id,
        "version": input.version,
        "operation": input.operation_name,
        "target": input.target,
        "files": files.iter().map(|file| &file.identity).collect::<Vec<_>>(),
    }));
    format!("skiff-protocol-v1:sha256:{hash}")
}

fn dynamic_build_id(
    service_unit: &Value,
    package_units: &[PublishedJsonArtifact],
) -> Result<String, TestArtifactError> {
    let bytes =
        runtime_program_service_unit_identity_bytes_from_json(service_unit).map_err(|error| {
            TestArtifactError::InvalidInput {
                message: error.to_string(),
            }
        })?;
    let package_build_identities = ordered_package_build_identities(service_unit, package_units)?;
    Ok(runtime_program_dynamic_build_id(
        &bytes,
        package_build_identities.iter().map(String::as_str),
    ))
}

fn ordered_package_build_identities(
    service_unit: &Value,
    package_units: &[PublishedJsonArtifact],
) -> Result<Vec<String>, TestArtifactError> {
    let package_unit_by_id = package_units
        .iter()
        .filter_map(|unit| {
            unit.value
                .get("packageId")
                .and_then(Value::as_str)
                .map(|package_id| (package_id.to_string(), unit.identity.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let dependencies = service_unit
        .get("packageDependencies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut identities = Vec::new();
    let mut loaded_build_by_package_id = BTreeMap::new();
    for dependency in dependencies {
        let package_id = dependency
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| TestArtifactError::InvalidInput {
                message: "service unit package dependency id must be a string".to_string(),
            })?;
        let build_identity = package_unit_by_id.get(package_id).cloned().ok_or_else(|| {
            TestArtifactError::InvalidInput {
                message: format!("test package unit for dependency {package_id} was not written"),
            }
        })?;
        if let Some(existing) = loaded_build_by_package_id.get(package_id) {
            if existing != &build_identity {
                return Err(TestArtifactError::InvalidInput {
                    message: format!(
                        "package {package_id} is resolved to both {existing} and {build_identity}"
                    ),
                });
            }
            continue;
        }
        identities.push(build_identity.clone());
        loaded_build_by_package_id.insert(package_id.to_string(), build_identity);
    }
    Ok(identities)
}

fn write_artifact_tree(
    root: &Path,
    files: &[PublishedFileIrArtifact],
    package_artifacts: &PackageArtifacts,
    service_unit: &PublishedJsonArtifact,
    assembly: &PublishedJsonArtifact,
    service_id: &PublicationId,
    version: &str,
    build_id: &str,
    pointer_build_id: &str,
    config: &Value,
) -> Result<(), TestArtifactError> {
    for file in files {
        write_json(root, &file.path, &file.value())?;
    }
    for file in &package_artifacts.file_units {
        write_json(root, &file.path, &file.value())?;
    }
    for package_unit in &package_artifacts.units {
        write_json(root, &package_unit.path, &package_unit.value)?;
    }
    for package_index in &package_artifacts.indexes {
        write_json(root, &package_index.path, &package_index.value)?;
    }
    write_json(root, &service_unit.path, &service_unit.value)?;
    write_json(root, &assembly.path, &assembly.value)?;
    let pointer_path = format!("dev/services/{}.json", service_id.artifact_path());
    let pointer = json!({
        "mode": "dev",
        "serviceId": service_id.as_str(),
        "profile": "test",
        "buildId": pointer_build_id,
        "contractHash": format!(
            "sha256:{}",
            assembly
                .value
                .pointer("/service/protocolIdentity")
                .and_then(Value::as_str)
                .and_then(|identity| identity.rsplit_once(":sha256:").map(|(_, hash)| hash))
                .unwrap_or("")
        ),
        "protocolIdentity": assembly.value.pointer("/service/protocolIdentity").cloned().unwrap_or(Value::Null),
        "serviceAssembly": {
            "assemblyIdentity": assembly.identity,
            "assemblyPath": assembly.path,
        },
        "serviceUnit": {
            "unitIdentity": service_unit.identity,
            "unitHash": service_unit.hash,
            "unitPath": service_unit.path,
        },
    });
    write_json(root, &pointer_path, &pointer)?;
    let service_path = service_id.artifact_path();
    let version_path = format!("versions/services/{service_path}/{version}.json");
    let version_pointer = json!({
        "schemaVersion": "skiff-service-version-pointer-v1",
        "serviceId": service_id.as_str(),
        "version": version,
        "buildId": build_id,
    });
    write_json(root, &version_path, &version_pointer)?;
    let build_path = format!(
        "builds/services/{service_path}/{}.json",
        service_build_hash(build_id)?
    );
    let build_record = json!({
        "schemaVersion": SERVICE_BUILD_IDENTITY_PREFIX,
        "serviceId": service_id.as_str(),
        "serviceVersion": version,
        "buildId": build_id,
        "contractIdentity": assembly.value.pointer("/service/protocolIdentity").cloned().unwrap_or(Value::Null),
        "serviceAssembly": {
            "assemblyIdentity": assembly.identity,
            "assemblyPath": assembly.path,
        },
        "serviceUnit": {
            "unitIdentity": service_unit.identity,
            "unitHash": service_unit.hash,
            "unitPath": service_unit.path,
        },
    });
    write_json(root, &build_path, &build_record)?;
    if config.as_object().is_some_and(|object| !object.is_empty()) {
        write_yaml(
            root,
            &format!("configs/services/{}/config.yml", service_id.artifact_path()),
            config,
        )?;
    }
    Ok(())
}

fn service_build_hash(build_id: &str) -> Result<&str, TestArtifactError> {
    let Some((prefix, hash)) = build_id.rsplit_once(":sha256:") else {
        return Err(TestArtifactError::InvalidInput {
            message: "service buildId must include :sha256:".to_string(),
        });
    };
    if prefix != SERVICE_BUILD_IDENTITY_PREFIX {
        return Err(TestArtifactError::InvalidInput {
            message: format!(
                "service buildId prefix must be {SERVICE_BUILD_IDENTITY_PREFIX}, got {prefix}"
            ),
        });
    }
    if hash.len() != 64
        || !hash
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
    {
        return Err(TestArtifactError::InvalidInput {
            message: "service buildId sha256 hash must be 64 lowercase hex characters".to_string(),
        });
    }
    Ok(hash)
}

fn write_json(root: &Path, relative_path: &str, value: &Value) -> Result<(), TestArtifactError> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| TestArtifactError::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|source| TestArtifactError::SerializeJson {
            path: path.display().to_string(),
            source,
        })?;
    fs::write(&path, [bytes, b"\n".to_vec()].concat()).map_err(|source| TestArtifactError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn write_yaml(root: &Path, relative_path: &str, value: &Value) -> Result<(), TestArtifactError> {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| TestArtifactError::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let text = serde_yaml::to_string(value).map_err(|source| TestArtifactError::SerializeYaml {
        path: path.display().to_string(),
        source,
    })?;
    fs::write(&path, text).map_err(|source| TestArtifactError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn config_wrapped_for_router(
    test_config: &Value,
    service_db_mongo_url: Option<&str>,
    dependencies: &[PackageDependencyConstraint],
) -> Value {
    let mut service = test_config.as_object().cloned().unwrap_or_default();
    if let Some(mongo_url) = service_db_mongo_url {
        service.insert(
            "serviceDb".to_string(),
            json!({
                "mongoUrl": mongo_url,
            }),
        );
    }

    let mut config = JsonMap::new();
    if !service.is_empty() {
        config.insert("service".to_string(), Value::Object(service));
    }

    let mut package_config = test_config.as_object().cloned().unwrap_or_default();
    package_config.remove("serviceDb");
    if !package_config.is_empty() {
        let packages = dependencies
            .iter()
            .map(|dependency| {
                (
                    dependency.alias.clone(),
                    Value::Object(package_config.clone()),
                )
            })
            .collect::<JsonMap<_, _>>();
        config.insert("packages".to_string(), Value::Object(packages));
    }

    Value::Object(config)
}

fn config_leaf_paths(config: &JsonMap<String, Value>) -> Vec<String> {
    let mut paths = Vec::new();
    collect_config_leaf_paths(config, "", &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_config_leaf_paths(
    config: &JsonMap<String, Value>,
    prefix: &str,
    paths: &mut Vec<String>,
) {
    for (key, value) in config {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Value::Object(object) = value {
            collect_config_leaf_paths(object, &path, paths);
        } else {
            paths.push(path);
        }
    }
}

fn empty_config_shape_value() -> Value {
    json!({
        "schemaVersion": "skiff-config-shape-v1",
        "entries": [],
    })
}

fn empty_config_requirements_value() -> Value {
    json!({
        "own": [],
        "dependency": [],
        "effective": [],
    })
}

fn config_activation_value(paths: &[String]) -> Value {
    json!({
        "schemaVersion": "skiff-config-activation-v1",
        "hasPaths": paths,
    })
}

fn package_config_and_effect_metadata(paths: &[String]) -> ConfigAndEffectMetadata {
    let mut config = BTreeMap::new();
    config.insert(
        "shape".to_string(),
        metadata_value_from_json(empty_config_shape_value()),
    );
    config.insert("uses".to_string(), metadata_value_from_json(Value::Null));
    config.insert(
        "activation".to_string(),
        metadata_value_from_json(config_activation_value(paths)),
    );
    ConfigAndEffectMetadata {
        config,
        effects: BTreeMap::new(),
    }
}

fn metadata_value_from_json(value: Value) -> MetadataValue {
    match value {
        Value::Null => MetadataValue::Null,
        Value::Bool(value) => MetadataValue::Bool(value),
        Value::Number(value) => MetadataValue::Number(value),
        Value::String(value) => MetadataValue::String(value),
        Value::Array(items) => {
            MetadataValue::Array(items.into_iter().map(metadata_value_from_json).collect())
        }
        Value::Object(object) => MetadataValue::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, metadata_value_from_json(value)))
                .collect(),
        ),
    }
}

fn runtime_program_db_metadata(
    input_artifacts: &[TestRuntimeArtifact],
    artifacts: &[PublishedFileIrArtifact],
) -> Vec<DbMetadataIr> {
    artifacts
        .iter()
        .flat_map(|artifact| {
            let input = input_artifacts.iter().find(|input| {
                input.source_path == artifact.source_path
                    && input.module_path == artifact.module_path
            });
            artifact.unit.declarations.db.values().map(move |db| {
                let package_id = input.and_then(|input| input.package_id.clone());
                DbMetadataIr {
                    module_path: artifact.unit.module_path.clone(),
                    source_role: artifact.role.clone(),
                    package_id: package_id.clone(),
                    package_version: package_id.as_ref().map(|_| "test".to_string()),
                    file_ir_identity: Some(artifact.identity.clone()),
                    kind: db.kind.clone(),
                    ty: db.type_ref.clone(),
                    type_name: db.type_name.clone(),
                    collection_name: db.collection_name.clone(),
                    key: Some(db.key.clone()),
                    fields: db.fields.clone(),
                    retention: db.retention.clone(),
                    leases: db.leases.clone(),
                    indexes: db
                        .indexes
                        .iter()
                        .map(|index| DbMetadataIndexIr {
                            name: index.name.clone(),
                            unique: index.unique,
                            fields: index.fields.clone(),
                            where_expr: index.where_expr.clone(),
                        })
                        .collect(),
                }
            })
        })
        .collect()
}

fn insert_package_function_export(
    exports: &mut PackageExportIndex,
    package_id: &str,
    symbol_path: &str,
    file: FileIrRef,
    executable_index: usize,
    executable: &ExecutableIr,
) {
    for symbol in package_export_symbol_aliases(package_id, symbol_path) {
        exports
            .functions
            .entry(symbol.clone())
            .or_insert_with(|| ExecutableExport {
                symbol,
                file: file.clone(),
                executable_index: executable_index as u32,
                signature: ExecutableSignatureIr {
                    params: executable.params.clone(),
                    return_type: executable.return_type.clone(),
                    self_type: executable.self_type.clone(),
                    may_suspend: executable.may_suspend,
                },
            });
    }
}

fn insert_package_type_export(
    exports: &mut PackageExportIndex,
    package_id: &str,
    symbol_path: &str,
    file: FileIrRef,
    type_index: usize,
    ty: &skiff_compiler_lowering::file_ir::TypeDeclIr,
) {
    for symbol in package_export_symbol_aliases(package_id, symbol_path) {
        exports
            .types
            .entry(symbol.clone())
            .or_insert_with(|| TypeExport {
                symbol,
                file: file.clone(),
                type_index: type_index as u32,
                descriptor: Some(ty.descriptor.clone()),
                type_params: ty.type_params.clone(),
                interface_methods: Vec::new(),
            });
    }
}

fn package_export_symbol_aliases(package_id: &str, symbol_path: &str) -> Vec<String> {
    let mut aliases = vec![symbol_path.to_string()];
    if !symbol_path.starts_with(&format!("{package_id}.")) {
        aliases.push(format!("{package_id}.{symbol_path}"));
    }
    aliases
}

fn package_symbol_for_file_executable(
    file: &FileIrUnit,
    executable: &ExecutableIr,
) -> Option<String> {
    let local_symbol = file
        .link_targets
        .executables
        .iter()
        .find_map(|(symbol, target)| {
            (file
                .executables
                .get(target.executable_index as usize)
                .is_some_and(|candidate| candidate.symbol == executable.symbol))
            .then(|| symbol.as_str())
        })
        .unwrap_or(executable.symbol.as_str());
    let root = package_root_for_module(&file.module_path)?;
    let symbol_path = file
        .module_path
        .strip_prefix(&format!("{root}."))
        .map(|module_tail| format!("{module_tail}.{local_symbol}"))
        .unwrap_or_else(|| local_symbol.to_string());
    Some(symbol_path)
}

fn package_symbol_for_file_type(file: &FileIrUnit, local_symbol: &str) -> Option<String> {
    let root = package_root_for_module(&file.module_path)?;
    let symbol_path = file
        .module_path
        .strip_prefix(&format!("{root}."))
        .map(|module_tail| format!("{module_tail}.{local_symbol}"))
        .unwrap_or_else(|| local_symbol.to_string());
    Some(symbol_path)
}

fn package_dependencies(
    artifacts: &[TestRuntimeArtifact],
    package_symbols_by_id: &HashMap<String, Vec<String>>,
    package_aliases: &BTreeMap<String, Vec<String>>,
) -> Vec<PackageDependencyConstraint> {
    let mut dependency_refs = BTreeMap::<String, String>::new();
    for package_id in package_symbols_by_id.keys() {
        if package_id == SKIFF_STD_PUBLICATION_ID {
            continue;
        }
        dependency_refs.insert(
            package_dependency_alias_for_id(package_id),
            package_id.clone(),
        );
    }
    for artifact in artifacts {
        for symbol in package_symbol_refs(&artifact.file_ir) {
            match &symbol.package {
                PackageRefIr::PackageId { package_id } => {
                    if package_id != SKIFF_STD_PUBLICATION_ID
                        && package_symbols_by_id.contains_key(package_id)
                    {
                        dependency_refs
                            .entry(package_dependency_alias_for_id(package_id))
                            .or_insert_with(|| package_id.clone());
                    }
                }
                PackageRefIr::Dependency { dependency_ref } => {
                    if dependency_refs.contains_key(dependency_ref) {
                        continue;
                    }
                    if let Some(package_id) = pseudo_package_id_for_dependency_ref_symbol(
                        package_symbols_by_id,
                        package_aliases,
                        dependency_ref,
                        &symbol.symbol_path,
                    ) {
                        dependency_refs.insert(dependency_ref.clone(), package_id.to_string());
                    }
                }
            }
        }
    }

    dependency_refs
        .into_iter()
        .map(|(alias, id)| PackageDependencyConstraint {
            id,
            version: "test".to_string(),
            alias,
            config: Value::Null,
        })
        .collect()
}

fn package_dependency_alias_for_id(package_id: &str) -> String {
    let mut alias = package_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !alias
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        alias.insert_str(0, "pkg_");
    }
    alias
}

fn package_symbol_refs(file: &FileIrUnit) -> Vec<PackageSymbolRef> {
    let mut symbols = Vec::new();
    symbols.extend(file.external_refs.package_symbols.iter().cloned());
    for ty in &file.type_table {
        collect_package_symbols_from_descriptor(
            &serde_json::to_value(&ty.descriptor).unwrap_or(Value::Null),
            &mut symbols,
        );
    }
    for executable in &file.executables {
        for param in &executable.params {
            collect_package_symbols_from_type_ref(&param.ty, &mut symbols);
        }
        collect_package_symbols_from_type_ref(&executable.return_type, &mut symbols);
        if let Some(ty) = &executable.self_type {
            collect_package_symbols_from_type_ref(ty, &mut symbols);
        }
        collect_package_symbols_from_body(&executable.body, &mut symbols);
    }
    symbols
}

fn collect_package_symbols_from_body(body: &ExecutableBody, symbols: &mut Vec<PackageSymbolRef>) {
    for expression in &body.expressions {
        match expression {
            skiff_compiler_lowering::file_ir::ExprIr::Construct { type_ref, .. } => {
                collect_package_symbols_from_type_ref(type_ref, symbols);
            }
            skiff_compiler_lowering::file_ir::ExprIr::Call { call } => {
                for ty in call.type_args.values() {
                    collect_package_symbols_from_type_ref(ty, symbols);
                }
            }
            skiff_compiler_lowering::file_ir::ExprIr::Catch { catch_type, .. } => {
                if let Some(ty) = catch_type {
                    collect_package_symbols_from_type_ref(ty, symbols);
                }
            }
            skiff_compiler_lowering::file_ir::ExprIr::DbOperation { operation } => {
                collect_package_symbols_from_type_ref(&operation.target.type_ref, symbols);
                collect_package_symbols_from_type_ref(&operation.result_type, symbols);
            }
            skiff_compiler_lowering::file_ir::ExprIr::DbTransaction { transaction } => {
                collect_package_symbols_from_type_ref(&transaction.result_type, symbols);
            }
            _ => {}
        }
    }
    for statement in &body.statements {
        if let skiff_compiler_lowering::file_ir::StmtIr::Match { arms, .. } = statement {
            for arm in arms {
                collect_package_symbols_from_pattern(&arm.pattern, symbols);
            }
        }
    }
}

fn collect_package_symbols_from_pattern(
    pattern: &skiff_compiler_lowering::file_ir::PatternIr,
    symbols: &mut Vec<PackageSymbolRef>,
) {
    if let skiff_compiler_lowering::file_ir::PatternIr::Type { ty } = pattern {
        collect_package_symbols_from_type_ref(ty, symbols);
    }
}

fn collect_package_symbols_from_type_ref(ty: &TypeRefIr, symbols: &mut Vec<PackageSymbolRef>) {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => symbols.push(symbol.clone()),
        TypeRefIr::Native { args, .. } => {
            for arg in args {
                collect_package_symbols_from_type_ref(arg, symbols);
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                collect_package_symbols_from_type_ref(field, symbols);
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                collect_package_symbols_from_type_ref(item, symbols);
            }
        }
        TypeRefIr::Nullable { inner } => collect_package_symbols_from_type_ref(inner, symbols),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_package_symbols_from_type_ref(&param.ty, symbols);
            }
            collect_package_symbols_from_type_ref(return_type, symbols);
        }
        _ => {}
    }
}

fn collect_package_symbols_from_descriptor(value: &Value, symbols: &mut Vec<PackageSymbolRef>) {
    if let Some(symbol) = descriptor_package_symbol(value) {
        symbols.push(symbol);
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_package_symbols_from_descriptor(item, symbols);
            }
        }
        Value::Object(object) => {
            for item in object.values() {
                collect_package_symbols_from_descriptor(item, symbols);
            }
        }
        _ => {}
    }
}

fn descriptor_package_symbol(value: &Value) -> Option<PackageSymbolRef> {
    let object = value.as_object()?;
    if object.get("kind").and_then(Value::as_str) == Some("packageSymbol") {
        return object
            .get("symbol")
            .and_then(|symbol| serde_json::from_value(symbol.clone()).ok());
    }
    if object.len() == 1 {
        return object
            .get("package")
            .and_then(|symbol| serde_json::from_value(symbol.clone()).ok());
    }
    None
}

fn pseudo_package_id_for_dependency_ref_symbol<'a>(
    package_symbols_by_id: &'a HashMap<String, Vec<String>>,
    package_aliases: &BTreeMap<String, Vec<String>>,
    dependency_ref: &str,
    symbol_path: &str,
) -> Option<&'a str> {
    if let Some(roots) = package_aliases.get(dependency_ref) {
        for root in roots {
            let Some(package_id) = package_root_for_module(root) else {
                continue;
            };
            let Some((package_id, symbols)) =
                package_symbols_by_id.get_key_value(package_id.as_str())
            else {
                continue;
            };
            if symbols.iter().any(|symbol| symbol == symbol_path) {
                return Some(package_id.as_str());
            }
        }
        for root in roots {
            let Some(package_id) = package_root_for_module(root) else {
                continue;
            };
            if let Some((package_id, _)) = package_symbols_by_id.get_key_value(package_id.as_str())
            {
                return Some(package_id.as_str());
            }
        }
    }
    pseudo_package_id_for_symbol(package_symbols_by_id, symbol_path)
}

fn pseudo_package_id_for_symbol<'a>(
    package_symbols_by_id: &'a HashMap<String, Vec<String>>,
    symbol_path: &str,
) -> Option<&'a str> {
    package_symbols_by_id
        .iter()
        .find(|(package_id, symbols)| {
            package_id.as_str() != SKIFF_STD_PUBLICATION_ID
                && symbols.iter().any(|symbol| symbol == symbol_path)
        })
        .map(|(package_id, _)| package_id.as_str())
}

fn package_root_for_module(module_path: &str) -> Option<String> {
    let root = module_path.split('.').next()?.to_string();
    (!root.is_empty()).then_some(root)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use skiff_compiler_core::artifact::{
        interface_instantiation_ref, DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr,
        DbObjectKindIr, InterfaceDeclIr, InterfaceOperationIr, TypeDeclIr, TypeDescriptorIr,
    };
    use skiff_compiler_emission::identity::PACKAGE_BUILD_IDENTITY_PREFIX;
    use skiff_compiler_lowering::file_ir::{
        ExecutableBody, ExecutableIr, ExecutableKind, ExecutableLinkTargetIr, SlotLayout, TypeRefIr,
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn writes_service_unit_hash_after_file_artifact_paths_are_attached() {
        let root = TempRoot::create("service-unit-hash");
        let output = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![TestRuntimeArtifact {
                source_path: "tests/service_test.skiff".to_string(),
                module_path: "internal.test".to_string(),
                role: "implementation".to_string(),
                package_id: None,
                file_ir: sample_file_ir("internal.test", "run"),
            }],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "internal.test".to_string(),
            target: "internal.test.run".to_string(),
            test_config: json!({}),
            service_db_mongo_url: None,
        })
        .expect("test service artifacts should be written");

        let service_unit_path = root.path.join(&output.service_unit_path);
        let service_unit_value: Value =
            serde_json::from_slice(&fs::read(&service_unit_path).expect("service unit file"))
                .expect("service unit JSON");
        let service_unit: ServiceUnit =
            serde_json::from_value(service_unit_value).expect("typed service unit");
        let expected_hash = service_unit_hash(&service_unit);

        assert!(output
            .service_unit_path
            .ends_with(&format!("{expected_hash}.json")));
        let file_artifact_path = service_unit.files[0]
            .artifact_path
            .as_deref()
            .expect("service file should point at written FileIR artifact");
        assert!(file_artifact_path.starts_with("units/files/"));
        assert!(root.path.join(file_artifact_path).is_file());

        let pointer_path = root.path.join("dev/services/example~com~~test.json");
        let pointer: Value =
            serde_json::from_slice(&fs::read(pointer_path).expect("dev pointer file"))
                .expect("dev pointer JSON");
        assert_eq!(
            pointer.pointer("/serviceUnit/unitHash"),
            Some(&Value::String(expected_hash))
        );

        let assembly_path = root.path.join(&output.service_assembly_path);
        let mut assembly_value: Value =
            serde_json::from_slice(&fs::read(assembly_path).expect("service assembly file"))
                .expect("service assembly JSON");
        assert!(assembly_value.get("configRequirements").is_some());
        let assembly_identity = assembly_value
            .pointer("/service/assemblyIdentity")
            .and_then(Value::as_str)
            .expect("service assembly should include assemblyIdentity")
            .to_string();
        assembly_value
            .pointer_mut("/service")
            .and_then(Value::as_object_mut)
            .expect("service assembly service should be an object")
            .remove("assemblyIdentity");
        let content_hash = value_sha256(&assembly_value);
        assert_eq!(
            assembly_identity,
            identity(SERVICE_ASSEMBLY_IDENTITY_PREFIX, &content_hash)
        );
    }

    #[test]
    fn orders_package_build_identities_by_service_dependency_order() {
        let service_unit = json!({
            "packageDependencies": [
                { "id": "example.com/b" },
                { "id": "example.com/a" }
            ],
        });
        let package_units = vec![
            PublishedJsonArtifact {
                value: json!({ "packageId": "example.com/a" }),
                identity: "build-a".to_string(),
                hash: "hash-a".to_string(),
                path: "units/packages/a/hash-a.json".to_string(),
            },
            PublishedJsonArtifact {
                value: json!({ "packageId": "example.com/b" }),
                identity: "build-b".to_string(),
                hash: "hash-b".to_string(),
                path: "units/packages/b/hash-b.json".to_string(),
            },
        ];

        let ordered = ordered_package_build_identities(&service_unit, &package_units)
            .expect("package identities should be ordered");

        assert_eq!(ordered, vec!["build-b".to_string(), "build-a".to_string()]);
    }

    #[test]
    fn test_artifacts_do_not_publish_platform_std_as_service_package_dependency() {
        let root = TempRoot::create("platform-std-package-dependency");
        let mut std_file = sample_file_ir("std.http", "request");
        std_file.link_targets.executables.insert(
            "http.request".to_string(),
            ExecutableLinkTargetIr {
                executable_index: 0,
            },
        );

        let output = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![
                TestRuntimeArtifact {
                    source_path: "std/http.skiff".to_string(),
                    module_path: "std.http".to_string(),
                    role: "package".to_string(),
                    package_id: Some(SKIFF_STD_PUBLICATION_ID.to_string()),
                    file_ir: std_file,
                },
                TestRuntimeArtifact {
                    source_path: "tests/service_test.skiff".to_string(),
                    module_path: "internal.test".to_string(),
                    role: "implementation".to_string(),
                    package_id: None,
                    file_ir: sample_file_ir("internal.test", "run"),
                },
            ],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "internal.test".to_string(),
            target: "internal.test.run".to_string(),
            test_config: json!({"bailian": {"apiKey": "sk-test"}}),
            service_db_mongo_url: None,
        })
        .expect("test service artifacts should be written");

        let service_unit_path = root.path.join(&output.service_unit_path);
        let service_unit_value: Value =
            serde_json::from_slice(&fs::read(&service_unit_path).expect("service unit file"))
                .expect("service unit JSON");
        let service_unit: ServiceUnit =
            serde_json::from_value(service_unit_value).expect("typed service unit");
        assert!(service_unit.package_dependencies.is_empty());

        let assembly_value: Value = serde_json::from_slice(
            &fs::read(root.path.join(&output.service_assembly_path))
                .expect("service assembly file"),
        )
        .expect("service assembly JSON");
        assert_eq!(assembly_value["packageConfigs"], Value::Array(Vec::new()));

        let config_path = root
            .path
            .join("configs/services/example~com~~test/config.yml");
        let config = fs::read_to_string(config_path).expect("router config file");
        assert!(config.contains("bailian"));
        assert!(!config.contains("skiff.run/std"));
    }

    #[test]
    fn test_artifacts_project_recoverable_db_metadata_for_any_interface_fields() {
        let root = TempRoot::create("recoverable-db-any-interface");
        let db_file = sample_recoverable_db_file_ir();

        let output = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![TestRuntimeArtifact {
                source_path: "packages/agent/run.skiff".to_string(),
                module_path: "agent.run".to_string(),
                role: "package".to_string(),
                package_id: Some("example.com/agent".to_string()),
                file_ir: db_file,
            }],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "agent.run".to_string(),
            target: "agent.run.run".to_string(),
            test_config: json!({}),
            service_db_mongo_url: None,
        })
        .expect("test service artifacts should be written");

        let service_unit_value: Value =
            serde_json::from_slice(&fs::read(root.path.join(&output.service_unit_path)).unwrap())
                .expect("service unit JSON");
        let assembly_value: Value = serde_json::from_slice(
            &fs::read(root.path.join(&output.service_assembly_path))
                .expect("service assembly JSON"),
        )
        .expect("service assembly JSON");

        for db_root in [&service_unit_value["db"], &assembly_value["db"]] {
            let agent_run = db_root
                .as_array()
                .and_then(|entries| entries.iter().find(|entry| entry["typeName"] == "AgentRun"))
                .expect("AgentRun DB metadata");
            let fields = agent_run["fields"].as_array().expect("db fields array");
            let runtime_bindings = fields
                .iter()
                .find(|field| field["name"] == "runtimeBindings")
                .expect("runtimeBindings field should remain in DB metadata");
            assert_eq!(runtime_bindings["type"]["kind"], "record");
            assert_eq!(
                runtime_bindings["type"]["fields"]["providers"]["args"][0]["kind"],
                "anyInterface"
            );
            assert!(
                fields.iter().any(|field| field["name"] == "currentConfig"),
                "plain recoverable sibling field should also remain"
            );
        }

        let lane = &service_unit_value["recoverableMetadata"]["storageLanes"]
            ["db:AgentRun:field:runtimeBindings"];
        assert_eq!(lane["lane"], "recoverableEnvelope");
        assert_eq!(lane["envelopeSlotRef"], "db:AgentRun.runtimeBindings");
        assert!(lane["expectedType"]["runtimeCarrierCheckRequired"]
            .as_bool()
            .expect("carrier check flag"));
    }

    #[test]
    fn test_artifacts_reject_non_recoverable_db_function_fields() {
        let root = TempRoot::create("recoverable-db-function-rejected");
        let mut db_file = sample_recoverable_db_file_ir();
        let callback_ty = TypeRefIr::Function {
            params: vec![FunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("string")),
        };
        db_file
            .declarations
            .db
            .get_mut("AgentRun")
            .unwrap()
            .fields
            .push(DbObjectFieldIr {
                name: "callback".to_string(),
                ty: callback_ty.clone(),
            });

        let TypeDescriptorIr::Record { fields } = &mut db_file.type_table[0].descriptor else {
            panic!("AgentRun type should be a record");
        };
        fields.insert("callback".to_string(), callback_ty);

        let error = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![TestRuntimeArtifact {
                source_path: "packages/agent/run.skiff".to_string(),
                module_path: "agent.run".to_string(),
                role: "package".to_string(),
                package_id: Some("example.com/agent".to_string()),
                file_ir: db_file,
            }],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "agent.run".to_string(),
            target: "agent.run.run".to_string(),
            test_config: json!({}),
            service_db_mongo_url: None,
        })
        .expect_err("function DB fields must not enter recoverable metadata");

        let message = error.to_string();
        assert!(message.contains("db field AgentRun.callback"));
        assert!(message.contains("callback function type"));
    }

    #[test]
    fn test_artifacts_use_artifact_safe_package_config_alias_for_package_id_refs() {
        let root = TempRoot::create("package-config-alias");
        let mongo_url = "mongodb://127.0.0.1:27017/skiff-test?directConnection=true";
        let output = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![
                TestRuntimeArtifact {
                    source_path: "pkg/api.skiff".to_string(),
                    module_path: "api".to_string(),
                    role: "package".to_string(),
                    package_id: Some("example.com/live".to_string()),
                    file_ir: sample_file_ir("api", "helper"),
                },
                TestRuntimeArtifact {
                    source_path: "tests/service_test.skiff".to_string(),
                    module_path: "internal.test".to_string(),
                    role: "implementation".to_string(),
                    package_id: None,
                    file_ir: sample_file_ir("internal.test", "run"),
                },
            ],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "internal.test".to_string(),
            target: "internal.test.run".to_string(),
            test_config: json!({"dashscope": {"apiKey": "sk-test"}}),
            service_db_mongo_url: Some(mongo_url.to_string()),
        })
        .expect("test service artifacts should be written");

        let service_unit_value: Value =
            serde_json::from_slice(&fs::read(root.path.join(&output.service_unit_path)).unwrap())
                .expect("service unit JSON");
        let service_unit: ServiceUnit =
            serde_json::from_value(service_unit_value).expect("typed service unit");
        assert_eq!(service_unit.package_dependencies.len(), 1);
        assert_eq!(
            service_unit.package_dependencies[0].alias,
            "example_com_live"
        );

        let config_path = root
            .path
            .join("configs/services/example~com~~test/config.yml");
        let config = fs::read_to_string(config_path).expect("router config file");
        assert!(config.contains("example_com_live"));
        assert!(!config.contains("example.com/live"));
        let config_value: Value = serde_yaml::from_str(&config).expect("router config YAML");
        assert_eq!(
            config_value.pointer("/service/serviceDb/mongoUrl"),
            Some(&Value::String(mongo_url.to_string()))
        );
        assert_eq!(
            config_value.pointer("/packages/example_com_live/dashscope/apiKey"),
            Some(&Value::String("sk-test".to_string()))
        );
        assert!(config_value
            .pointer("/packages/example_com_live/serviceDb")
            .is_none());
    }

    #[test]
    fn test_artifacts_write_service_db_config_without_test_config() {
        let root = TempRoot::create("service-db-empty-config");
        let mongo_url = "mongodb://127.0.0.1:27017/skiff-test?directConnection=true";
        let output = write_test_service_artifact_root(WriteTestServiceArtifactRootInput {
            artifact_root: root.path.clone(),
            service_id: "example.com/test".to_string(),
            version: "test".to_string(),
            artifacts: vec![
                TestRuntimeArtifact {
                    source_path: "pkg/api.skiff".to_string(),
                    module_path: "api".to_string(),
                    role: "package".to_string(),
                    package_id: Some("example.com/live".to_string()),
                    file_ir: sample_file_ir("api", "helper"),
                },
                TestRuntimeArtifact {
                    source_path: "tests/service_test.skiff".to_string(),
                    module_path: "internal.test".to_string(),
                    role: "implementation".to_string(),
                    package_id: None,
                    file_ir: sample_file_ir("internal.test", "run"),
                },
            ],
            package_aliases: BTreeMap::new(),
            operation_name: "run".to_string(),
            operation_module: "internal.test".to_string(),
            target: "internal.test.run".to_string(),
            test_config: json!({}),
            service_db_mongo_url: Some(mongo_url.to_string()),
        })
        .expect("test service artifacts should be written");

        assert_eq!(output.service_db_mongo_url.as_deref(), Some(mongo_url));

        let service_unit_value: Value =
            serde_json::from_slice(&fs::read(root.path.join(&output.service_unit_path)).unwrap())
                .expect("service unit JSON");
        let service_unit: ServiceUnit =
            serde_json::from_value(service_unit_value).expect("typed service unit");
        assert_eq!(service_unit.package_dependencies.len(), 1);
        assert_eq!(
            service_unit.package_dependencies[0].alias,
            "example_com_live"
        );

        let config_path = root
            .path
            .join("configs/services/example~com~~test/config.yml");
        let config = fs::read_to_string(config_path).expect("router config file");
        let config_value: Value = serde_yaml::from_str(&config).expect("router config YAML");
        assert_eq!(
            config_value.pointer("/service/serviceDb/mongoUrl"),
            Some(&Value::String(mongo_url.to_string()))
        );
        assert!(config_value.pointer("/packages").is_none());
    }

    #[test]
    fn dynamic_test_build_id_normalizes_missing_package_dependency_config() {
        let publication_abi =
            serde_json::to_value(PublicationAbiUnit::empty("example.com/test", "test", ""))
                .expect("empty publication ABI serializes");
        let service_unit = json!({
            "schemaVersion": SERVICE_UNIT_SCHEMA_VERSION,
            "service": { "id": "example.com/test", "displayName": "Skiff Test" },
            "version": "test",
            "protocolIdentity": "skiff-protocol-v1:sha256:61d38bc757fa99efcf975457088fcd700ec74f6d1b1dde44202a74344b0dc4e3",
            "publicationAbi": publication_abi.clone(),
            "files": [],
            "packageDependencies": [
                { "id": "example.com/pkg", "version": "test", "alias": "example_com_pkg" }
            ],
            "packageAbiExpectations": [],
            "operations": [],
            "db": [],
            "gateway": {},
            "config": {},
        });
        let service_unit_with_config = json!({
            "schemaVersion": SERVICE_UNIT_SCHEMA_VERSION,
            "service": { "id": "example.com/test", "displayName": "Skiff Test" },
            "version": "test",
            "protocolIdentity": "skiff-protocol-v1:sha256:61d38bc757fa99efcf975457088fcd700ec74f6d1b1dde44202a74344b0dc4e3",
            "publicationAbi": publication_abi,
            "files": [],
            "packageDependencies": [
                { "id": "example.com/pkg", "version": "test", "alias": "example_com_pkg", "config": null }
            ],
            "packageAbiExpectations": [],
            "operations": [],
            "db": [],
            "gateway": {},
            "config": {},
        });
        let package_units = vec![PublishedJsonArtifact {
            value: json!({ "packageId": "example.com/pkg" }),
            identity: identity(
                PACKAGE_BUILD_IDENTITY_PREFIX,
                "4b24b73ab87ead763385ad32675bc66b5f113ec8730b16489428ed0a21b8d1ea",
            ),
            hash: "hash-pkg".to_string(),
            path: "units/packages/example~com~~pkg/hash-pkg.json".to_string(),
        }];

        let build_id = dynamic_build_id(&service_unit, &package_units).expect("build id");
        let build_id_with_config =
            dynamic_build_id(&service_unit_with_config, &package_units).expect("build id");

        assert_eq!(build_id, build_id_with_config);
    }

    fn sample_file_ir(module_path: &str, symbol: &str) -> FileIrUnit {
        let mut unit = FileIrUnit::empty(module_path, "source-ast-hash");
        unit.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: format!("{module_path}.{symbol}"),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::native("unit"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            source_span: None,
        });
        unit.link_targets.executables.insert(
            symbol.to_string(),
            ExecutableLinkTargetIr {
                executable_index: 0,
            },
        );
        unit
    }

    fn sample_recoverable_db_file_ir() -> FileIrUnit {
        let mut unit = sample_file_ir("agent.run", "run");
        let event_receiver_symbol = TypeRefIr::ServiceSymbol {
            symbol: skiff_compiler_lowering::file_ir::ServiceSymbolRef {
                module_path: "agent.run".to_string(),
                symbol: "AgentEventReceiver".to_string(),
            },
        };
        let tool_provider_symbol = TypeRefIr::ServiceSymbol {
            symbol: skiff_compiler_lowering::file_ir::ServiceSymbolRef {
                module_path: "agent.run".to_string(),
                symbol: "ToolProvider".to_string(),
            },
        };
        let event_receiver_any = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(event_receiver_symbol, Vec::new()),
        };
        let tool_provider_any = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(tool_provider_symbol, Vec::new()),
        };
        let runtime_bindings_ty = TypeRefIr::Record {
            fields: BTreeMap::from([
                ("events".to_string(), event_receiver_any),
                (
                    "providers".to_string(),
                    TypeRefIr::Native {
                        name: "Array".to_string(),
                        args: vec![tool_provider_any],
                    },
                ),
            ]),
        };
        let current_config_ty = TypeRefIr::Record {
            fields: BTreeMap::from([("runtimeBindings".to_string(), runtime_bindings_ty.clone())]),
        };

        unit.type_table.push(TypeDeclIr {
            name: "AgentRun".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("id".to_string(), TypeRefIr::native("string")),
                    ("currentConfig".to_string(), current_config_ty.clone()),
                    ("runtimeBindings".to_string(), runtime_bindings_ty.clone()),
                ]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        unit.declarations.types.insert(
            "AgentRun".to_string(),
            skiff_compiler_lowering::file_ir::TypeDeclarationIr {
                type_index: 0,
                symbol: "AgentRun".to_string(),
                source_span: None,
            },
        );
        unit.declarations.interfaces.insert(
            "AgentEventReceiver".to_string(),
            sample_interface("AgentEventReceiver"),
        );
        unit.declarations
            .interfaces
            .insert("ToolProvider".to_string(), sample_interface("ToolProvider"));
        unit.declarations.db.insert(
            "AgentRun".to_string(),
            DbDeclarationIr {
                type_ref: TypeRefIr::native("AgentRun"),
                type_name: "AgentRun".to_string(),
                collection_name: "agent_run".to_string(),
                kind: DbObjectKindIr::Object,
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::native("string"),
                },
                fields: vec![
                    DbObjectFieldIr {
                        name: "id".to_string(),
                        ty: TypeRefIr::native("string"),
                    },
                    DbObjectFieldIr {
                        name: "currentConfig".to_string(),
                        ty: current_config_ty,
                    },
                    DbObjectFieldIr {
                        name: "runtimeBindings".to_string(),
                        ty: runtime_bindings_ty,
                    },
                ],
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        unit
    }

    fn sample_interface(name: &str) -> InterfaceDeclIr {
        InterfaceDeclIr {
            name: name.to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: "ping".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::native("string"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        }
    }

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn create(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "skiff-compiler-{label}-{}-{}-{}",
                std::process::id(),
                current_nanos(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("temp artifact root");
            Self { path }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn current_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    }
}
