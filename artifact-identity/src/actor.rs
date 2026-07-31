use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map, Value};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorAbiInput, ActorImplementationIdentity, ActorMethodIdentity,
    CallTargetIr, ExprIr, FileIrUnit,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, ACTOR_ABI_IDENTITY_PREFIX, ACTOR_ABI_IDENTITY_SCHEMA_MARKER,
    ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_SCHEMA_MARKER,
    ACTOR_METHOD_IDENTITY_PREFIX, ACTOR_METHOD_IDENTITY_SCHEMA_MARKER,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActorAbiIdentityPreimage<'a> {
    schema: &'static str,
    abi: &'a ActorAbiInput,
}

pub fn actor_abi_identity(abi: &ActorAbiInput) -> Result<ActorAbiIdentity> {
    let bytes = canonical_ir_bytes(
        &ActorAbiIdentityPreimage {
            schema: ACTOR_ABI_IDENTITY_SCHEMA_MARKER,
            abi,
        },
        ArtifactIdentityError::SerializeActorAbiIdentity,
    )?;
    Ok(ActorAbiIdentity::new(framed_identity(
        ACTOR_ABI_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActorMethodIdentityPreimage<'a> {
    schema: &'static str,
    module_path: &'a str,
    actor_name: &'a str,
    method_name: &'a str,
}

/// Derives the stable identity of an actor method from its declaration owner and
/// name. Signature facts deliberately remain in the actor ABI identity, so an
/// ABI revision does not silently rename every method.
pub fn actor_method_identity(
    module_path: &str,
    actor_name: &str,
    method_name: &str,
) -> Result<ActorMethodIdentity> {
    if module_path.trim().is_empty()
        || actor_name.trim().is_empty()
        || method_name.trim().is_empty()
    {
        return Err(
            ArtifactIdentityError::InvalidActorImplementationIdentityInput {
                message: "actor method module path, actor name and method name must be non-empty"
                    .to_string(),
            },
        );
    }
    let bytes = canonical_ir_bytes(
        &ActorMethodIdentityPreimage {
            schema: ACTOR_METHOD_IDENTITY_SCHEMA_MARKER,
            module_path,
            actor_name,
            method_name,
        },
        ArtifactIdentityError::SerializeActorMethodIdentity,
    )?;
    Ok(ActorMethodIdentity::new(framed_identity(
        ACTOR_METHOD_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActorImplementationIdentityPreimage {
    schema: &'static str,
    actor_abi_identity: ActorAbiIdentity,
    roots: BTreeMap<ActorMethodIdentity, ExecutableKey>,
    executables: Vec<NormalizedExecutable>,
    constants: Vec<NormalizedConstant>,
    types: Vec<NormalizedType>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecutableKey {
    module_path: String,
    symbol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedExecutable {
    key: ExecutableKey,
    executable: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstantKey {
    module_path: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedConstant {
    key: ConstantKey,
    constant: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeKey {
    module_path: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedType {
    key: TypeKey,
    declaration: Value,
}

/// Hashes exactly the executable graph reachable from an actor's public method
/// roots. The graph is serialized as a sorted set of nodes, rather than as
/// recursively nested hashes, so self-recursion and arbitrary SCCs are stable.
pub fn actor_implementation_identity(
    units: &[FileIrUnit],
    actor_module_path: &str,
    actor_name: &str,
) -> Result<ActorImplementationIdentity> {
    let units_by_module = index_units(units)?;
    let owner = units_by_module.get(actor_module_path).ok_or_else(|| {
        invalid_actor_input(format!(
            "actor owner module `{actor_module_path}` was not found"
        ))
    })?;
    let actor = owner
        .actor_declarations
        .iter()
        .find(|actor| actor.abi.actor_name == actor_name)
        .ok_or_else(|| {
            invalid_actor_input(format!(
                "actor `{actor_name}` was not found in module `{actor_module_path}`"
            ))
        })?;

    let public_ids = actor
        .abi
        .public_methods
        .iter()
        .map(|method| method.method_identity.clone())
        .collect::<BTreeSet<_>>();
    let implementation_ids = actor
        .method_implementations
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if public_ids != implementation_ids {
        return Err(invalid_actor_input(
            "actor method implementations do not exactly match its public method identities",
        ));
    }

    let mut roots = BTreeMap::new();
    let mut pending = Vec::new();
    for (method, index) in &actor.method_implementations {
        let key = executable_key(owner, *index)?;
        roots.insert(method.clone(), key.clone());
        pending.push(key);
    }

    let mut reachable = BTreeSet::new();
    let mut reachable_constants = BTreeSet::new();
    let mut pending_constants: Vec<ConstantKey> = Vec::new();
    while !pending.is_empty() || !pending_constants.is_empty() {
        let (unit, expressions) = if let Some(key) = pending.pop() {
            if !reachable.insert(key.clone()) {
                continue;
            }
            let (unit, executable) = resolve_executable(&units_by_module, &key)?;
            (unit, &executable.body.expressions)
        } else {
            let key = pending_constants.pop().expect("queue is non-empty");
            if !reachable_constants.insert(key.clone()) {
                continue;
            }
            let (unit, constant) = resolve_constant(&units_by_module, &key)?;
            (unit, &constant.body.expressions)
        };
        for expression in expressions {
            if let ExprIr::LoadConst { const_index } = expression {
                pending_constants.push(constant_key(unit, *const_index)?);
            }
            let ExprIr::Call { call } = expression else {
                continue;
            };
            if let Some(callee) = direct_callee_key(&units_by_module, unit, &call.target)? {
                pending.push(callee);
            }
        }
    }

    let mut executables = Vec::new();
    let mut pending_types = Vec::new();
    for key in reachable {
        let (unit, executable) = resolve_executable(&units_by_module, &key)?;
        let mut value = serde_json::to_value(executable)
            .map_err(ArtifactIdentityError::SerializeActorImplementationIdentity)?;
        collect_type_keys(&value, &units_by_module, unit, &mut pending_types)?;
        normalize_executable_value(&mut value, &units_by_module, unit)?;
        executables.push(NormalizedExecutable {
            key,
            executable: value,
        });
    }
    let mut constants = Vec::new();
    for key in reachable_constants {
        let (unit, constant) = resolve_constant(&units_by_module, &key)?;
        let mut value = serde_json::to_value(constant)
            .map_err(ArtifactIdentityError::SerializeActorImplementationIdentity)?;
        collect_type_keys(&value, &units_by_module, unit, &mut pending_types)?;
        normalize_executable_value(&mut value, &units_by_module, unit)?;
        constants.push(NormalizedConstant {
            key,
            constant: value,
        });
    }
    let mut reachable_types = BTreeSet::new();
    let mut types = Vec::new();
    while let Some(key) = pending_types.pop() {
        if !reachable_types.insert(key.clone()) {
            continue;
        }
        let (unit, declaration) = resolve_type(&units_by_module, &key)?;
        let mut value = serde_json::to_value(declaration)
            .map_err(ArtifactIdentityError::SerializeActorImplementationIdentity)?;
        collect_type_keys(&value, &units_by_module, unit, &mut pending_types)?;
        normalize_executable_value(&mut value, &units_by_module, unit)?;
        types.push(NormalizedType {
            key,
            declaration: value,
        });
    }
    types.sort_by(|left, right| left.key.cmp(&right.key));

    let preimage = ActorImplementationIdentityPreimage {
        schema: ACTOR_IMPLEMENTATION_IDENTITY_SCHEMA_MARKER,
        actor_abi_identity: actor.actor_abi_identity.clone(),
        roots,
        executables,
        constants,
        types,
    };
    let bytes = canonical_ir_bytes(
        &preimage,
        ArtifactIdentityError::SerializeActorImplementationIdentity,
    )?;
    Ok(ActorImplementationIdentity::new(framed_identity(
        ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

fn index_units(units: &[FileIrUnit]) -> Result<BTreeMap<&str, &FileIrUnit>> {
    let mut indexed = BTreeMap::new();
    for unit in units {
        if indexed.insert(unit.module_path.as_str(), unit).is_some() {
            return Err(invalid_actor_input(format!(
                "duplicate File IR module path `{}`",
                unit.module_path
            )));
        }
    }
    Ok(indexed)
}

fn executable_key(unit: &FileIrUnit, index: u32) -> Result<ExecutableKey> {
    let executable = unit.executables.get(index as usize).ok_or_else(|| {
        invalid_actor_input(format!(
            "module `{}` has no executable at index {index}",
            unit.module_path
        ))
    })?;
    Ok(ExecutableKey {
        module_path: unit.module_path.clone(),
        symbol: executable.symbol.clone(),
    })
}

fn constant_key(unit: &FileIrUnit, index: u32) -> Result<ConstantKey> {
    let constant = unit.constants.get(index as usize).ok_or_else(|| {
        invalid_actor_input(format!(
            "module `{}` has no constant at index {index}",
            unit.module_path
        ))
    })?;
    Ok(ConstantKey {
        module_path: unit.module_path.clone(),
        name: constant.name.clone(),
    })
}

fn resolve_constant<'a>(
    units: &BTreeMap<&'a str, &'a FileIrUnit>,
    key: &ConstantKey,
) -> Result<(&'a FileIrUnit, &'a skiff_artifact_model::file_ir::ConstIr)> {
    let unit = units.get(key.module_path.as_str()).ok_or_else(|| {
        invalid_actor_input(format!("module `{}` was not found", key.module_path))
    })?;
    let mut matching = unit
        .constants
        .iter()
        .filter(|constant| constant.name == key.name);
    let constant = matching.next().ok_or_else(|| {
        invalid_actor_input(format!(
            "constant `{}` was not found in module `{}`",
            key.name, key.module_path
        ))
    })?;
    if matching.next().is_some() {
        return Err(invalid_actor_input(format!(
            "duplicate constant `{}` in module `{}`",
            key.name, key.module_path
        )));
    }
    Ok((unit, constant))
}

fn type_key(unit: &FileIrUnit, index: u32) -> Result<TypeKey> {
    let declaration = unit.type_table.get(index as usize).ok_or_else(|| {
        invalid_actor_input(format!(
            "module `{}` has no type at index {index}",
            unit.module_path
        ))
    })?;
    Ok(TypeKey {
        module_path: unit.module_path.clone(),
        name: declaration.name.clone(),
    })
}

fn resolve_type<'a>(
    units: &BTreeMap<&'a str, &'a FileIrUnit>,
    key: &TypeKey,
) -> Result<(&'a FileIrUnit, &'a skiff_artifact_model::types::TypeDeclIr)> {
    let unit = units.get(key.module_path.as_str()).ok_or_else(|| {
        invalid_actor_input(format!("module `{}` was not found", key.module_path))
    })?;
    let mut matching = unit
        .type_table
        .iter()
        .filter(|declaration| declaration.name == key.name);
    let declaration = matching.next().ok_or_else(|| {
        invalid_actor_input(format!(
            "type `{}` was not found in module `{}`",
            key.name, key.module_path
        ))
    })?;
    if matching.next().is_some() {
        return Err(invalid_actor_input(format!(
            "duplicate type `{}` in module `{}`",
            key.name, key.module_path
        )));
    }
    Ok((unit, declaration))
}

fn resolve_executable<'a>(
    units: &BTreeMap<&'a str, &'a FileIrUnit>,
    key: &ExecutableKey,
) -> Result<(
    &'a FileIrUnit,
    &'a skiff_artifact_model::executable::ExecutableIr,
)> {
    let unit = units.get(key.module_path.as_str()).ok_or_else(|| {
        invalid_actor_input(format!("module `{}` was not found", key.module_path))
    })?;
    let mut matching = unit
        .executables
        .iter()
        .filter(|executable| executable.symbol == key.symbol);
    let executable = matching.next().ok_or_else(|| {
        invalid_actor_input(format!(
            "executable `{}` was not found in module `{}`",
            key.symbol, key.module_path
        ))
    })?;
    if matching.next().is_some() {
        return Err(invalid_actor_input(format!(
            "duplicate executable symbol `{}` in module `{}`",
            key.symbol, key.module_path
        )));
    }
    Ok((unit, executable))
}

fn direct_callee_key(
    units: &BTreeMap<&str, &FileIrUnit>,
    owner: &FileIrUnit,
    target: &CallTargetIr,
) -> Result<Option<ExecutableKey>> {
    let (module_path, index) = match target {
        CallTargetIr::LocalExecutable { executable_index } => {
            (owner.module_path.as_str(), *executable_index)
        }
        CallTargetIr::PublicationExecutable {
            module_path,
            executable_index,
        } => (module_path.as_str(), *executable_index),
        _ => return Ok(None),
    };
    let unit = units.get(module_path).ok_or_else(|| {
        invalid_actor_input(format!("callee module `{module_path}` was not found"))
    })?;
    executable_key(unit, index).map(Some)
}

fn normalize_executable_value(
    value: &mut Value,
    units: &BTreeMap<&str, &FileIrUnit>,
    owner: &FileIrUnit,
) -> Result<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize_executable_value(item, units, owner)?;
            }
        }
        Value::Object(object) => {
            object.remove("sourceSpan");
            if let Some(kind) = object
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                if kind == "actorMethod" {
                    // A called Actor's implementation is selected independently.
                    // Including it here would make an unrelated Actor upgrade
                    // perturb this Actor's implementation identity.
                    object.remove("actorImplementationIdentity");
                }
                let target = match kind.as_str() {
                    "localExecutable" => object
                        .get("executableIndex")
                        .and_then(Value::as_u64)
                        .map(|index| (owner.module_path.as_str(), index as u32)),
                    "publicationExecutable" => object
                        .get("modulePath")
                        .and_then(Value::as_str)
                        .zip(object.get("executableIndex").and_then(Value::as_u64))
                        .map(|(module, index)| (module, index as u32)),
                    _ => None,
                };
                if let Some((module, index)) = target {
                    let unit = units.get(module).ok_or_else(|| {
                        invalid_actor_input(format!("callee module `{module}` was not found"))
                    })?;
                    let key = executable_key(unit, index)?;
                    *object = Map::from_iter([
                        (
                            "kind".to_string(),
                            Value::String("canonicalExecutable".to_string()),
                        ),
                        ("modulePath".to_string(), Value::String(key.module_path)),
                        ("symbol".to_string(), Value::String(key.symbol)),
                    ]);
                    return Ok(());
                }
                if kind == "loadConst" {
                    let index = object
                        .get("constIndex")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid_actor_input("loadConst is missing constIndex"))?
                        as u32;
                    let key = constant_key(owner, index)?;
                    *object = Map::from_iter([
                        (
                            "kind".to_string(),
                            Value::String("canonicalConstant".to_string()),
                        ),
                        ("modulePath".to_string(), Value::String(key.module_path)),
                        ("name".to_string(), Value::String(key.name)),
                    ]);
                    return Ok(());
                }
                let type_target = match kind.as_str() {
                    "localType" => object
                        .get("typeIndex")
                        .and_then(Value::as_u64)
                        .map(|index| (owner.module_path.as_str(), index as u32)),
                    "publicationType" => object
                        .get("modulePath")
                        .and_then(Value::as_str)
                        .zip(object.get("typeIndex").and_then(Value::as_u64))
                        .map(|(module, index)| (module, index as u32)),
                    _ => None,
                };
                if let Some((module, index)) = type_target {
                    let unit = units.get(module).ok_or_else(|| {
                        invalid_actor_input(format!("type module `{module}` was not found"))
                    })?;
                    let key = type_key(unit, index)?;
                    *object = Map::from_iter([
                        (
                            "kind".to_string(),
                            Value::String("canonicalType".to_string()),
                        ),
                        ("modulePath".to_string(), Value::String(key.module_path)),
                        ("name".to_string(), Value::String(key.name)),
                    ]);
                    return Ok(());
                }
            }
            for child in object.values_mut() {
                normalize_executable_value(child, units, owner)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_type_keys(
    value: &Value,
    units: &BTreeMap<&str, &FileIrUnit>,
    owner: &FileIrUnit,
    output: &mut Vec<TypeKey>,
) -> Result<()> {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_type_keys(item, units, owner, output)?;
            }
        }
        Value::Object(object) => {
            if let Some(Value::String(kind)) = object.get("kind") {
                let target = match kind.as_str() {
                    "localType" => object
                        .get("typeIndex")
                        .and_then(Value::as_u64)
                        .map(|index| (owner.module_path.as_str(), index as u32)),
                    "publicationType" => object
                        .get("modulePath")
                        .and_then(Value::as_str)
                        .zip(object.get("typeIndex").and_then(Value::as_u64))
                        .map(|(module, index)| (module, index as u32)),
                    _ => None,
                };
                if let Some((module, index)) = target {
                    let unit = units.get(module).ok_or_else(|| {
                        invalid_actor_input(format!("type module `{module}` was not found"))
                    })?;
                    output.push(type_key(unit, index)?);
                    return Ok(());
                }
            }
            for child in object.values() {
                collect_type_keys(child, units, owner, output)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn invalid_actor_input(message: impl Into<String>) -> ArtifactIdentityError {
    ArtifactIdentityError::InvalidActorImplementationIdentityInput {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
