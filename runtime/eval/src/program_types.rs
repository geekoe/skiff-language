#[cfg(any(test, feature = "test-support"))]
use serde_json::Value;
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linked_program::{type_descriptor_to_value, type_ref_to_value};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, FunctionTypeParamIr, LinkedExecutable,
    LinkedInterfaceInstantiationRef, LinkedNominalTypeRefBase, LinkedTypeRef, PackageRefIr,
    PackageSymbolRef, ResolvedSymbol, ServiceSymbolRef, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;

pub use skiff_runtime_linked_program::executable_type_param_names;

#[cfg(any(test, feature = "test-support"))]
use super::type_descriptor::substitute_type_descriptor;
use super::type_descriptor::TypeSubstitutions;
use super::Interpreter;
#[cfg(any(test, feature = "test-support"))]
use crate::error::{Result, RuntimeError};

pub fn program_type_ref_kind(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::PublicationType { .. } => "publicationType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
        LinkedTypeRef::PackageSchema { .. } => "packageSchema",
        LinkedTypeRef::AppliedNominal { .. } => "appliedNominal",
        LinkedTypeRef::Address { .. } => "address",
        LinkedTypeRef::Native { .. } => "builtin",
        LinkedTypeRef::Record { .. } => "record",
        LinkedTypeRef::Union { .. } => "union",
        LinkedTypeRef::Nullable { .. } => "nullable",
        LinkedTypeRef::Literal { .. } => "literal",
        LinkedTypeRef::TypeParam { .. } => "typeParam",
        LinkedTypeRef::Function { .. } => "function",
        LinkedTypeRef::DbObjectSymbol { .. } => "dbObjectSymbol",
        LinkedTypeRef::AnyInterface { .. } => "anyInterface",
    }
}

impl Interpreter {
    #[cfg(any(test, feature = "test-support"))]
    pub fn program_type_descriptor<'p>(
        &self,
        program: impl Into<ProgramTypeView<'p>>,
        executable_addr: &ExecutableAddr,
        ty: Option<&LinkedTypeRef>,
    ) -> Result<Option<Value>> {
        let program = program.into();
        let Some(type_ref) = ty else {
            return Ok(None);
        };
        let descriptor = match type_ref {
            LinkedTypeRef::Address { addr } => Ok(Some(type_descriptor_to_value(
                program.types.descriptor(addr).ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram type address {addr} is not interned"
                    ))
                })?,
            ))),
            LinkedTypeRef::Native { .. }
            | LinkedTypeRef::AppliedNominal { .. }
            | LinkedTypeRef::Record { .. }
            | LinkedTypeRef::Union { .. }
            | LinkedTypeRef::Nullable { .. }
            | LinkedTypeRef::DbObjectSymbol { .. }
            | LinkedTypeRef::Literal { .. }
            | LinkedTypeRef::TypeParam { .. }
            | LinkedTypeRef::Function { .. }
            | LinkedTypeRef::PackageSchema { .. }
            | LinkedTypeRef::AnyInterface { .. } => Ok(Some(type_ref_to_value(type_ref))),
            LinkedTypeRef::LocalType { .. }
            | LinkedTypeRef::PublicationType { .. }
            | LinkedTypeRef::ServiceSymbol { .. }
            | LinkedTypeRef::PackageSymbol { .. } => Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram type ref {} was not linked before execution",
                program_type_ref_kind(type_ref)
            ))),
        }?;
        Ok(descriptor.map(|descriptor| {
            resolve_program_descriptor_refs_view(program, executable_addr, descriptor, 0)
        }))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn program_runtime_type_entries<'p>(program: impl Into<ProgramTypeView<'p>>) -> Option<Value> {
    let program = program.into();
    if program.types.descriptors.is_empty() {
        return None;
    }
    Some(Value::Array(
        program
            .types
            .descriptors
            .values()
            .map(|decl| type_descriptor_to_value(&decl.descriptor))
            .collect(),
    ))
}

