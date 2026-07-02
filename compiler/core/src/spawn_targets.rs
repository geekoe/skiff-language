use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallIr, CallTargetIr, ExecutableDeclarationIr, ExecutableIr, ExecutableKind, FileIrRef,
    FileIrUnit, MetadataValue, OperationAbiRef, OperationCallableKind, OperationTargetRef,
    PackageOperationTarget, PackageRefIr, PackageUnit, SpawnTargetIr, SpawnTargetKindIr, TypeRefIr,
};

const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SPAWN_FUNCTION_TARGET_PREFIX: &str = "function:";

#[derive(Clone, Debug)]
pub struct PackageSpawnTargetSource {
    pub package_id: String,
    pub dependency_refs: Vec<String>,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<FileIrUnit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnTargetProjectionError {
    pub message: String,
}

impl std::fmt::Display for SpawnTargetProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SpawnTargetProjectionError {}

type Result<T> = std::result::Result<T, SpawnTargetProjectionError>;

pub fn service_spawn_targets_with_packages(
    service_file_ir_units: &[FileIrUnit],
    package_sources: &[PackageSpawnTargetSource],
    service_protocol_identity: &str,
) -> Result<Vec<SpawnTargetIr>> {
    let mut targets = BTreeMap::<String, SpawnTargetIr>::new();
    for unit in service_file_ir_units {
        for executable in &unit.executables {
            for expr in &executable.body.expressions {
                let skiff_artifact_model::ExprIr::Call { call } = expr else {
                    continue;
                };
                let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
                    continue;
                };
                if !spawn_submit_is_function(metadata)? {
                    continue;
                }
                let Some(target) = service_spawn_target_for_call(
                    service_file_ir_units,
                    package_sources,
                    unit,
                    call,
                    service_protocol_identity,
                )?
                else {
                    continue;
                };
                targets
                    .entry(target.target_identity.clone())
                    .or_insert(target);
            }
        }
    }
    for package in package_sources {
        for unit in &package.file_ir_units {
            for executable in &unit.executables {
                for expr in &executable.body.expressions {
                    let skiff_artifact_model::ExprIr::Call { call } = expr else {
                        continue;
                    };
                    let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
                        continue;
                    };
                    if !spawn_submit_is_function(metadata)? {
                        continue;
                    }
                    let Some(target) = package_spawn_target_for_call(
                        package_sources,
                        package,
                        unit,
                        call,
                        service_protocol_identity,
                    )?
                    else {
                        continue;
                    };
                    targets
                        .entry(target.target_identity.clone())
                        .or_insert(target);
                }
            }
        }
    }
    Ok(targets.into_values().collect())
}

fn spawn_submit_is_function(metadata: &MetadataValue) -> Result<bool> {
    let MetadataValue::Object(object) = metadata else {
        return Err(error("spawnSubmit metadata must be an object"));
    };
    let Some(MetadataValue::String(target_kind)) = object.get("targetKind") else {
        return Err(error("spawnSubmit metadata targetKind must be a string"));
    };
    if target_kind != "function" {
        return Err(error(format!(
            "spawn target kind {target_kind} is unsupported"
        )));
    }
    Ok(true)
}

