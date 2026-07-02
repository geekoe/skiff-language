#[cfg(any(test, feature = "test-support"))]
use serde_json::Value;
use skiff_runtime_linked_program::{
    ExecutableAddr, FunctionTypeParamIr, LinkedExecutable, LinkedInterfaceInstantiationRef,
    LinkedTypeRef, PackageRefIr, PackageSymbolRef, ResolvedSymbol, ServiceSymbolRef, TypeAddr,
};
#[cfg(any(test, feature = "test-support"))]
use skiff_runtime_linked_program::{type_descriptor_to_value, type_ref_to_value};
use skiff_runtime_linked_type_plan::ProgramTypeView;

pub use skiff_runtime_linked_program::executable_type_param_names;

use super::type_descriptor::TypeSubstitutions;
#[cfg(any(test, feature = "test-support"))]
use super::type_descriptor::substitute_type_descriptor;
use super::Interpreter;
#[cfg(any(test, feature = "test-support"))]
use crate::error::{Result, RuntimeError};

pub fn program_type_ref_kind(type_ref: &LinkedTypeRef) -> &'static str {
    match type_ref {
        LinkedTypeRef::LocalType { .. } => "localType",
        LinkedTypeRef::ServiceSymbol { .. } => "serviceSymbol",
        LinkedTypeRef::PackageSymbol { .. } => "packageSymbol",
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
            | LinkedTypeRef::Record { .. }
            | LinkedTypeRef::Union { .. }
            | LinkedTypeRef::Nullable { .. }
            | LinkedTypeRef::DbObjectSymbol { .. }
            | LinkedTypeRef::Literal { .. }
            | LinkedTypeRef::TypeParam { .. }
            | LinkedTypeRef::Function { .. }
            | LinkedTypeRef::AnyInterface { .. } => Ok(Some(type_ref_to_value(type_ref))),
            LinkedTypeRef::LocalType { .. }
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
        LinkedTypeRef::ServiceSymbol { symbol } => program_service_type_addr(program, symbol)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::PackageSymbol { symbol } => program_package_type_addr(program, symbol)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
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
        LinkedTypeRef::ServiceSymbol { symbol } => Some(symbol.symbol_path()),
        LinkedTypeRef::PackageSymbol { symbol } => Some(symbol.symbol_path.clone()),
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

/// Characterization-test oracle for the type-descriptor pipeline.
///
/// This module pins down the CURRENT behavior of
/// `LinkedTypeRef --program_type_descriptor--> resolved Value --from_descriptor--> RuntimeTypePlan`.
///
/// It exists so a future `RuntimeTypePlan::from_linked` (which constructs the
/// plan directly from `LinkedTypeRef`, bypassing the JSON `serde_json::Value`
/// round-trip) can be proven structurally equivalent to today's behavior.
///
/// The oracle asserts on the `{:?}` debug string of the resulting
/// `RuntimeTypePlan` because the plan derives `Debug` but not `PartialEq`, and
/// direct linked construction historically needed to match that shape. Resolved
/// declaration tests compare a metadata-free shape instead: the linked path now
/// deliberately preserves declaration nominal metadata that the JSON pipeline
/// drops.
#[cfg(all(test, any()))]
mod from_linked_oracle {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate as runtime_root;
    use serde_json::Number;
    use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
    use skiff_runtime_model::type_plan::{RuntimeTypeNode, RuntimeTypePlan};

    use super::*;
    use crate::program::{
        anonymous_type_decl, ExecutableAddr, ExecutableKind, FileAddr, LinkedFileUnit,
        LinkedTypeDescriptor, LinkedTypeRef, LiteralIr, PackageRefIr, PackageSymbolRef, ParamIr,
        ResolvedSymbol, RuntimeProgram, RuntimeTypeContext, ServiceMeta, ServiceSymbolRef,
        SlotLayoutIr, TypeAddr, UnitAddr,
    };
    use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};

    /// Builds a minimal single-service-file `RuntimeProgram`. Callers mutate
    /// `program.types` / `program.link_overlay` to install fixtures for the
    /// resolution kinds under test.
    fn empty_program() -> RuntimeProgram {
        RuntimeProgram {
            service: ServiceMeta {
                id: "svc".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: "v1".to_string(),
            build_id: "build:program".to_string(),
            service_files: vec![Arc::new(LinkedFileUnit {
                schema_version: "skiff-file-ir-v3".to_string(),
                file_ir_identity: "file:svc".to_string(),
                source_ast_hash: "source:svc".to_string(),
                module_path: "svc.main".to_string(),
                ir_format_version: None,
                opcode_table_version: None,
                source_map: Default::default(),
                declarations: Default::default(),
                link_targets: Default::default(),
                types: Vec::new(),
                constants: Vec::new(),
                executables: Vec::new(),
                external_refs: Default::default(),
            })],
            packages: Vec::new(),
            package_files: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            routes: Default::default(),
            spawn_routes: Default::default(),
            operations: Default::default(),
            operation_receivers: Default::default(),
            db: Vec::new(),
            actors: Vec::new(),
            link_overlay: Default::default(),
            gateway: Default::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    /// Type address in the current service unit/file (file index 0).
    fn service_type_addr(type_index: usize) -> TypeAddr {
        service_file_type_addr(0, type_index)
    }

    fn service_file_type_addr(file_index: usize, type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(file_index),
            type_index,
        }
    }

    /// Interns `descriptor` at `service_type_addr(type_index)` in `program.types`.
    fn intern_service_type(
        program: &mut RuntimeProgram,
        type_index: usize,
        descriptor: LinkedTypeDescriptor,
    ) {
        intern_type_at(program, service_type_addr(type_index), descriptor);
    }

    fn intern_type_at(
        program: &mut RuntimeProgram,
        addr: TypeAddr,
        descriptor: LinkedTypeDescriptor,
    ) {
        program
            .types
            .descriptors
            .insert(addr, anonymous_type_decl("<test>", descriptor));
    }

    fn builtin(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn builtin_generic(name: &str, args: Vec<LinkedTypeRef>) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args,
        }
    }

    /// Runs the reference pipeline:
    /// `LinkedTypeRef --program_type_descriptor--> resolved Value --from_descriptor--> RuntimeTypePlan`
    /// and returns the resulting plan.
    fn oracle_plan(program: &RuntimeProgram, ty: &LinkedTypeRef) -> RuntimeTypePlan {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let descriptor = interpreter
            .program_type_descriptor(&image, &addr, Some(ty))
            .expect("program_type_descriptor should succeed")
            .expect("type ref should produce a descriptor");
        RuntimeTypePlan::from_descriptor(&descriptor).expect("from_descriptor should succeed")
    }

    /// Runs the reference pipeline:
    /// `LinkedTypeRef --program_type_descriptor--> resolved Value --from_descriptor--> RuntimeTypePlan`
    /// and returns the resulting plan's `{:?}` debug string.
    fn oracle_debug(program: &RuntimeProgram, ty: &LinkedTypeRef) -> String {
        let plan = oracle_plan(program, ty);
        format!("{plan:?}")
    }

    /// Runs the Step-1 native skeleton:
    /// `LinkedTypeRef --RuntimeTypePlan::from_linked--> RuntimeTypePlan`
    /// and returns the resulting plan.
    fn from_linked_plan(program: &RuntimeProgram, ty: &LinkedTypeRef) -> RuntimeTypePlan {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let ctx = PlanContext::new(&image, &addr);
        RuntimeTypePlan::from_linked(ty, &ctx).expect("from_linked should succeed")
    }

    /// Runs the Step-1 native skeleton:
    /// `LinkedTypeRef --RuntimeTypePlan::from_linked--> RuntimeTypePlan`
    /// and returns the resulting plan's `{:?}` debug string.
    fn from_linked_debug(program: &RuntimeProgram, ty: &LinkedTypeRef) -> String {
        let plan = from_linked_plan(program, ty);
        format!("{plan:?}")
    }

    /// Asserts the Step-1 native `from_linked` produces the SAME debug string as
    /// the reference JSON pipeline for the given (natively-handled) type ref.
    fn assert_from_linked_matches_oracle(program: &RuntimeProgram, ty: &LinkedTypeRef) {
        assert_eq!(
            from_linked_debug(program, ty),
            oracle_debug(program, ty),
            "from_linked must match the reference pipeline for {ty:?}"
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    struct PlanFieldShape {
        name: String,
        required: bool,
        ty: PlanShape,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum PlanShape {
        Alias(Box<PlanShape>),
        Nullable(Box<PlanShape>),
        Union(Vec<PlanShape>),
        LiteralString(String),
        Representation {
            type_name: String,
            payload: Box<PlanShape>,
        },
        Json,
        JsonObject,
        Bytes,
        Date,
        String,
        Bool,
        Number,
        Integer,
        Null,
        Stream(Box<PlanShape>),
        Array(Box<PlanShape>),
        Map {
            key: Box<PlanShape>,
            value: Box<PlanShape>,
        },
        Record {
            fields: Vec<PlanFieldShape>,
        },
        Unknown,
    }

    fn plan_shape(plan: &RuntimeTypePlan) -> PlanShape {
        match plan.node() {
            RuntimeTypeNode::Alias(inner) => PlanShape::Alias(Box::new(plan_shape(inner))),
            RuntimeTypeNode::Nullable(inner) => PlanShape::Nullable(Box::new(plan_shape(inner))),
            RuntimeTypeNode::Union(items) => {
                PlanShape::Union(items.iter().map(plan_shape).collect())
            }
            RuntimeTypeNode::LiteralString(value) => PlanShape::LiteralString(value.clone()),
            RuntimeTypeNode::Representation { type_name, payload } => PlanShape::Representation {
                type_name: type_name.clone(),
                payload: Box::new(plan_shape(payload)),
            },
            RuntimeTypeNode::Json => PlanShape::Json,
            RuntimeTypeNode::JsonObject => PlanShape::JsonObject,
            RuntimeTypeNode::Bytes => PlanShape::Bytes,
            RuntimeTypeNode::Date => PlanShape::Date,
            RuntimeTypeNode::String => PlanShape::String,
            RuntimeTypeNode::Bool => PlanShape::Bool,
            RuntimeTypeNode::Number => PlanShape::Number,
            RuntimeTypeNode::Integer => PlanShape::Integer,
            RuntimeTypeNode::Null => PlanShape::Null,
            RuntimeTypeNode::Stream(inner) => PlanShape::Stream(Box::new(plan_shape(inner))),
            RuntimeTypeNode::Array(inner) => PlanShape::Array(Box::new(plan_shape(inner))),
            RuntimeTypeNode::Map { key, value } => PlanShape::Map {
                key: Box::new(plan_shape(key)),
                value: Box::new(plan_shape(value)),
            },
            RuntimeTypeNode::Record { fields, .. } => PlanShape::Record {
                fields: fields
                    .iter()
                    .map(|field| PlanFieldShape {
                        name: field.name.clone(),
                        required: field.required,
                        ty: plan_shape(&field.ty),
                    })
                    .collect(),
            },
            RuntimeTypeNode::Unknown => PlanShape::Unknown,
        }
    }

    /// Resolved declarations are shape-equivalent to the JSON oracle, but the
    /// linked path intentionally keeps declaration names and record boundaries.
    fn assert_from_linked_shape_matches_oracle(
        program: &RuntimeProgram,
        ty: &LinkedTypeRef,
    ) -> RuntimeTypePlan {
        let from_linked = from_linked_plan(program, ty);
        let oracle = oracle_plan(program, ty);
        assert_eq!(
            plan_shape(&from_linked),
            plan_shape(&oracle),
            "from_linked must match the reference shape for {ty:?}"
        );
        from_linked
    }

    fn assert_declaration_name(plan: &RuntimeTypePlan, expected_name: &str) {
        assert_eq!(plan.named_type_name(), Some(expected_name));
    }

    fn assert_record_declaration_metadata(plan: &RuntimeTypePlan, expected_name: &str) {
        assert_declaration_name(plan, expected_name);
        assert_eq!(plan.boundary_record_kind(), Some(expected_name));
    }

    fn record_field<'a>(plan: &'a RuntimeTypePlan, name: &str) -> &'a RuntimeTypePlan {
        let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
            panic!("expected record plan while looking for field {name}: {plan:?}");
        };
        fields
            .iter()
            .find(|field| field.name.as_str() == name)
            .map(|field| &field.ty)
            .unwrap_or_else(|| panic!("expected record field {name}: {plan:?}"))
    }

    /// Runs the reference top-level dispatch (`program_type_descriptor`) and
    /// returns its error string, asserting it errors.
    fn oracle_error(program: &RuntimeProgram, ty: &LinkedTypeRef) -> String {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        interpreter
            .program_type_descriptor(&image, &addr, Some(ty))
            .expect_err("reference pipeline must error at top level")
            .to_string()
    }

    /// Runs `from_linked` and returns its error string, asserting it errors.
    fn from_linked_error(program: &RuntimeProgram, ty: &LinkedTypeRef) -> String {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let ctx = PlanContext::new(&image, &addr);
        RuntimeTypePlan::from_linked(ty, &ctx)
            .expect_err("from_linked must error at top level")
            .to_string()
    }

    /// Asserts `from_linked` errors with the SAME message as the reference
    /// top-level dispatch (the top-level "was not linked" asymmetry).
    fn assert_from_linked_error_matches_oracle(program: &RuntimeProgram, ty: &LinkedTypeRef) {
        assert_eq!(
            from_linked_error(program, ty),
            oracle_error(program, ty),
            "from_linked must error identically to the reference for {ty:?}"
        );
    }

    // --- from_linked native-arm equivalence (Step 1) ------------------------

    #[test]
    fn from_linked_matches_oracle_for_builtin_leaves() {
        let program = empty_program();
        for name in [
            "string",
            "number",
            "integer",
            "bool",
            "bytes",
            "Json",
            "JsonObject",
            "null",
        ] {
            assert_from_linked_matches_oracle(&program, &builtin(name));
        }
    }

    #[test]
    fn from_linked_matches_oracle_for_array() {
        let program = empty_program();
        assert_from_linked_matches_oracle(
            &program,
            &builtin_generic("Array", vec![builtin("string")]),
        );
    }

    #[test]
    fn from_linked_matches_oracle_for_map() {
        let program = empty_program();
        assert_from_linked_matches_oracle(
            &program,
            &builtin_generic("Map", vec![builtin("string"), builtin("number")]),
        );
    }

    #[test]
    fn from_linked_matches_oracle_for_inline_record() {
        let program = empty_program();
        let ty = LinkedTypeRef::Record {
            fields: BTreeMap::from([
                ("name".to_string(), builtin("string")),
                (
                    "age".to_string(),
                    LinkedTypeRef::Nullable {
                        inner: Box::new(builtin("number")),
                    },
                ),
            ]),
        };
        assert_from_linked_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_matches_oracle_for_union() {
        let program = empty_program();
        let ty = LinkedTypeRef::Union {
            items: vec![builtin("string"), builtin("number")],
        };
        assert_from_linked_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_matches_oracle_for_nullable() {
        let program = empty_program();
        let ty = LinkedTypeRef::Nullable {
            inner: Box::new(builtin("string")),
        };
        assert_from_linked_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_matches_oracle_for_literal_string() {
        let program = empty_program();
        let ty = LinkedTypeRef::Literal {
            value: LiteralIr::String {
                value: "hello".to_string(),
            },
        };
        assert_from_linked_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_matches_oracle_for_literal_number_unknown() {
        let program = empty_program();
        let ty = LinkedTypeRef::Literal {
            value: LiteralIr::Number {
                value: Number::from(7),
            },
        };
        assert_from_linked_matches_oracle(&program, &ty);
    }

    // --- Builtin leaf types -------------------------------------------------

    #[test]
    fn builtin_string() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("string")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }"
        );
    }

    #[test]
    fn builtin_number() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("number")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"number\"), node: Number }"
        );
    }

    #[test]
    fn builtin_integer() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("integer")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"integer\"), node: Integer }"
        );
    }

    #[test]
    fn builtin_bool() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("bool")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"bool\"), node: Bool }"
        );
    }

    #[test]
    fn builtin_bytes() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("bytes")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"bytes\"), node: Bytes }"
        );
    }

    #[test]
    fn builtin_json() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("Json")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"Json\"), node: Json }"
        );
    }

    #[test]
    fn builtin_json_object() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("JsonObject")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"JsonObject\"), node: JsonObject }"
        );
    }

    #[test]
    fn builtin_null() {
        let program = empty_program();
        assert_eq!(
            oracle_debug(&program, &builtin("null")),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"null\"), node: Null }"
        );
    }

    // --- Builtin generics ---------------------------------------------------

    #[test]
    fn builtin_array() {
        let program = empty_program();
        let ty = builtin_generic("Array", vec![builtin("string")]);
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"Array\"), node: Array(RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }) }"
        );
    }

    #[test]
    fn builtin_map() {
        let program = empty_program();
        let ty = builtin_generic("Map", vec![builtin("string"), builtin("number")]);
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"Map\"), node: Map { key: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }, value: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"number\"), node: Number } } }"
        );
    }

    // --- Record (inline) ----------------------------------------------------

    #[test]
    fn inline_record() {
        let program = empty_program();
        let ty = LinkedTypeRef::Record {
            fields: BTreeMap::from([
                ("name".to_string(), builtin("string")),
                (
                    "age".to_string(),
                    LinkedTypeRef::Nullable {
                        inner: Box::new(builtin("number")),
                    },
                ),
            ]),
        };
        assert_eq!(oracle_debug(&program, &ty), ORACLE_INLINE_RECORD);
    }

    // --- Union, Nullable, Literal, Representation ---------------------------

    #[test]
    fn union() {
        let program = empty_program();
        let ty = LinkedTypeRef::Union {
            items: vec![builtin("string"), builtin("number")],
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"union\", named_type_name: None, node: Union([RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }, RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"number\"), node: Number }]) }"
        );
    }

    #[test]
    fn nullable() {
        let program = empty_program();
        let ty = LinkedTypeRef::Nullable {
            inner: Box::new(builtin("string")),
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"nullable\", named_type_name: None, node: Nullable(RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }) }"
        );
    }

    #[test]
    fn literal_string() {
        let program = empty_program();
        let ty = LinkedTypeRef::Literal {
            value: LiteralIr::String {
                value: "hello".to_string(),
            },
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"literal\", named_type_name: None, node: LiteralString(\"hello\") }"
        );
    }

    #[test]
    fn literal_number_falls_through_to_unknown() {
        // Only literal *strings* are recognized by from_descriptor; other
        // literal kinds fall through. Pin that behavior.
        let program = empty_program();
        let ty = LinkedTypeRef::Literal {
            value: LiteralIr::Number {
                value: Number::from(7),
            },
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"literal\", named_type_name: None, node: Unknown }"
        );
    }

    #[test]
    fn representation() {
        // `representation` descriptors are not produced by type_ref_to_value;
        // they are synthesized elsewhere. Exercise from_descriptor directly on
        // a representation Value so the node kind is characterized.
        let descriptor = serde_json::json!({
            "kind": "representation",
            "name": "MyRepr",
            "representation": { "kind": "builtin", "name": "string", "args": [] },
        });
        let plan =
            RuntimeTypePlan::from_descriptor(&descriptor).expect("from_descriptor should succeed");
        assert_eq!(
            format!("{plan:?}"),
            "RuntimeTypePlan { label: \"representation\", named_type_name: None, node: Representation { type_name: \"MyRepr\", payload: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String } } }"
        );
    }

    // --- LocalType reference (resolved against program.types) ---------------

    #[test]
    fn local_type_at_top_level_errors_unlinked() {
        // CHARACTERIZATION: a top-level `localType` ref is NOT resolved by
        // program_type_descriptor; it returns an InvalidArtifact error
        // ("was not linked before execution"). LocalType resolution only fires
        // when a localType JSON object appears *nested* inside an already
        // interned descriptor (see `local_type_resolves_when_nested`).
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            3,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("id".to_string(), builtin("string"))]),
            },
        );
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let err = interpreter
            .program_type_descriptor(
                &image,
                &addr,
                Some(&LinkedTypeRef::LocalType { type_index: 3 }),
            )
            .expect_err("top-level localType ref must error");
        assert_eq!(
            err.to_string(),
            "RuntimeProgram type ref localType was not linked before execution"
        );
    }

    #[test]
    fn local_type_resolves_when_nested() {
        // A `localType` ref nested inside an interned descriptor (reached via an
        // Address) IS resolved by resolve_program_descriptor_refs against the
        // current unit's program.types.
        let mut program = empty_program();
        // Target of the localType ref.
        intern_service_type(
            &mut program,
            3,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("id".to_string(), builtin("string"))]),
            },
        );
        // Outer interned descriptor whose field is a localType ref to index 3.
        intern_service_type(
            &mut program,
            4,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "inner".to_string(),
                    LinkedTypeRef::LocalType { type_index: 3 },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(4),
        };
        assert_eq!(oracle_debug(&program, &ty), ORACLE_LOCAL_TYPE_NESTED);
    }

    // --- Address reference --------------------------------------------------

    #[test]
    fn address_reference() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            5,
            LinkedTypeDescriptor::Alias {
                target: builtin("string"),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(5),
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"alias\", named_type_name: None, node: Alias(RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }) }"
        );
    }

    // --- ServiceSymbol reference (resolved via structured service key) ------

    #[test]
    fn service_symbol_at_top_level_errors_unlinked() {
        // CHARACTERIZATION: like localType, a top-level non-http serviceSymbol
        // ref errors. Structured service lookup only fires for serviceSymbol
        // objects nested inside an interned descriptor.
        let program = empty_program();
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let err = interpreter
            .program_type_descriptor(
                &image,
                &addr,
                Some(&LinkedTypeRef::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "svc.main".to_string(),
                        symbol: "MyExported".to_string(),
                    },
                }),
            )
            .expect_err("top-level non-http serviceSymbol ref must error");
        assert_eq!(
            err.to_string(),
            "RuntimeProgram type ref serviceSymbol was not linked before execution"
        );
    }

    #[test]
    fn service_symbol_resolves_when_nested() {
        let mut program = empty_program();
        // Target descriptor for the exported symbol.
        intern_service_type(
            &mut program,
            7,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("ok".to_string(), builtin("bool"))]),
            },
        );
        program.types.exported_types.insert_service(
            crate::program::types::ServiceSymbolKey::new("svc.main", "MyExported"),
            service_type_addr(7),
        );
        // Outer interned descriptor with a nested serviceSymbol field.
        intern_service_type(
            &mut program,
            8,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "ref".to_string(),
                    LinkedTypeRef::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "svc.main".to_string(),
                            symbol: "MyExported".to_string(),
                        },
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(8),
        };
        assert_eq!(oracle_debug(&program, &ty), ORACLE_SERVICE_SYMBOL_NESTED);
    }

    #[test]
    fn service_symbol_resolution_does_not_parse_display_path_fallbacks() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            7,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("ok".to_string(), builtin("bool"))]),
            },
        );
        program.types.exported_types.insert_service(
            crate::program::types::ServiceSymbolKey::new("svc.callee", "compute"),
            service_type_addr(7),
        );

        let current_addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let structured = serde_json::json!({
            "kind": "serviceSymbol",
            "symbol": {
                "modulePath": "svc.callee",
                "symbol": "compute"
            }
        });
        assert_eq!(
            program_descriptor_ref_addr(&image, &current_addr, structured.as_object().unwrap()),
            Some(service_type_addr(7))
        );

        let overlong_module = serde_json::json!({
            "kind": "serviceSymbol",
            "symbol": {
                "modulePath": "svc.callee.compute",
                "symbol": "ignored"
            }
        });
        assert_eq!(
            program_descriptor_ref_addr(
                &image,
                &current_addr,
                overlong_module.as_object().unwrap()
            ),
            None
        );

        let display_path_as_symbol = serde_json::json!({
            "kind": "serviceSymbol",
            "symbol": "svc.callee.compute"
        });
        assert_eq!(
            program_descriptor_ref_addr(
                &image,
                &current_addr,
                display_path_as_symbol.as_object().unwrap()
            ),
            None
        );
    }

    // --- PackageSymbol reference (cross-package via link_overlay) ------------

    #[test]
    fn package_symbol_resolves_when_nested() {
        let mut program = empty_program();
        // Target descriptor that the resolved package TypeAddr points at.
        intern_service_type(
            &mut program,
            9,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("value".to_string(), builtin("number"))]),
            },
        );
        // Map package id -> slot, and install a Type symbol resolution in the
        // overlay keyed by the diagnostic label `package[<slot>]::<symbol>`.
        program
            .link_overlay
            .package_slots_by_id
            .insert("dep-pkg".to_string(), 0);
        program.link_overlay.symbols.insert(
            "package[0]::pkg.mod.Thing".to_string(),
            ResolvedSymbol::Type {
                addr: service_type_addr(9),
            },
        );
        // Outer interned descriptor with a nested packageSymbol field.
        intern_service_type(
            &mut program,
            10,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "ref".to_string(),
                    LinkedTypeRef::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: "dep-pkg".to_string(),
                            },
                            symbol_path: "pkg.mod.Thing".to_string(),
                            abi_expectation: None,
                        },
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(10),
        };
        assert_eq!(oracle_debug(&program, &ty), ORACLE_PACKAGE_SYMBOL_NESTED);
    }

    // --- std-http fallback symbol path --------------------------------------

    #[test]
    fn std_http_service_symbol_errors_unlinked() {
        let program = empty_program();
        let ty = LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "std.http".to_string(),
                symbol: "HttpRequest".to_string(),
            },
        };
        assert!(oracle_error(&program, &ty)
            .contains("RuntimeProgram type ref serviceSymbol was not linked before execution"));
    }

    #[test]
    fn std_http_package_symbol_errors_unlinked() {
        let program = empty_program();
        let ty = LinkedTypeRef::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: "std.http.HttpResponse".to_string(),
                abi_expectation: None,
            },
        };
        assert!(oracle_error(&program, &ty)
            .contains("RuntimeProgram type ref packageSymbol was not linked before execution"));
    }

    // --- Nested / recursive record (references another type by address) -----

    #[test]
    fn nested_record_resolves_addresses_recursively() {
        let mut program = empty_program();
        // Inner type interned at index 1.
        intern_service_type(
            &mut program,
            1,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("leaf".to_string(), builtin("string"))]),
            },
        );
        // Outer record references inner by address.
        let ty = LinkedTypeRef::Record {
            fields: BTreeMap::from([(
                "child".to_string(),
                LinkedTypeRef::Address {
                    addr: service_type_addr(1),
                },
            )]),
        };
        assert_eq!(oracle_debug(&program, &ty), ORACLE_NESTED_RECORD);
    }

    #[test]
    fn self_referential_address_is_bounded_by_depth_guard() {
        // A type whose field references itself by address. The current
        // resolution recurses with a depth guard (>32 returns the value
        // unresolved), so this must terminate rather than overflow.
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            2,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "self_ref".to_string(),
                    LinkedTypeRef::Address {
                        addr: service_type_addr(2),
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(2),
        };
        // Just assert it terminates and produces a Record node (the deep tail
        // bottoms out in Unknown once the depth guard trips).
        let debug = oracle_debug(&program, &ty);
        assert!(
            debug.starts_with("RuntimeTypePlan { label: \"record\""),
            "expected a record plan, got: {debug}"
        );
        assert!(
            debug.contains("Record"),
            "expected nested Record nodes, got: {debug}"
        );
    }

    // --- TypeParam (no substitutions on this pipeline) ----------------------

    #[test]
    fn type_param_without_substitutions_is_unknown() {
        // `program_type_descriptor` does NOT apply TypeSubstitutions (that is a
        // separate path: program_type_descriptor_value_with_substitutions).
        // On the bare pipeline a typeParam descriptor is unrecognized -> Unknown.
        let program = empty_program();
        let ty = LinkedTypeRef::TypeParam {
            name: "T0".to_string(),
        };
        assert_eq!(
            oracle_debug(&program, &ty),
            "RuntimeTypePlan { label: \"typeParam\", named_type_name: None, node: Unknown }"
        );
    }

    // --- Step 2: from_linked native RESOLUTION equivalence ------------------
    //
    // Every kind that from_linked now resolves natively (Address at top level,
    // nested LocalType / ServiceSymbol / PackageSymbol resolution, cycles, the
    // std-http fallback pass-through, and the top-level error asymmetry) is
    // checked against the reference pipeline here. Successful declaration
    // resolution compares structural shape, because the linked path keeps
    // declaration names and record boundary metadata that the JSON descriptor
    // oracle drops.

    #[test]
    fn from_linked_matches_oracle_for_top_level_address_record() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            1,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("name".to_string(), builtin("string")),
                    (
                        "age".to_string(),
                        LinkedTypeRef::Nullable {
                            inner: Box::new(builtin("number")),
                        },
                    ),
                ]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(1),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_record_declaration_metadata(&plan, "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_top_level_address_alias() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            5,
            LinkedTypeDescriptor::Alias {
                target: builtin("string"),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(5),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_declaration_name(&plan, "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_top_level_address_union() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            6,
            LinkedTypeDescriptor::Union {
                variants: vec![builtin("string"), builtin("number")],
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(6),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_declaration_name(&plan, "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_address_to_nested_address_chain() {
        // Top-level Address -> record whose field is an Address -> record. Both
        // ref levels must resolve recursively, matching the reference walk.
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            1,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("leaf".to_string(), builtin("string"))]),
            },
        );
        let outer = LinkedTypeRef::Record {
            fields: BTreeMap::from([(
                "child".to_string(),
                LinkedTypeRef::Address {
                    addr: service_type_addr(1),
                },
            )]),
        };
        // Inline outer record at top level (its child Address resolves nested).
        let plan = assert_from_linked_shape_matches_oracle(&program, &outer);
        assert_record_declaration_metadata(record_field(&plan, "child"), "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_nested_local_type() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            3,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("id".to_string(), builtin("string"))]),
            },
        );
        intern_service_type(
            &mut program,
            4,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "inner".to_string(),
                    LinkedTypeRef::LocalType { type_index: 3 },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(4),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_record_declaration_metadata(&plan, "<test>");
        assert_record_declaration_metadata(record_field(&plan, "inner"), "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_nested_service_symbol() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            7,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("ok".to_string(), builtin("bool"))]),
            },
        );
        program.types.exported_types.insert_service(
            crate::program::types::ServiceSymbolKey::new("svc.main", "MyExported"),
            service_type_addr(7),
        );
        intern_service_type(
            &mut program,
            8,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "ref".to_string(),
                    LinkedTypeRef::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "svc.main".to_string(),
                            symbol: "MyExported".to_string(),
                        },
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(8),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_record_declaration_metadata(&plan, "<test>");
        assert_record_declaration_metadata(record_field(&plan, "ref"), "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_nested_package_symbol() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            9,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("value".to_string(), builtin("number"))]),
            },
        );
        program
            .link_overlay
            .package_slots_by_id
            .insert("dep-pkg".to_string(), 0);
        program.link_overlay.symbols.insert(
            "package[0]::pkg.mod.Thing".to_string(),
            ResolvedSymbol::Type {
                addr: service_type_addr(9),
            },
        );
        intern_service_type(
            &mut program,
            10,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "ref".to_string(),
                    LinkedTypeRef::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: "dep-pkg".to_string(),
                            },
                            symbol_path: "pkg.mod.Thing".to_string(),
                            abi_expectation: None,
                        },
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(10),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_record_declaration_metadata(&plan, "<test>");
        assert_record_declaration_metadata(record_field(&plan, "ref"), "<test>");
    }

    #[test]
    fn from_linked_matches_oracle_for_self_referential_address_cycle() {
        // The depth-32 cap must truncate the cycle at the identical node, so the
        // full debug string (record nesting + the Unknown tail) matches exactly.
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            2,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "self_ref".to_string(),
                    LinkedTypeRef::Address {
                        addr: service_type_addr(2),
                    },
                )]),
            },
        );
        let ty = LinkedTypeRef::Address {
            addr: service_type_addr(2),
        };
        let plan = assert_from_linked_shape_matches_oracle(&program, &ty);
        assert_record_declaration_metadata(&plan, "<test>");
    }

    #[test]
    fn from_linked_errors_like_oracle_for_std_http_service_symbol() {
        let program = empty_program();
        let ty = LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "std.http".to_string(),
                symbol: "HttpRequest".to_string(),
            },
        };
        assert_from_linked_error_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_errors_like_oracle_for_std_http_package_symbol() {
        let program = empty_program();
        let ty = LinkedTypeRef::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: "std.http.HttpResponse".to_string(),
                abi_expectation: None,
            },
        };
        assert_from_linked_error_matches_oracle(&program, &ty);
    }

    #[test]
    fn from_linked_errors_like_oracle_for_top_level_local_type() {
        let mut program = empty_program();
        intern_service_type(
            &mut program,
            3,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("id".to_string(), builtin("string"))]),
            },
        );
        assert_from_linked_error_matches_oracle(
            &program,
            &LinkedTypeRef::LocalType { type_index: 3 },
        );
    }

    #[test]
    fn from_linked_errors_like_oracle_for_top_level_service_symbol() {
        let program = empty_program();
        assert_from_linked_error_matches_oracle(
            &program,
            &LinkedTypeRef::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "svc.main".to_string(),
                    symbol: "MyExported".to_string(),
                },
            },
        );
    }

    #[test]
    fn from_linked_errors_like_oracle_for_top_level_package_symbol() {
        let program = empty_program();
        assert_from_linked_error_matches_oracle(
            &program,
            &LinkedTypeRef::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "dep-pkg".to_string(),
                    },
                    symbol_path: "pkg.mod.Thing".to_string(),
                    abi_expectation: None,
                },
            },
        );
    }

    // --- Step 5: TypeParam substitution equivalence -------------------------
    //
    // These pin `from_linked` + `PlanContext::with_substitutions` (the LINKED
    // domain bindings) against the reference JSON path
    // `program_type_descriptor_value_with_substitutions(..) -> from_descriptor`.
    //
    // The reference substitution entries are the same caller-normalized
    // `LinkedTypeRef`s that `call_type_substitutions` stores. The descriptor
    // fallback materializes them only when a type parameter is replaced.

    /// Builds the structured substitution map the legacy descriptor fallback
    /// consumes. The entries are linked refs, matching `call_type_substitutions`;
    /// `substitute_type_value` materializes them only at the fallback boundary.
    fn type_subs_from_linked(subs: &BTreeMap<String, LinkedTypeRef>) -> TypeSubstitutions {
        let mut out = TypeSubstitutions::new();
        for (name, bound) in subs {
            out.insert(name.clone(), bound.clone());
        }
        out
    }

    /// Reference pipeline WITH substitutions:
    /// `program_type_descriptor_value_with_substitutions -> from_descriptor`.
    fn oracle_debug_with_subs(
        program: &RuntimeProgram,
        ty: &LinkedTypeRef,
        subs: &BTreeMap<String, LinkedTypeRef>,
    ) -> String {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let json_subs = type_subs_from_linked(subs);
        let descriptor = program_type_descriptor_value_with_substitutions(
            &interpreter,
            &image,
            &addr,
            Some(ty),
            &json_subs,
        )
        .expect("reference path should produce a descriptor");
        let plan =
            RuntimeTypePlan::from_descriptor(&descriptor).expect("from_descriptor should succeed");
        format!("{plan:?}")
    }

    /// Native pipeline WITH substitutions:
    /// `from_linked` + `PlanContext::with_substitutions` (LINKED-domain bindings).
    fn from_linked_debug_with_subs(
        program: &RuntimeProgram,
        ty: &LinkedTypeRef,
        subs: &BTreeMap<String, LinkedTypeRef>,
    ) -> String {
        let addr = ExecutableAddr::service(0, 0);
        let image = program.linked_image();
        let ctx = PlanContext::with_substitutions(&image, &addr, subs);
        let plan = RuntimeTypePlan::from_linked(ty, &ctx).expect("from_linked should succeed");
        format!("{plan:?}")
    }

    fn assert_subs_match_oracle(
        program: &RuntimeProgram,
        ty: &LinkedTypeRef,
        subs: &BTreeMap<String, LinkedTypeRef>,
    ) {
        assert_eq!(
            from_linked_debug_with_subs(program, ty, subs),
            oracle_debug_with_subs(program, ty, subs),
            "from_linked+with_substitutions must match the reference path for {ty:?} with {subs:?}"
        );
    }

    fn type_param(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::TypeParam {
            name: name.to_string(),
        }
    }

    fn generic_callee(type_param_name: &str) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "svc.main.generic".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "value".to_string(),
                slot: 0,
                ty: type_param(type_param_name),
            }],
            return_type: Some(type_param(type_param_name)),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: Default::default(),
        }
    }

    #[test]
    fn subs_type_param_bound_to_builtin() {
        let program = empty_program();
        let subs = BTreeMap::from([("T0".to_string(), builtin("string"))]);
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
    }

    #[test]
    fn subs_type_param_bound_to_record() {
        let program = empty_program();
        let record = LinkedTypeRef::Record {
            fields: BTreeMap::from([
                ("name".to_string(), builtin("string")),
                ("age".to_string(), builtin("number")),
            ]),
        };
        let subs = BTreeMap::from([("T0".to_string(), record)]);
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
    }

    #[test]
    fn subs_type_param_bound_to_nullable() {
        let program = empty_program();
        let nullable = LinkedTypeRef::Nullable {
            inner: Box::new(builtin("string")),
        };
        let subs = BTreeMap::from([("T0".to_string(), nullable)]);
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
    }

    #[test]
    fn subs_type_param_unbound_is_unknown() {
        let program = empty_program();
        // Empty bindings: the param is unbound on both paths -> Unknown.
        let subs: BTreeMap<String, LinkedTypeRef> = BTreeMap::new();
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
        // Non-empty bindings that do not cover this param -> still unbound.
        let other = BTreeMap::from([("U".to_string(), builtin("string"))]);
        assert_subs_match_oracle(&program, &type_param("T0"), &other);
    }

    #[test]
    fn subs_generic_container_param_bound() {
        let program = empty_program();
        let array_of_t = builtin_generic("Array", vec![type_param("T0")]);
        let subs = BTreeMap::from([("T0".to_string(), builtin("number"))]);
        assert_subs_match_oracle(&program, &array_of_t, &subs);
    }

    #[test]
    fn subs_record_field_of_param_type() {
        let program = empty_program();
        let record = LinkedTypeRef::Record {
            fields: BTreeMap::from([
                ("value".to_string(), type_param("T0")),
                ("label".to_string(), builtin("string")),
            ]),
        };
        let subs = BTreeMap::from([("T0".to_string(), builtin("number"))]);
        assert_subs_match_oracle(&program, &record, &subs);
        let debug = from_linked_debug_with_subs(&program, &record, &subs);
        assert!(
            debug.contains("node: Unknown"),
            "record field object-map positions should not substitute: {debug}"
        );
        assert!(
            !debug.contains("node: Number"),
            "record field object-map positions must preserve the legacy no-substitution behavior: {debug}"
        );
    }

    #[test]
    fn subs_inline_union_item_of_param_type() {
        let program = empty_program();
        let union = LinkedTypeRef::Union {
            items: vec![type_param("T0"), builtin("string")],
        };
        let subs = BTreeMap::from([("T0".to_string(), builtin("number"))]);
        assert_subs_match_oracle(&program, &union, &subs);
        let debug = from_linked_debug_with_subs(&program, &union, &subs);
        assert!(
            debug.contains("node: Number"),
            "inline union items should substitute: {debug}"
        );
    }

    #[test]
    fn subs_param_bound_to_generic_containing_same_param() {
        // Self-referential-ish: T0 -> Array<T0>. The JSON path substitutes the
        // outer T0 once (cloning the resolved Array<typeParam T0> Value without
        // re-recursing), leaving the inner T0 unresolved -> Unknown. The native
        // path must terminate the same way via param shadowing.
        let program = empty_program();
        let array_of_t = builtin_generic("Array", vec![type_param("T0")]);
        let subs = BTreeMap::from([("T0".to_string(), array_of_t)]);
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
    }

    #[test]
    fn subs_param_bound_to_record_containing_same_param() {
        // T0 -> { next: T0 }. The inner T0 must stay unresolved (Unknown) on both
        // paths, pinning the shadow/termination behavior for records too.
        let program = empty_program();
        let rec_of_t = LinkedTypeRef::Record {
            fields: BTreeMap::from([("next".to_string(), type_param("T0"))]),
        };
        let subs = BTreeMap::from([("T0".to_string(), rec_of_t)]);
        assert_subs_match_oracle(&program, &type_param("T0"), &subs);
    }

    #[test]
    fn subs_nested_param_inside_container_bound_to_param_chain() {
        // Array<T0> with T0 -> Nullable<U>, U -> number. Replacement is cloned
        // once: U inside the T0 replacement is not recursively substituted by
        // the sibling binding, matching the legacy JSON replacement behavior.
        let program = empty_program();
        let array_of_t = builtin_generic("Array", vec![type_param("T0")]);
        let subs = BTreeMap::from([
            (
                "T0".to_string(),
                LinkedTypeRef::Nullable {
                    inner: Box::new(type_param("U")),
                },
            ),
            ("U".to_string(), builtin("number")),
        ]);
        assert_subs_match_oracle(&program, &array_of_t, &subs);
        let debug = from_linked_debug_with_subs(&program, &array_of_t, &subs);
        assert!(
            debug.contains("Nullable(RuntimeTypePlan { label: \"typeParam\", named_type_name: None, node: Unknown })"),
            "sibling binding U must not be applied inside the cloned T0 replacement: {debug}"
        );
        assert!(
            !debug.contains("node: Number"),
            "replacement internals should not recursively see sibling bindings: {debug}"
        );
    }

    #[test]
    fn call_substitutions_resolve_array_nullable_actual_under_caller_subs() {
        let program = empty_program();
        let image = program.linked_image();
        let caller_addr = ExecutableAddr::service(0, 0);
        let callee = generic_callee("U");
        let mut caller_subs = TypeSubstitutions::new();
        caller_subs.insert("T".to_string(), builtin("string"));
        let type_args = BTreeMap::from([(
            "U".to_string(),
            builtin_generic(
                "Array",
                vec![LinkedTypeRef::Nullable {
                    inner: Box::new(type_param("T")),
                }],
            ),
        )]);

        let substitutions =
            call_type_substitutions(&image, &caller_addr, &caller_subs, &callee, &type_args);

        assert_eq!(
            substitutions.get("U"),
            Some(&builtin_generic(
                "Array",
                vec![LinkedTypeRef::Nullable {
                    inner: Box::new(builtin("string")),
                }],
            ))
        );
    }

    #[test]
    fn call_substitutions_do_not_apply_caller_subs_inside_record_actual_fields() {
        let program = empty_program();
        let image = program.linked_image();
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let caller_addr = ExecutableAddr::service(0, 0);
        let callee_addr = ExecutableAddr::service(0, 1);
        let callee = generic_callee("U");
        let mut caller_subs = TypeSubstitutions::new();
        caller_subs.insert("T".to_string(), builtin("string"));
        let record_actual = LinkedTypeRef::Record {
            fields: BTreeMap::from([("value".to_string(), type_param("T"))]),
        };
        let type_args = BTreeMap::from([("U".to_string(), record_actual.clone())]);

        let substitutions =
            call_type_substitutions(&image, &caller_addr, &caller_subs, &callee, &type_args);

        assert_eq!(substitutions.get("U"), Some(&record_actual));
        let descriptor = program_type_descriptor_value_with_substitutions(
            &interpreter,
            &image,
            &callee_addr,
            Some(&type_param("U")),
            &substitutions,
        )
        .expect("legacy descriptor fallback should materialize the record binding");
        let debug = format!(
            "{:?}",
            RuntimeTypePlan::from_descriptor(&descriptor).expect("descriptor should plan")
        );
        assert!(
            debug.contains("node: Unknown"),
            "record field TypeParam should remain unsubstituted after replacement clone: {debug}"
        );
        assert!(
            !debug.contains("node: String"),
            "caller substitution must not leak into record field object-map positions: {debug}"
        );
    }

    #[test]
    fn call_substitutions_normalize_caller_local_type_before_callee_uses_it() {
        let mut program = empty_program();
        let caller_addr = ExecutableAddr::service(0, 0);
        let callee_addr = ExecutableAddr::service(1, 0);
        let caller_type_addr = service_file_type_addr(0, 3);
        let callee_drift_addr = service_file_type_addr(1, 3);
        intern_type_at(
            &mut program,
            caller_type_addr.clone(),
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("caller".to_string(), builtin("string"))]),
            },
        );
        intern_type_at(
            &mut program,
            callee_drift_addr,
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([("callee".to_string(), builtin("number"))]),
            },
        );
        let interpreter = Interpreter::with_program(
            Arc::new(program.clone()),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let image = program.linked_image();
        let callee = generic_callee("T");
        let type_args =
            BTreeMap::from([("T".to_string(), LinkedTypeRef::LocalType { type_index: 3 })]);

        let substitutions = call_type_substitutions(
            &image,
            &caller_addr,
            &TypeSubstitutions::new(),
            &callee,
            &type_args,
        );

        assert_eq!(
            substitutions.get("T"),
            Some(&LinkedTypeRef::Address {
                addr: caller_type_addr
            })
        );
        let ctx =
            PlanContext::with_substitutions(&image, &callee_addr, substitutions.as_linked_map());
        let plan = RuntimeTypePlan::from_linked(&type_param("T"), &ctx)
            .expect("normalized caller local type should resolve from callee");
        let debug = format!("{plan:?}");
        assert!(
            debug.contains("name: \"caller\"") && debug.contains("node: String"),
            "callee should use the caller-origin address binding: {debug}"
        );
        assert!(
            !debug.contains("name: \"callee\""),
            "raw LocalType would drift to the callee file: {debug}"
        );
        let descriptor = program_type_descriptor_value_with_substitutions(
            &interpreter,
            &image,
            &callee_addr,
            Some(&type_param("T")),
            &substitutions,
        )
        .expect("legacy descriptor fallback should materialize the normalized binding");
        let descriptor_text =
            serde_json::to_string(&descriptor).expect("descriptor should serialize");
        assert!(
            descriptor_text.contains("caller") && !descriptor_text.contains("callee"),
            "legacy descriptor fallback should resolve the caller-origin address: {descriptor_text}"
        );
    }

    // Larger expected debug strings, hoisted out for readability.

    const ORACLE_INLINE_RECORD: &str = "RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"age\", ty: RuntimeTypePlan { label: \"nullable\", named_type_name: None, node: Nullable(RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"number\"), node: Number }) }, required: false }, RuntimeRecordFieldPlan { name: \"name\", ty: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }, required: true }], boundary_record_kind: None } }";

    const ORACLE_LOCAL_TYPE_NESTED: &str = "RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"inner\", ty: RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"id\", ty: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }, required: true }], boundary_record_kind: None } }, required: true }], boundary_record_kind: None } }";

    const ORACLE_SERVICE_SYMBOL_NESTED: &str = "RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"ref\", ty: RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"ok\", ty: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"bool\"), node: Bool }, required: true }], boundary_record_kind: None } }, required: true }], boundary_record_kind: None } }";

    const ORACLE_PACKAGE_SYMBOL_NESTED: &str = "RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"ref\", ty: RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"value\", ty: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"number\"), node: Number }, required: true }], boundary_record_kind: None } }, required: true }], boundary_record_kind: None } }";

    const ORACLE_NESTED_RECORD: &str = "RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"child\", ty: RuntimeTypePlan { label: \"record\", named_type_name: None, node: Record { fields: [RuntimeRecordFieldPlan { name: \"leaf\", ty: RuntimeTypePlan { label: \"builtin\", named_type_name: Some(\"string\"), node: String }, required: true }], boundary_record_kind: None } }, required: true }], boundary_record_kind: None } }";
}