pub fn call_type_substitutions<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    caller_addr: &ExecutableAddr,
    caller_substitutions: &TypeSubstitutions,
    callee: &LinkedExecutable,
    type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
) -> TypeSubstitutions {
    let program = program.into();
    let formal_names = executable_type_param_names(callee);
    if formal_names.is_empty() || type_args.is_empty() {
        return TypeSubstitutions::new();
    }

    let ordered_args = ordered_type_args(type_args);
    let mut substitutions = TypeSubstitutions::new();
    for (index, formal_name) in formal_names.iter().enumerate() {
        let Some(actual) = type_args
            .get(formal_name)
            .or_else(|| ordered_args.get(index).map(|(_, ty)| *ty))
        else {
            continue;
        };
        let binding =
            normalize_call_type_arg_binding(program, caller_addr, actual, caller_substitutions);
        substitutions.insert(formal_name.clone(), binding);
    }
    substitutions
}

fn normalize_call_type_arg_binding(
    program: ProgramTypeView<'_>,
    caller_addr: &ExecutableAddr,
    type_ref: &LinkedTypeRef,
    caller_substitutions: &TypeSubstitutions,
) -> LinkedTypeRef {
    normalize_call_type_arg_binding_inner(
        program,
        caller_addr,
        type_ref,
        caller_substitutions,
        TypeParamSubstitution::Apply,
    )
}

pub fn normalize_program_type_ref<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    caller_addr: &ExecutableAddr,
    type_ref: &LinkedTypeRef,
    caller_substitutions: &TypeSubstitutions,
) -> LinkedTypeRef {
    let program = program.into();
    normalize_call_type_arg_binding_inner(
        program,
        caller_addr,
        type_ref,
        caller_substitutions,
        TypeParamSubstitution::Apply,
    )
}

#[derive(Clone, Copy)]
enum TypeParamSubstitution {
    Apply,
    ResolveOnly,
}

fn normalize_call_type_arg_binding_inner(
    program: ProgramTypeView<'_>,
    caller_addr: &ExecutableAddr,
    type_ref: &LinkedTypeRef,
    caller_substitutions: &TypeSubstitutions,
    type_param_substitution: TypeParamSubstitution,
) -> LinkedTypeRef {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => match type_param_substitution {
            TypeParamSubstitution::Apply => caller_substitutions
                .get(name)
                .cloned()
                .unwrap_or_else(|| type_ref.clone()),
            TypeParamSubstitution::ResolveOnly => type_ref.clone(),
        },
        LinkedTypeRef::LocalType { type_index } => LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: caller_addr.unit.clone(),
                file: caller_addr.file.clone(),
                type_index: *type_index,
            },
        },
        LinkedTypeRef::PublicationType {
            module_path,
            type_index,
        } => program_publication_type_addr(program, caller_addr, module_path, *type_index)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::ServiceSymbol { symbol } => program_service_type_addr(program, symbol)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::PackageSymbol { symbol } => program_package_type_addr(program, symbol)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::PackageSchema { .. } => type_ref.clone(),
        LinkedTypeRef::AppliedNominal { base, arguments } => LinkedTypeRef::AppliedNominal {
            base: normalize_nominal_base(program, caller_addr, base),
            arguments: arguments
                .iter()
                .map(|argument| {
                    normalize_call_type_arg_binding_inner(
                        program,
                        caller_addr,
                        argument,
                        caller_substitutions,
                        TypeParamSubstitution::Apply,
                    )
                })
                .collect(),
        },
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| {
                    normalize_call_type_arg_binding_inner(
                        program,
                        caller_addr,
                        arg,
                        caller_substitutions,
                        TypeParamSubstitution::Apply,
                    )
                })
                .collect(),
        },
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        normalize_call_type_arg_binding_inner(
                            program,
                            caller_addr,
                            ty,
                            caller_substitutions,
                            TypeParamSubstitution::ResolveOnly,
                        ),
                    )
                })
                .collect(),
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| {
                    normalize_call_type_arg_binding_inner(
                        program,
                        caller_addr,
                        item,
                        caller_substitutions,
                        TypeParamSubstitution::Apply,
                    )
                })
                .collect(),
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(normalize_call_type_arg_binding_inner(
                program,
                caller_addr,
                inner,
                caller_substitutions,
                TypeParamSubstitution::Apply,
            )),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| {
                        normalize_call_type_arg_binding_inner(
                            program,
                            caller_addr,
                            arg,
                            caller_substitutions,
                            TypeParamSubstitution::Apply,
                        )
                    })
                    .collect(),
            },
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: normalize_call_type_arg_binding_inner(
                        program,
                        caller_addr,
                        &param.ty,
                        caller_substitutions,
                        TypeParamSubstitution::ResolveOnly,
                    ),
                })
                .collect(),
            return_type: Box::new(normalize_call_type_arg_binding_inner(
                program,
                caller_addr,
                return_type,
                caller_substitutions,
                TypeParamSubstitution::ResolveOnly,
            )),
        },
        LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::Address { .. } => type_ref.clone(),
    }
}