fn service_spawn_target_for_call(
    file_ir_units: &[FileIrUnit],
    package_sources: &[PackageSpawnTargetSource],
    unit: &FileIrUnit,
    call: &CallIr,
    service_protocol_identity: &str,
) -> Result<Option<SpawnTargetIr>> {
    match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => {
            let Some((declaration_name, declaration)) =
                executable_declaration_for_index(unit, *executable_index)
            else {
                return Err(error(format!(
                    "spawn target executable index {executable_index} is not declared in module {}",
                    unit.module_path
                )));
            };
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", declaration.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                unit,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::ExternalServiceSymbol { symbol } => {
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", symbol.symbol_path());
            Ok(Some(service_function_spawn_target(
                file_ir_units,
                &target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::PackageSymbol {
            package_ref,
            operation,
        } => Ok(Some(package_operation_spawn_target(
            package_sources,
            package_ref,
            operation,
            service_protocol_identity,
        )?)),
        CallTargetIr::InterfaceMethod { .. } => Ok(None),
        CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => Ok(None),
    }
}

fn package_spawn_target_for_call(
    package_sources: &[PackageSpawnTargetSource],
    package: &PackageSpawnTargetSource,
    unit: &FileIrUnit,
    call: &CallIr,
    service_protocol_identity: &str,
) -> Result<Option<SpawnTargetIr>> {
    match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => {
            let Some((declaration_name, declaration)) =
                executable_declaration_for_index(unit, *executable_index)
            else {
                return Err(error(format!(
                    "spawn target executable index {executable_index} is not declared in package module {}",
                    unit.module_path
                )));
            };
            let executable = unit
                .executables
                .get(declaration.executable_index as usize)
                .ok_or_else(|| {
                    error(format!(
                        "spawn target {}.{} points to missing executable index {}",
                        unit.module_path, declaration_name, declaration.executable_index
                    ))
                })?;
            let target_identity = package_handler_target(&package.package_id, &executable.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                unit,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::ExternalServiceSymbol { symbol } => {
            let symbol_path = symbol.symbol_path();
            let target_identity = package_handler_target(&package.package_id, &symbol_path);
            Ok(Some(package_function_spawn_target(
                package,
                &target_identity,
                &symbol_path,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::PackageSymbol {
            package_ref,
            operation,
        } => Ok(Some(package_operation_spawn_target(
            package_sources,
            package_ref,
            operation,
            service_protocol_identity,
        )?)),
        CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. }
        | CallTargetIr::InterfaceMethod { .. } => Ok(None),
    }
}

fn service_function_spawn_target(
    file_ir_units: &[FileIrUnit],
    target_identity: &str,
    service_protocol_identity: &str,
) -> Result<SpawnTargetIr> {
    for unit in file_ir_units {
        for (declaration_name, declaration) in &unit.declarations.executables {
            let Some(executable) = unit.executables.get(declaration.executable_index as usize)
            else {
                return Err(error(format!(
                    "spawn target {}.{} points to missing executable index {}",
                    unit.module_path, declaration_name, declaration.executable_index
                )));
            };
            if executable.kind != ExecutableKind::Function {
                continue;
            }
            if format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", executable.symbol) != target_identity {
                continue;
            }
            return Ok(SpawnTargetIr {
                target_identity: target_identity.to_string(),
                kind: SpawnTargetKindIr::Function,
                executable_target: operation_target_ref(
                    unit,
                    declaration_name,
                    declaration.executable_index,
                    OperationCallableKind::InternalFunction,
                ),
                param_types: executable
                    .params
                    .iter()
                    .map(|param| param.ty.clone())
                    .collect(),
                return_type: spawn_function_return_type(target_identity, &executable.return_type)?,
                service_protocol_identity: service_protocol_identity.to_string(),
            });
        }
    }
    Err(error(format!(
        "spawn target {target_identity} does not resolve to a service function"
    )))
}

fn package_operation_spawn_target(
    package_sources: &[PackageSpawnTargetSource],
    package_ref: &PackageRefIr,
    operation: &OperationAbiRef,
    service_protocol_identity: &str,
) -> Result<SpawnTargetIr> {
    let package = package_source_for_ref(package_sources, package_ref, &operation.public_path)?;
    let Some(target) = package
        .unit
        .implementation_links
        .operation_targets
        .get(&operation.operation_abi_id)
    else {
        return Err(error(format!(
            "spawn package target {} operationAbiId {} does not resolve to a package operation target",
            package.package_id, operation.operation_abi_id
        )));
    };
    let PackageOperationTarget::LocalExecutable { target, .. } = target else {
        return Err(error(format!(
            "spawn package target {}.{} resolves to a receiver operation; spawn supports function targets only",
            package.package_id, operation.public_path
        )));
    };
    let executable = package_operation_executable(package, target)?;
    let target_identity = package_handler_target(&package.package_id, &operation.public_path);
    Ok(SpawnTargetIr {
        target_identity: target_identity.clone(),
        kind: SpawnTargetKindIr::Function,
        executable_target: target.clone(),
        param_types: executable
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: spawn_function_return_type(&target_identity, &executable.return_type)?,
        service_protocol_identity: service_protocol_identity.to_string(),
    })
}

fn package_function_spawn_target(
    package: &PackageSpawnTargetSource,
    target_identity: &str,
    executable_symbol: &str,
    service_protocol_identity: &str,
) -> Result<SpawnTargetIr> {
    for unit in &package.file_ir_units {
        for (declaration_name, declaration) in &unit.declarations.executables {
            let Some(executable) = unit.executables.get(declaration.executable_index as usize)
            else {
                return Err(error(format!(
                    "spawn target {}.{} points to missing executable index {}",
                    unit.module_path, declaration_name, declaration.executable_index
                )));
            };
            if executable.kind != ExecutableKind::Function || executable.symbol != executable_symbol
            {
                continue;
            }
            return Ok(SpawnTargetIr {
                target_identity: target_identity.to_string(),
                kind: SpawnTargetKindIr::Function,
                executable_target: operation_target_ref(
                    unit,
                    declaration_name,
                    declaration.executable_index,
                    OperationCallableKind::InternalFunction,
                ),
                param_types: executable
                    .params
                    .iter()
                    .map(|param| param.ty.clone())
                    .collect(),
                return_type: spawn_function_return_type(target_identity, &executable.return_type)?,
                service_protocol_identity: service_protocol_identity.to_string(),
            });
        }
    }
    Err(error(format!(
        "spawn package target {}.{executable_symbol} does not resolve to a package function",
        package.package_id
    )))
}

fn package_operation_executable<'a>(
    package: &'a PackageSpawnTargetSource,
    target: &OperationTargetRef,
) -> Result<&'a ExecutableIr> {
    let Some(unit) = package
        .file_ir_units
        .iter()
        .find(|unit| unit.file_ir_identity == target.file_ref.file_ir_identity)
    else {
        return Err(error(format!(
            "package operation target file {} is missing from package {}",
            target.file_ref.file_ir_identity, package.package_id
        )));
    };
    unit.executables
        .get(target.executable_index as usize)
        .ok_or_else(|| {
            error(format!(
                "package operation target {} executable index {} is missing",
                target.file_ref.file_ir_identity, target.executable_index
            ))
        })
}

fn package_source_for_ref<'a>(
    package_sources: &'a [PackageSpawnTargetSource],
    package_ref: &PackageRefIr,
    operation_path: &str,
) -> Result<&'a PackageSpawnTargetSource> {
    let package_id = match package_ref {
        PackageRefIr::PackageId { package_id } => package_id.as_str(),
        PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
    };
    package_sources
        .iter()
        .find(|package| {
            package.package_id == package_id
                || package
                    .dependency_refs
                    .iter()
                    .any(|dependency_ref| dependency_ref == package_id)
        })
        .ok_or_else(|| {
            error(format!(
                "spawn package target {operation_path} does not resolve to a linked package"
            ))
        })
}

fn executable_declaration_for_index(
    unit: &FileIrUnit,
    executable_index: u32,
) -> Option<(&String, &ExecutableDeclarationIr)> {
    unit.declarations
        .executables
        .iter()
        .find(|(_, declaration)| declaration.executable_index == executable_index)
}

fn operation_target_ref(
    unit: &FileIrUnit,
    symbol: &str,
    executable_index: u32,
    callable_kind: OperationCallableKind,
) -> OperationTargetRef {
    OperationTargetRef {
        file_ref: FileIrRef::new(unit.file_ir_identity.clone(), unit.module_path.clone()),
        executable_index,
        callable_abi_id: format!("callable:{}.{}", unit.module_path, symbol),
        callable_kind,
    }
}

fn function_spawn_target_from_declaration(
    unit: &FileIrUnit,
    declaration_name: &str,
    executable_index: u32,
    target_identity: String,
    service_protocol_identity: &str,
) -> Result<SpawnTargetIr> {
    let Some(executable) = unit.executables.get(executable_index as usize) else {
        return Err(error(format!(
            "spawn target {}.{} points to missing executable index {}",
            unit.module_path, declaration_name, executable_index
        )));
    };
    if executable.kind != ExecutableKind::Function || declaration_name.contains('.') {
        return Err(error(format!(
            "spawn target {target_identity} must resolve to a function"
        )));
    }
    Ok(SpawnTargetIr {
        target_identity,
        kind: SpawnTargetKindIr::Function,
        executable_target: operation_target_ref(
            unit,
            declaration_name,
            executable_index,
            OperationCallableKind::InternalFunction,
        ),
        param_types: executable
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: spawn_function_return_type(&executable.symbol, &executable.return_type)?,
        service_protocol_identity: service_protocol_identity.to_string(),
    })
}

fn package_handler_target(package_id: &str, symbol_path: &str) -> String {
    format!(
        "package.{}.{}",
        encode_package_target_segment(package_id),
        encode_package_target_segment(symbol_path)
    )
}

fn encode_package_target_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn spawn_function_return_type(target_identity: &str, ty: &TypeRefIr) -> Result<Option<TypeRefIr>> {
    match ty {
        TypeRefIr::Native { name, args }
            if args.is_empty() && (name == "void" || name == "null") =>
        {
            Ok(None)
        }
        other => Err(error(format!(
            "spawn target {target_identity} must return void/null, found {other:?}"
        ))),
    }
}

fn error(message: impl Into<String>) -> SpawnTargetProjectionError {
    SpawnTargetProjectionError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::{ExecutableBody, ExprIr, SlotLayout};

    #[test]
    fn projects_service_function_spawn_target_from_file_ir() {
        let targets = service_spawn_targets_with_packages(&[service_unit("void")], &[], "proto-1")
            .expect("spawn target projection should succeed");

        assert_eq!(targets.len(), 1);
        let target = &targets[0];
        assert_eq!(target.target_identity, "function:app.run");
        assert_eq!(target.kind, SpawnTargetKindIr::Function);
        assert_eq!(target.executable_target.executable_index, 1);
        assert_eq!(target.executable_target.callable_abi_id, "callable:app.run");
        assert!(target.return_type.is_none());
        assert_eq!(target.service_protocol_identity, "proto-1");
    }

    #[test]
    fn rejects_non_void_spawn_function_return() {
        let error = service_spawn_targets_with_packages(&[service_unit("string")], &[], "proto-1")
            .expect_err("spawn target projection should reject non-void return");

        assert!(error
            .message
            .contains("spawn target app.run must return void/null"));
    }

    fn service_unit(return_type: &str) -> FileIrUnit {
        let mut unit = FileIrUnit::empty("app", "hash");
        unit.file_ir_identity = "file:app".to_string();
        unit.declarations.executables.insert(
            "caller".to_string(),
            ExecutableDeclarationIr {
                executable_index: 0,
                symbol: "app.caller".to_string(),
                source_span: None,
            },
        );
        unit.declarations.executables.insert(
            "run".to_string(),
            ExecutableDeclarationIr {
                executable_index: 1,
                symbol: "app.run".to_string(),
                source_span: None,
            },
        );
        unit.executables = vec![
            ExecutableIr {
                kind: ExecutableKind::Function,
                symbol: "app.caller".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: void_type(),
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody {
                    expressions: vec![ExprIr::Call {
                        call: CallIr {
                            target: CallTargetIr::LocalExecutable {
                                executable_index: 1,
                            },
                            args: Vec::new(),
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::from([(
                                SPAWN_SUBMIT_METADATA_KEY.to_string(),
                                MetadataValue::Object(BTreeMap::from([(
                                    "targetKind".to_string(),
                                    MetadataValue::String("function".to_string()),
                                )])),
                            )]),
                        },
                    }],
                    ..ExecutableBody::default()
                },
                source_span: None,
            },
            ExecutableIr {
                kind: ExecutableKind::Function,
                symbol: "app.run".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::Native {
                    name: return_type.to_string(),
                    args: Vec::new(),
                },
                self_type: None,
                slots: SlotLayout::default(),
                may_suspend: false,
                body: ExecutableBody::default(),
                source_span: None,
            },
        ];
        unit
    }

    fn void_type() -> TypeRefIr {
        TypeRefIr::Native {
            name: "void".to_string(),
            args: Vec::new(),
        }
    }
}
