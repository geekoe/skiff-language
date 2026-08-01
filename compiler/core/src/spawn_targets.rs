use std::collections::BTreeMap;

use skiff_artifact_model::{
    CallIr, CallTargetIr, ExecutableDeclarationIr, ExecutableKind, FileIrRef, FileIrUnit,
    MetadataValue, OperationCallableKind, OperationTargetRef, SpawnTargetIr, SpawnTargetKindIr,
    TypeRefIr,
};

pub use crate::type_closure::PackageTypeSource as PackageSpawnTargetSource;

const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SPAWN_FUNCTION_TARGET_PREFIX: &str = "function:";

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
                if !spawn_submit_has_projected_target(metadata)? {
                    continue;
                }
                let Some(target) = service_spawn_target_for_call(
                    service_file_ir_units,
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
                    if !spawn_submit_has_projected_target(metadata)? {
                        continue;
                    }
                    let Some(target) = package_spawn_target_for_call(
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

fn spawn_submit_has_projected_target(metadata: &MetadataValue) -> Result<bool> {
    let MetadataValue::Object(object) = metadata else {
        return Err(error("spawnSubmit metadata must be an object"));
    };
    let Some(MetadataValue::String(target_kind)) = object.get("targetKind") else {
        return Err(error("spawnSubmit metadata targetKind must be a string"));
    };
    if target_kind == "function" {
        return Ok(true);
    }
    if target_kind == "actorMethod" {
        // Actor method spawns route by actor identity, not by executable
        // route. They are validated at assembly link time and carry no
        // artifact-level SpawnTargetIr.
        return Ok(false);
    }
    Err(error(format!(
        "spawn target kind {target_kind} is unsupported"
    )))
}

fn service_spawn_target_for_call(
    file_ir_units: &[FileIrUnit],
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
        CallTargetIr::PublicationExecutable {
            module_path,
            executable_index,
        } => {
            let (target_unit, declaration_name, declaration) =
                publication_executable_declaration_for_index(
                    file_ir_units,
                    module_path,
                    *executable_index,
                    "service",
                )?;
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", declaration.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                target_unit,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        // A direct package call is external to this File IR owner. Assembly
        // resolves its callable identity; spawn projection does not relink it.
        CallTargetIr::PackageCallable { .. } => Ok(None),
        CallTargetIr::InterfaceMethod { .. } | CallTargetIr::ActorMethod { .. } => Ok(None),
        // A service boundary call is not a same-build executable spawn target.
        CallTargetIr::ServiceCall { .. } => Ok(None),
        CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. } => Ok(None),
    }
}

fn package_spawn_target_for_call(
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
        CallTargetIr::PublicationExecutable {
            module_path,
            executable_index,
        } => {
            let (target_unit, declaration_name, declaration) =
                publication_executable_declaration_for_index(
                    &package.file_ir_units,
                    module_path,
                    *executable_index,
                    &format!("package {}", package.package_id),
                )?;
            let target_identity = package_handler_target(&package.package_id, &declaration.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                target_unit,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        // A direct package call is external to this File IR owner. Assembly
        // resolves its callable identity; spawn projection does not relink it.
        CallTargetIr::PackageCallable { .. } => Ok(None),
        // A service boundary call is not a same-build executable spawn target.
        CallTargetIr::ServiceCall { .. } => Ok(None),
        CallTargetIr::ServiceDependencySymbol { .. }
        | CallTargetIr::Native { .. }
        | CallTargetIr::Builtin { .. }
        | CallTargetIr::ReceiverBuiltin { .. }
        | CallTargetIr::InterfaceMethod { .. }
        | CallTargetIr::ActorMethod { .. } => Ok(None),
    }
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

fn publication_executable_declaration_for_index<'a>(
    file_ir_units: &'a [FileIrUnit],
    module_path: &str,
    executable_index: u32,
    publication_context: &str,
) -> Result<(&'a FileIrUnit, &'a String, &'a ExecutableDeclarationIr)> {
    let mut matching_units = file_ir_units
        .iter()
        .filter(|unit| unit.module_path == module_path);
    let Some(target_unit) = matching_units.next() else {
        return Err(error(format!(
            "spawn target executable index {executable_index} references missing {publication_context} module {module_path}"
        )));
    };
    if matching_units.next().is_some() {
        return Err(error(format!(
            "spawn target executable index {executable_index} references duplicate {publication_context} module {module_path}"
        )));
    }
    let Some((declaration_name, declaration)) =
        executable_declaration_for_index(target_unit, executable_index)
    else {
        return Err(error(format!(
            "spawn target executable index {executable_index} is not declared in {publication_context} module {module_path}"
        )));
    };
    Ok((target_unit, declaration_name, declaration))
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
        TypeRefIr::Builtin { name, args }
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
mod tests;