fn normalize_nominal_base(
    program: ProgramTypeView<'_>,
    caller_addr: &ExecutableAddr,
    base: &LinkedNominalTypeRefBase,
) -> LinkedNominalTypeRefBase {
    match base {
        LinkedNominalTypeRefBase::LocalType { type_index } => LinkedNominalTypeRefBase::Address {
            addr: TypeAddr {
                unit: caller_addr.unit.clone(),
                file: caller_addr.file.clone(),
                type_index: *type_index,
            },
        },
        LinkedNominalTypeRefBase::PublicationType {
            module_path,
            type_index,
        } => program_publication_type_addr(program, caller_addr, module_path, *type_index)
            .map(|addr| LinkedNominalTypeRefBase::Address { addr })
            .unwrap_or_else(|| base.clone()),
        LinkedNominalTypeRefBase::ServiceSymbol { symbol } => {
            program_service_type_addr(program, symbol)
                .map(|addr| LinkedNominalTypeRefBase::Address { addr })
                .unwrap_or_else(|| base.clone())
        }
        LinkedNominalTypeRefBase::PackageSymbol { symbol } => {
            program_package_type_addr(program, symbol)
                .map(|addr| LinkedNominalTypeRefBase::Address { addr })
                .unwrap_or_else(|| base.clone())
        }
        LinkedNominalTypeRefBase::PackageSchema { .. }
        | LinkedNominalTypeRefBase::Address { .. } => base.clone(),
    }
}

fn ordered_type_args(
    type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
) -> Vec<(&String, &LinkedTypeRef)> {
    let mut items = type_args.iter().collect::<Vec<_>>();
    items.sort_by(
        |(left, _), (right, _)| match (type_arg_index(left), type_arg_index(right)) {
            (Some(left), Some(right)) => left.cmp(&right),
            _ => left.cmp(right),
        },
    );
    items
}

fn type_arg_index(key: &str) -> Option<usize> {
    key.strip_prefix('T')?.parse().ok()
}

#[cfg(test)]
mod executable_type_param_name_tests {
    use std::collections::BTreeMap;

    use skiff_runtime_linked_program::{
        ExecutableKind, LinkedExecutable, LinkedExecutableBody, LinkedTypeRef, ParamIr,
        ServiceSymbolRef, SlotLayoutIr,
    };

    use super::executable_type_param_names;

    #[test]
    fn runtime_program_collects_type_params_from_structural_return_types() {
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "svc.main.generic".to_string(),
            type_params: vec!["Explicit".to_string()],
            params: vec![ParamIr {
                name: "thread".to_string(),
                slot: 0,
                ty: LinkedTypeRef::DbObjectSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "svc.main".to_string(),
                        symbol: "Thread".to_string(),
                    },
                },
                mode: skiff_runtime_linked_program::ParamModeIr::Value,
            }],
            return_type: Some(LinkedTypeRef::Record {
                fields: BTreeMap::from([("value".to_string(), type_param("U"))]),
            }),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };

        assert_eq!(
            executable_type_param_names(&executable),
            vec!["Explicit".to_string(), "U".to_string()]
        );
    }

    fn type_param(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::TypeParam {
            name: name.to_string(),
        }
    }
}

