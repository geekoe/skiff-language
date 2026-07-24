use std::collections::BTreeMap;

use skiff_artifact_model::{NativeSignatureDef, NativeSignatureTypeExpr};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, LinkedInterfaceInstantiationRef, LinkedTypeRef, NativeTarget,
    ResolvedSymbol, TypeAddr,
};
use skiff_runtime_native_contract::{
    type_arg_key, validate_native_call_arg_count, validate_native_call_type_arg_refs,
    NativeCallPlan, NativeTypeArgRef,
};

use crate::{
    error::{Error as RuntimeError, Result},
    type_plan::{
        native_builtin_plan, PlanContext, ProgramTypeView, RuntimeTypePlan,
        RuntimeTypePlanLinkedExt,
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
    expr: &NativeSignatureTypeExpr,
    type_args: &[ResolvedNativeTypeArg],
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    match expr {
        NativeSignatureTypeExpr::TypeParam(index) => type_args
            .get(*index)
            .map(|arg| arg.plan.clone())
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "native signature references missing T{index}"
                ))
            }),
        NativeSignatureTypeExpr::Builtin(name) => native_builtin_plan(name),
        NativeSignatureTypeExpr::Package {
            package_id,
            public_path,
        } => {
            let addr = native_package_type_addr(program, package_id, public_path)?;
            RuntimeTypePlan::from_linked(
                &LinkedTypeRef::Address { addr },
                &PlanContext::from_type_view(program, current_addr),
            )
        }
        NativeSignatureTypeExpr::Array(item) => {
            let item = resolve_native_type_expr_plan(item, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_array(item))
        }
        NativeSignatureTypeExpr::Map(key, value) => {
            let key = resolve_native_type_expr_plan(key, type_args, program, current_addr)?;
            let value = resolve_native_type_expr_plan(value, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_map(key, value))
        }
        NativeSignatureTypeExpr::Nullable(inner) => {
            let inner = resolve_native_type_expr_plan(inner, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_nullable(inner))
        }
        NativeSignatureTypeExpr::Stream(item) => {
            let item = resolve_native_type_expr_plan(item, type_args, program, current_addr)?;
            Ok(RuntimeTypePlan::synthetic_stream(item))
        }
    }
}

fn native_package_type_addr(
    program: ProgramTypeView<'_>,
    package_id: &str,
    public_path: &str,
) -> Result<TypeAddr> {
    let package_slot = program
        .link_overlay
        .package_slot_for_id(package_id)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "native signature references unknown package {package_id}"
            ))
        })?;
    let addr = program
        .types
        .exported_package_type(package_slot, public_path)
        .ok_or_else(|| {
            let detail = match program
                .link_overlay
                .resolved_package_symbol(package_slot, public_path)
            {
                Some(ResolvedSymbol::Type { .. }) => "is not an admitted linked nominal type",
                Some(_) => "does not name a type",
                None => "is not a public type",
            };
            RuntimeError::InvalidArtifact(format!(
                "native signature package type {package_id}::{public_path} {detail}"
            ))
        })?
        .clone();
    match program
        .link_overlay
        .resolved_package_symbol(package_slot, public_path)
    {
        Some(ResolvedSymbol::Type { addr: resolved }) if resolved == &addr => Ok(addr),
        Some(ResolvedSymbol::Type { .. }) => Err(RuntimeError::InvalidArtifact(format!(
            "native signature package type {package_id}::{public_path} has mismatched linked type addresses"
        ))),
        Some(_) => Err(RuntimeError::InvalidArtifact(format!(
            "native signature package type {package_id}::{public_path} does not name a type"
        ))),
        None => Ok(addr),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use skiff_runtime_linked_program::{
        ExecutableAddr, FileAddr, LinkOverlay, LinkedFileUnit, PackageSymbolKey, PackageUnit,
        ResolvedSymbol, RuntimeTypeContext, TypeAddr, UnitAddr,
    };

    use super::{native_builtin_plan, native_package_type_addr, ProgramTypeView};

    fn package_addr(slot: usize, index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Package(slot),
            file: FileAddr::loaded_file(0),
            type_index: index,
        }
    }

    fn view<'a>(
        overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
        packages: &'a [Arc<PackageUnit>],
    ) -> ProgramTypeView<'a> {
        ProgramTypeView::new(&[], packages, package_files, overlay, types)
    }

    #[test]
    fn package_native_type_uses_exact_package_id_and_public_path() {
        let left_addr = package_addr(0, 0);
        let right_addr = package_addr(1, 0);
        let mut overlay = LinkOverlay {
            package_slots_by_id: [
                ("example.left".to_string(), 0),
                ("example.right".to_string(), 1),
            ]
            .into_iter()
            .collect(),
            ..LinkOverlay::default()
        };
        overlay.symbols.insert_package(
            PackageSymbolKey::new(0, "api.Options"),
            ResolvedSymbol::Type {
                addr: left_addr.clone(),
            },
        );
        overlay.symbols.insert_package(
            PackageSymbolKey::new(1, "api.Options"),
            ResolvedSymbol::Type {
                addr: right_addr.clone(),
            },
        );
        let mut types = RuntimeTypeContext::default();
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(0, "api.Options"), left_addr.clone());
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(1, "api.Options"), right_addr.clone());
        let package_files = vec![Vec::new(), Vec::new()];
        let packages = Vec::new();
        let program = view(&overlay, &types, &package_files, &packages);

        assert_eq!(
            native_package_type_addr(program, "example.left", "api.Options").unwrap(),
            left_addr
        );
        assert_eq!(
            native_package_type_addr(program, "example.right", "api.Options").unwrap(),
            right_addr
        );
    }

    #[test]
    fn package_native_type_fails_closed_for_missing_or_wrong_kind_facts() {
        let addr = package_addr(0, 0);
        let mut overlay = LinkOverlay {
            package_slots_by_id: [("example.pkg".to_string(), 0)].into_iter().collect(),
            ..LinkOverlay::default()
        };
        overlay.symbols.insert_package(
            PackageSymbolKey::new(0, "api.NotAType"),
            ResolvedSymbol::Executable {
                addr: ExecutableAddr::package(0, 0, 0),
            },
        );
        let mut types = RuntimeTypeContext::default();
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(0, "api.NotAType"), addr);
        let package_files = vec![Vec::new()];
        let packages = Vec::new();
        let program = view(&overlay, &types, &package_files, &packages);

        for (package_id, public_path, expected) in [
            ("missing.pkg", "api.NotAType", "unknown package"),
            ("example.pkg", "api.Missing", "is not a public type"),
            ("example.pkg", "api.NotAType", "does not name a type"),
        ] {
            let error = native_package_type_addr(program, package_id, public_path).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn package_public_path_cannot_masquerade_as_builtin() {
        let error = native_builtin_plan("std.file.CreateOptions").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unknown builtin type std.file.CreateOptions"),
            "{error}"
        );
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
