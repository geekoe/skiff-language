use std::collections::BTreeMap;

use skiff_artifact_model::{NativeSignatureDef, NativeSignatureTypeExpr};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, LinkedInterfaceInstantiationRef, LinkedTypeDescriptor, LinkedTypeRef,
    NativeTarget, ResolvedSymbol, TypeAddr,
};
use skiff_runtime_model::service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity};
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
    let mut plan = NativeCallPlan::new(spec.key, arg_plans, return_plan, spec.required_context);
    if let Some(owner) = native_named_union_error_owner(spec.key.as_str(), program)? {
        plan = plan
            .with_named_union_error_owner(owner)
            .map_err(RuntimeError::InvalidArtifact)?;
    }

    Ok(Some(plan))
}

const STD_PACKAGE_ID: &str = "skiff.run/std";
const WEBSOCKET_REQUEST_BINDING: &str = "std.websocket.requestJsonToConnection";
const WEBSOCKET_REQUEST_ERROR_TYPE: &str = "std.websocket.WebSocketRequestError";

fn native_named_union_error_owner(
    binding_key: &str,
    program: ProgramTypeView<'_>,
) -> Result<Option<NamedUnionOwnerIdentity>> {
    if binding_key != WEBSOCKET_REQUEST_BINDING {
        return Ok(None);
    }
    let package_slot = program
        .link_overlay
        .package_slot_for_id(STD_PACKAGE_ID)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{WEBSOCKET_REQUEST_BINDING} requires linked package {STD_PACKAGE_ID}"
            ))
        })?;
    let package = program.packages.get(package_slot).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "{WEBSOCKET_REQUEST_BINDING} std package slot {package_slot} is missing linked code"
        ))
    })?;
    if package.package_id() != STD_PACKAGE_ID {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{WEBSOCKET_REQUEST_BINDING} std package slot {package_slot} resolves to {}",
            package.package_id()
        )));
    }
    let addr = program
        .types
        .exported_package_type(package_slot, WEBSOCKET_REQUEST_ERROR_TYPE)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{WEBSOCKET_REQUEST_BINDING} requires public type \
                 {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE}"
            ))
        })?
        .clone();
    match program
        .link_overlay
        .resolved_package_symbol(package_slot, WEBSOCKET_REQUEST_ERROR_TYPE)
    {
        Some(ResolvedSymbol::Type { addr: resolved }) if resolved == &addr => {}
        Some(ResolvedSymbol::Type { .. }) => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{WEBSOCKET_REQUEST_BINDING} linked owner \
                 {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE} is ambiguous across type addresses"
            )))
        }
        Some(symbol) => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{WEBSOCKET_REQUEST_BINDING} linked owner \
                 {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE} has wrong symbol kind {}",
                symbol.export_kind()
            )))
        }
        None => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{WEBSOCKET_REQUEST_BINDING} linked owner \
                 {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE} is missing from the executable symbol overlay"
            )))
        }
    }
    let declaration = program.types.declaration(&addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "{WEBSOCKET_REQUEST_BINDING} linked owner \
             {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE} has no admitted type declaration"
        ))
    })?;
    if !declaration.type_params.is_empty()
        || !matches!(declaration.descriptor, LinkedTypeDescriptor::Union { .. })
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{WEBSOCKET_REQUEST_BINDING} linked owner \
             {STD_PACKAGE_ID}::{WEBSOCKET_REQUEST_ERROR_TYPE} must be an exact non-generic named union"
        )));
    }
    let short_name = WEBSOCKET_REQUEST_ERROR_TYPE
        .rsplit('.')
        .next()
        .expect("canonical type path has a final component");
    if declaration.name != short_name && declaration.name != WEBSOCKET_REQUEST_ERROR_TYPE {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{WEBSOCKET_REQUEST_BINDING} linked owner declaration {} does not match \
             {WEBSOCKET_REQUEST_ERROR_TYPE}",
            declaration.name
        )));
    }
    Ok(Some(NamedUnionOwnerIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr,
            type_arguments: Vec::new(),
        },
    )))
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
        anonymous_type_decl, ExecutableAddr, FileAddr, LinkOverlay, LinkedTypeDescriptor,
        PackageSymbolKey, ResolvedSymbol, RuntimeExecutionPackage, RuntimeTypeContext, TypeAddr,
        UnitAddr,
    };
    use skiff_runtime_model::service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity};

    use crate::type_plan::test_runtime_package;

    use super::{
        native_builtin_plan, native_named_union_error_owner, native_package_type_addr,
        ProgramTypeView, STD_PACKAGE_ID, WEBSOCKET_REQUEST_BINDING, WEBSOCKET_REQUEST_ERROR_TYPE,
    };

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
        packages: &'a [Arc<RuntimeExecutionPackage>],
    ) -> ProgramTypeView<'a> {
        ProgramTypeView::new(&[], packages, overlay, types)
    }

    #[test]
    fn native_signature_package_type_uses_exact_package_id_and_public_path() {
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
        let packages = Vec::new();
        let program = view(&overlay, &types, &packages);

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
    fn native_signature_package_type_fails_closed_for_missing_or_wrong_kind_facts() {
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
        let packages = Vec::new();
        let program = view(&overlay, &types, &packages);

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

    fn std_owner_program(
        addr: TypeAddr,
        symbol: Option<ResolvedSymbol>,
        descriptor: LinkedTypeDescriptor,
    ) -> (
        LinkOverlay,
        RuntimeTypeContext,
        Vec<Arc<RuntimeExecutionPackage>>,
    ) {
        let mut overlay = LinkOverlay {
            package_slots_by_id: [(STD_PACKAGE_ID.to_string(), 0)].into_iter().collect(),
            ..LinkOverlay::default()
        };
        if let Some(symbol) = symbol {
            overlay.symbols.insert_package(
                PackageSymbolKey::new(0, WEBSOCKET_REQUEST_ERROR_TYPE),
                symbol,
            );
        }
        let mut types = RuntimeTypeContext::default();
        types.exported_types.insert_package(
            PackageSymbolKey::new(0, WEBSOCKET_REQUEST_ERROR_TYPE),
            addr.clone(),
        );
        types.descriptors.insert(
            addr,
            anonymous_type_decl("WebSocketRequestError", descriptor),
        );
        let packages = vec![test_runtime_package(0, STD_PACKAGE_ID, Vec::new())];
        (overlay, types, packages)
    }

    #[test]
    fn websocket_request_owner_is_exact_current_linked_std_union() {
        let addr = package_addr(0, 7);
        let (overlay, types, packages) = std_owner_program(
            addr.clone(),
            Some(ResolvedSymbol::Type { addr: addr.clone() }),
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
        );
        let program = view(&overlay, &types, &packages);

        assert_eq!(
            native_named_union_error_owner(WEBSOCKET_REQUEST_BINDING, program).unwrap(),
            Some(NamedUnionOwnerIdentity::LocalExecution(
                LocalExecutionTypeIdentity {
                    addr,
                    type_arguments: Vec::new(),
                }
            ))
        );
        assert_eq!(
            native_named_union_error_owner("std.websocket.sendTextToConnection", program).unwrap(),
            None
        );
    }

    #[test]
    fn websocket_request_owner_fails_closed_before_dispatch_for_bad_link_facts() {
        let exact_addr = package_addr(0, 7);
        let other_addr = package_addr(0, 8);
        for (symbol, descriptor, expected) in [
            (
                None,
                LinkedTypeDescriptor::Union {
                    branches: Vec::new(),
                },
                "missing from the executable symbol overlay",
            ),
            (
                Some(ResolvedSymbol::Executable {
                    addr: ExecutableAddr::package(0, 0, 0),
                }),
                LinkedTypeDescriptor::Union {
                    branches: Vec::new(),
                },
                "wrong symbol kind executable",
            ),
            (
                Some(ResolvedSymbol::Type {
                    addr: other_addr.clone(),
                }),
                LinkedTypeDescriptor::Union {
                    branches: Vec::new(),
                },
                "ambiguous across type addresses",
            ),
            (
                Some(ResolvedSymbol::Type {
                    addr: exact_addr.clone(),
                }),
                LinkedTypeDescriptor::Record {
                    fields: Default::default(),
                },
                "must be an exact non-generic named union",
            ),
        ] {
            let (overlay, types, packages) =
                std_owner_program(exact_addr.clone(), symbol, descriptor);
            let program = view(&overlay, &types, &packages);
            let error =
                native_named_union_error_owner(WEBSOCKET_REQUEST_BINDING, program).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn websocket_request_owner_is_never_reused_across_linked_programs() {
        let left_addr = package_addr(0, 7);
        let right_addr = package_addr(0, 9);
        let (left_overlay, left_types, left_packages) = std_owner_program(
            left_addr.clone(),
            Some(ResolvedSymbol::Type {
                addr: left_addr.clone(),
            }),
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
        );
        let (right_overlay, right_types, right_packages) = std_owner_program(
            right_addr.clone(),
            Some(ResolvedSymbol::Type {
                addr: right_addr.clone(),
            }),
            LinkedTypeDescriptor::Union {
                branches: Vec::new(),
            },
        );
        let left = native_named_union_error_owner(
            WEBSOCKET_REQUEST_BINDING,
            view(&left_overlay, &left_types, &left_packages),
        )
        .unwrap();
        let right = native_named_union_error_owner(
            WEBSOCKET_REQUEST_BINDING,
            view(&right_overlay, &right_types, &right_packages),
        )
        .unwrap();

        assert_ne!(left, right);
    }
}

fn substitute_type_params(
    type_ref: &LinkedTypeRef,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
) -> LinkedTypeRef {
    substitute_type_params_inner(type_ref, substitutions, &mut Vec::new())
}

fn substitute_type_params_inner(
    type_ref: &LinkedTypeRef,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
    resolving: &mut Vec<String>,
) -> LinkedTypeRef {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => {
            let Some(bound) = substitutions.get(name) else {
                return type_ref.clone();
            };
            if resolving.iter().any(|active| active == name) {
                return type_ref.clone();
            }
            resolving.push(name.clone());
            let substituted = substitute_type_params_inner(bound, substitutions, resolving);
            resolving.pop();
            substituted
        }
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_type_params_inner(arg, substitutions, resolving))
                .collect(),
        },
        LinkedTypeRef::AppliedNominal { base, arguments } => LinkedTypeRef::AppliedNominal {
            base: base.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute_type_params_inner(argument, substitutions, resolving))
                .collect(),
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| substitute_type_params_inner(item, substitutions, resolving))
                .collect(),
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(substitute_type_params_inner(
                inner,
                substitutions,
                resolving,
            )),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| substitute_type_params_inner(arg, substitutions, resolving))
                    .collect(),
            },
        },
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        substitute_type_params_inner(ty, substitutions, resolving),
                    )
                })
                .collect(),
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params
                .iter()
                .map(
                    |parameter| skiff_runtime_linked_program::FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: substitute_type_params_inner(&parameter.ty, substitutions, resolving),
                    },
                )
                .collect(),
            return_type: Box::new(substitute_type_params_inner(
                return_type,
                substitutions,
                resolving,
            )),
        },
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::PackageSchema { .. }
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
        LinkedTypeRef::AppliedNominal { arguments, .. } => arguments
            .iter()
            .find_map(|argument| unresolved_type_param_name(argument, allowed_unresolved)),
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
        | LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => None,
    }
}
