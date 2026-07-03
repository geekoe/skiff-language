use std::collections::BTreeMap;

use skiff_artifact_model::{NativeSignatureDef, NativeTypeExprDef};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, LinkedInterfaceInstantiationRef, LinkedTypeRef, NativeTarget, TypeAddr,
};
use skiff_runtime_native_contract::{
    type_arg_key, validate_native_call_arg_count, validate_native_call_type_arg_refs,
    NativeCallPlan, NativeTypeArgRef,
};

use crate::{
    error::{Error as RuntimeError, Result},
    type_plan::{
        native_builtin_fallback_plan, PlanContext, ProgramTypeView, RuntimeTypeNode,
        RuntimeTypePlan, RuntimeTypePlanLinkedExt,
    },
};

pub use skiff_runtime_native_contract::{NativeCallValidation, NativeSignatureRegistry};

pub fn resolve_call_plan_with_registry<'a>(
    registry: &NativeSignatureRegistry,
    binding_key: &str,
    diagnostic_target: &str,
    call: &CallIr,
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
) -> Result<Option<NativeCallPlan>> {
    let Some(spec) = registry.binding_spec(binding_key) else {
        return Ok(None);
    };
    let signature = spec.signature;
    validate_native_call_arg_count(signature, call.args.len()).map_err(|message| {
        RuntimeError::InvalidArtifact(format!("{diagnostic_target} call {message}"))
    })?;
    if let Some(message) = validate_native_call_type_arg_refs(
        signature,
        call.type_args
            .keys()
            .map(|key| NativeTypeArgRef::new(key.as_str(), None)),
    ) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} call {message}"
        )));
    }

    let resolved_type_args = resolve_native_type_args(
        signature,
        &call.type_args,
        program,
        current_addr,
        substitutions,
        diagnostic_target,
    )?;
    let arg_plans = signature
        .params
        .iter()
        .map(|expr| resolve_native_type_expr_plan(expr, &resolved_type_args, program, current_addr))
        .collect::<Result<Vec<_>>>()?;
    let return_plan = resolve_native_type_expr_plan(
        &signature.return_type,
        &resolved_type_args,
        program,
        current_addr,
    )?;
    let plan = NativeCallPlan::new(spec.key, arg_plans, return_plan, spec.required_context);

    Ok(Some(plan))
}

#[allow(dead_code)]
pub fn validate_native_call_artifact(
    target: &NativeTarget,
    arg_count: usize,
    type_args: &BTreeMap<String, LinkedTypeRef>,
    enclosing_type_params: &[String],
) -> NativeCallValidation {
    let type_args = type_args.iter().map(|(key, ty)| {
        NativeTypeArgRef::new(
            key.as_str(),
            unresolved_type_param_name(ty, Some(enclosing_type_params)),
        )
    });
    NativeSignatureRegistry::builtins().validate_native_call_artifact(target, arg_count, type_args)
}

pub fn resolve_native_call_plan<'a>(
    binding_key: &str,
    diagnostic_target: &str,
    call: &CallIr,
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
) -> Result<Option<NativeCallPlan>> {
    resolve_call_plan_with_registry(
        &NativeSignatureRegistry::builtins(),
        binding_key,
        diagnostic_target,
        call,
        program,
        current_addr,
        substitutions,
    )
}

pub fn program_call_first_type_arg_plan<'a>(
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    call: &CallIr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
) -> Result<Option<RuntimeTypePlan>> {
    let Some(ty) = call.type_args.values().next() else {
        return Ok(None);
    };
    let plan = RuntimeTypePlan::from_linked(
        ty,
        &PlanContext::with_substitutions_from_type_view(program, current_addr, substitutions),
    )?;
    Ok(Some(plan))
}

pub fn native_signature(binding_key: &str) -> Option<&'static NativeSignatureDef> {
    NativeSignatureRegistry::builtins().signature(binding_key)
}

#[derive(Clone, Debug)]
struct ResolvedNativeTypeArg {
    plan: RuntimeTypePlan,
}

fn resolve_native_type_args<'a>(
    signature: &NativeSignatureDef,
    type_args: &BTreeMap<String, LinkedTypeRef>,
    program: ProgramTypeView<'a>,
    current_addr: &'a ExecutableAddr,
    substitutions: &'a BTreeMap<String, LinkedTypeRef>,
    target: &str,
) -> Result<Vec<ResolvedNativeTypeArg>> {
    let mut resolved_type_args = Vec::with_capacity(signature.type_param_count);
    for index in 0..signature.type_param_count {
        let key = type_arg_key(index);
        let type_ref = type_args.get(&key).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("{target} call is missing typeArgs[{index}]"))
        })?;
        let substituted = substitute_type_params(type_ref, substitutions);
        if let Some(name) = unresolved_type_param_name(&substituted, None) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{target} call has unresolved typeArgs[{index}] {name}"
            )));
        }
        let plan = RuntimeTypePlan::from_linked(
            type_ref,
            &PlanContext::with_substitutions_from_type_view(program, current_addr, substitutions),
        )?;
        resolved_type_args.push(ResolvedNativeTypeArg { plan });
    }
    Ok(resolved_type_args)
}