pub fn program_type_name(type_ref: &LinkedTypeRef) -> Option<String> {
    match type_ref {
        LinkedTypeRef::Native { name, .. } => Some(name.clone()),
        LinkedTypeRef::LocalType { type_index } => Some(format!("localType[{type_index}]")),
        LinkedTypeRef::Address { addr } => Some(addr.to_string()),
        LinkedTypeRef::PublicationType {
            module_path,
            type_index,
        } => Some(format!("publicationType[{module_path}:{type_index}]")),
        LinkedTypeRef::ServiceSymbol { symbol } => Some(symbol.symbol_path()),
        LinkedTypeRef::PackageSymbol { symbol } => Some(symbol.symbol_path.clone()),
        LinkedTypeRef::PackageSchema {
            stable_schema_key, ..
        } => Some(stable_schema_key.clone()),
        LinkedTypeRef::AppliedNominal { base, .. } => match base {
            LinkedNominalTypeRefBase::Address { addr } => Some(addr.to_string()),
            _ => None,
        },
        LinkedTypeRef::Nullable { inner } => program_type_name(inner),
        LinkedTypeRef::AnyInterface { interface } => {
            Some(format!("any {}", interface.interface_abi_id))
        }
        LinkedTypeRef::DbObjectSymbol { symbol } => Some(symbol.symbol_path()),
        LinkedTypeRef::TypeParam { name } => Some(name.clone()),
        LinkedTypeRef::Record { .. }
        | LinkedTypeRef::Union { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::Function { .. } => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn program_type_ref_fallback_value(type_ref: &LinkedTypeRef) -> Value {
    program_type_name(type_ref)
        .map(Value::String)
        .or_else(|| serde_json::to_value(type_ref).ok())
        .unwrap_or(Value::Null)
}

#[cfg(any(test, feature = "test-support"))]
fn program_type_descriptor_value(
    interpreter: &Interpreter,
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    ty: Option<&LinkedTypeRef>,
) -> Option<Value> {
    program_type_descriptor_value_with_substitutions(
        interpreter,
        program,
        current_addr,
        ty,
        &TypeSubstitutions::new(),
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn program_type_descriptor_value_with_substitutions<'p>(
    interpreter: &Interpreter,
    program: impl Into<ProgramTypeView<'p>>,
    current_addr: &ExecutableAddr,
    ty: Option<&LinkedTypeRef>,
    substitutions: &TypeSubstitutions,
) -> Option<Value> {
    let program = program.into();
    let ty = ty?;
    let descriptor = interpreter
        .program_type_descriptor(program, current_addr, Some(ty))
        .ok()
        .flatten()
        .unwrap_or_else(|| program_type_ref_fallback_value(ty));
    let resolved = resolve_program_descriptor_refs_view(program, current_addr, descriptor, 0);
    let substituted =
        substitute_type_descriptor(Some(&resolved), substitutions).unwrap_or(resolved);
    Some(resolve_program_descriptor_refs_view(
        program,
        current_addr,
        substituted,
        0,
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub fn resolve_program_descriptor_refs<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    current_addr: &ExecutableAddr,
    value: Value,
    depth: usize,
) -> Value {
    resolve_program_descriptor_refs_view(program.into(), current_addr, value, depth)
}

#[cfg(any(test, feature = "test-support"))]
fn resolve_program_descriptor_refs_view(
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    value: Value,
    depth: usize,
) -> Value {
    if depth > 32 {
        return value;
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| {
                    resolve_program_descriptor_refs_view(program, current_addr, item, depth + 1)
                })
                .collect(),
        ),
        Value::Object(mut object) => {
            if let Some(addr) = program_descriptor_ref_addr(program, current_addr, &object) {
                if let Some(descriptor) = program.types.descriptor(&addr) {
                    return resolve_program_descriptor_refs_view(
                        program,
                        current_addr,
                        type_descriptor_to_value(descriptor),
                        depth + 1,
                    );
                }
            }
            for item in object.values_mut() {
                let resolved = resolve_program_descriptor_refs_view(
                    program,
                    current_addr,
                    item.take(),
                    depth + 1,
                );
                *item = resolved;
            }
            Value::Object(object)
        }
        other => other,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn program_descriptor_ref_addr<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    current_addr: &ExecutableAddr,
    object: &serde_json::Map<String, Value>,
) -> Option<TypeAddr> {
    let program = program.into();
    match object.get("kind").and_then(Value::as_str) {
        Some("localType") | Some("local") => {
            return program_json_type_index(object).map(|type_index| TypeAddr {
                unit: current_addr.unit.clone(),
                file: current_addr.file.clone(),
                type_index,
            });
        }
        Some("address") => {
            return object.get("addr").and_then(program_json_type_addr);
        }
        Some("serviceSymbol") => {
            let symbol = object.get("symbol").and_then(program_json_service_symbol)?;
            return program_service_type_addr(program, &symbol);
        }
        Some("packageSymbol") => {
            let symbol = object.get("symbol").and_then(program_json_package_symbol)?;
            return program_package_type_addr(program, &symbol);
        }
        _ => {}
    }

    if object.len() != 1 {
        return None;
    }
    if let Some(addr) = object
        .get("address")
        .or_else(|| object.get("addr"))
        .and_then(program_json_type_addr)
    {
        return Some(addr);
    }
    if let Some(type_index) = object
        .get("local")
        .or_else(|| object.get("localType"))
        .and_then(program_json_usize)
    {
        return Some(TypeAddr {
            unit: current_addr.unit.clone(),
            file: current_addr.file.clone(),
            type_index,
        });
    }
    if let Some(symbol) = object.get("service").and_then(program_json_service_symbol) {
        return program_service_type_addr(program, &symbol);
    }
    if let Some(symbol) = object.get("package").and_then(program_json_package_symbol) {
        return program_package_type_addr(program, &symbol);
    }
    None
}

fn program_service_type_addr(
    program: ProgramTypeView<'_>,
    symbol: &ServiceSymbolRef,
) -> Option<TypeAddr> {
    program
        .types
        .exported_service_type(&symbol.module_path, &symbol.symbol)
        .cloned()
}

pub(crate) fn program_publication_type_addr(
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    module_path: &str,
    type_index: usize,
) -> Option<TypeAddr> {
    let files = match &current_addr.unit {
        UnitAddr::Service => program.service_files,
        UnitAddr::Package(slot) => program.packages.get(*slot)?.files(),
    };
    let file_index = files
        .iter()
        .position(|file| file.module_path == module_path)?;
    Some(TypeAddr {
        unit: current_addr.unit.clone(),
        file: FileAddr::LoadedFileIndex(file_index),
        type_index,
    })
}

pub fn program_package_type_addr<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    symbol: &PackageSymbolRef,
) -> Option<TypeAddr> {
    let program = program.into();
    let resolved = match &symbol.package {
        PackageRefIr::PackageId { package_id } => program
            .link_overlay
            .resolved_package_id_symbol(package_id, &symbol.symbol_path),
        PackageRefIr::Dependency { dependency_ref } => program
            .link_overlay
            .resolved_package_dependency_ref_symbol(dependency_ref, &symbol.symbol_path),
    }?;
    match resolved {
        ResolvedSymbol::Type { addr } => Some(addr.clone()),
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn program_json_type_index(object: &serde_json::Map<String, Value>) -> Option<usize> {
    object
        .get("typeIndex")
        .or_else(|| object.get("type_index"))
        .and_then(program_json_usize)
}

#[cfg(any(test, feature = "test-support"))]
fn program_json_usize(value: &Value) -> Option<usize> {
    value.as_u64().and_then(|index| usize::try_from(index).ok())
}

#[cfg(any(test, feature = "test-support"))]
fn program_json_type_addr(value: &Value) -> Option<TypeAddr> {
    serde_json::from_value(value.clone()).ok()
}

#[cfg(any(test, feature = "test-support"))]
fn program_json_service_symbol(value: &Value) -> Option<ServiceSymbolRef> {
    serde_json::from_value(value.clone()).ok()
}

#[cfg(any(test, feature = "test-support"))]
fn program_json_package_symbol(value: &Value) -> Option<PackageSymbolRef> {
    serde_json::from_value(value.clone()).ok()
}