fn resolve_native_type_expr_plan(
    expr: &NativeTypeExprDef,
    type_args: &[ResolvedNativeTypeArg],
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    match expr {
        NativeTypeExprDef::TypeParam(index) => type_args
            .get(*index)
            .map(|arg| arg.plan.clone())
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "native signature references missing T{index}"
                ))
            }),
        NativeTypeExprDef::Builtin(name) => {
            if let Some(addr) = std_package_type_addr(program, name) {
                return RuntimeTypePlan::from_linked(
                    &LinkedTypeRef::Address { addr },
                    &PlanContext::from_type_view(program, current_addr),
                );
            }
            native_builtin_fallback_plan(name)
        }
        NativeTypeExprDef::Array(item) => {
            let item = resolve_native_type_expr_plan(item, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_array(item))
        }
        NativeTypeExprDef::Map(key, value) => {
            let key = resolve_native_type_expr_plan(key, type_args, program, current_addr)?;
            let value = resolve_native_type_expr_plan(value, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_map(key, value))
        }
        NativeTypeExprDef::Nullable(inner) => {
            let inner = resolve_native_type_expr_plan(inner, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_nullable(inner))
        }
        NativeTypeExprDef::Stream(item) => {
            let item = resolve_native_type_expr_plan(item, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_stream(item))
        }
        NativeTypeExprDef::ActorRef(item) => {
            let item = resolve_native_type_expr_plan(item, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_named_builtin(
                "ActorRef",
                RuntimeTypeNode::Unknown,
                vec![item],
            ))
        }
    }
}

fn substitute_type_params(
    type_ref: &LinkedTypeRef,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
) -> LinkedTypeRef {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_type_params(arg, substitutions))
                .collect(),
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| substitute_type_params(item, substitutions))
                .collect(),
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(substitute_type_params(inner, substitutions)),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| substitute_type_params(arg, substitutions))
                    .collect(),
            },
        },
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), ty.clone()))
                .collect(),
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params.clone(),
            return_type: return_type.clone(),
        },
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => type_ref.clone(),
    }
}

fn unresolved_type_param_name<'a>(
    type_ref: &'a LinkedTypeRef,
    allowed_unresolved: Option<&[String]>,
) -> Option<&'a str> {
    match type_ref {
        LinkedTypeRef::TypeParam { name }
            if allowed_unresolved
                .is_some_and(|allowed| allowed.iter().any(|item| item == name)) =>
        {
            None
        }
        LinkedTypeRef::TypeParam { name } => Some(name.as_str()),
        LinkedTypeRef::Native { args, .. } => args
            .iter()
            .find_map(|arg| unresolved_type_param_name(arg, allowed_unresolved)),
        LinkedTypeRef::Union { items } => items
            .iter()
            .find_map(|item| unresolved_type_param_name(item, allowed_unresolved)),
        LinkedTypeRef::Nullable { inner } => unresolved_type_param_name(inner, allowed_unresolved),
        LinkedTypeRef::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .find_map(|arg| unresolved_type_param_name(arg, allowed_unresolved)),
        // Keep record/function/stored replacements fail-closed like the existing
        // runtime JSON substitution path: substitutions are cloned once and not
        // recursively applied inside these shapes.
        LinkedTypeRef::Record { fields } => fields
            .values()
            .find_map(|field| unresolved_type_param_name(field, allowed_unresolved)),
        LinkedTypeRef::Function {
            params,
            return_type,
        } => params
            .iter()
            .find_map(|param| unresolved_type_param_name(&param.ty, allowed_unresolved))
            .or_else(|| unresolved_type_param_name(return_type, allowed_unresolved)),
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => None,
    }
}

fn std_package_type_addr(program: ProgramTypeView<'_>, name: &str) -> Option<TypeAddr> {
    let name = name.trim();
    if name == "Duration" {
        return std_duration_type_addr(program);
    }
    if !name.starts_with("std.") {
        return None;
    }
    std_package_exported_type_addr(program, name).or_else(|| {
        name.strip_prefix("std.")
            .and_then(|symbol_path| std_package_exported_type_addr(program, symbol_path))
    })
}

fn std_duration_type_addr(program: ProgramTypeView<'_>) -> Option<TypeAddr> {
    std_package_exported_type_addr(program, "std.time.Duration")
        .or_else(|| std_package_exported_type_addr(program, "time.Duration"))
}

fn std_package_exported_type_addr(
    program: ProgramTypeView<'_>,
    symbol_path: &str,
) -> Option<TypeAddr> {
    let package_slot = program
        .link_overlay
        .package_slot_for_id("skiff.run/std")
        .or_else(|| program.link_overlay.package_slot_for_dependency_ref("std"))?;
    program
        .types
        .exported_package_type(package_slot, symbol_path)
        .cloned()
}
