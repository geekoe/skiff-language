use std::collections::BTreeMap;

use skiff_runtime_linked_program::{
    type_ref_to_value, CallIr, ExecutableAddr, FileAddr, LinkedActorDeclaration,
    LinkedActorDeclarationOwner, LinkedInterfaceInstantiationRef, LinkedTypeRef, NativeTarget,
    TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{self as linked_type_plan, ProgramTypeView};
use skiff_runtime_model::type_plan::RuntimeTypePlan;
use skiff_runtime_native::dispatch::{
    runtime_shared_native_route, RuntimeActorNativeMetadata, RuntimeNativeInvocation,
    RuntimeNativeRoute,
};
use skiff_runtime_native_contract::{
    validate_native_call_arg_count, NativeCallPlan, NativeDispatchTarget, NativeSignatureRegistry,
};

use super::{
    env::Env,
    program_types::{program_package_type_addr, program_publication_type_addr},
    Interpreter,
};
use crate::{
    assembly_execution::RuntimeExecutionProjection,
    error::{Result, RuntimeError},
};

pub fn resolve_runtime_native_invocation(
    interpreter: &Interpreter,
    current_addr: &ExecutableAddr,
    env: &Env,
    call: &CallIr,
    target: &NativeTarget,
) -> Result<RuntimeNativeInvocation> {
    let program = interpreter.program_projection()?.type_view();
    resolve_runtime_native_invocation_in_type_view(
        interpreter,
        program,
        current_addr,
        env,
        call,
        target,
    )
}

pub(crate) fn resolve_runtime_execution_native_invocation(
    interpreter: &Interpreter,
    projection: &RuntimeExecutionProjection<'_>,
    current_addr: &ExecutableAddr,
    env: &Env,
    call: &CallIr,
    target: &NativeTarget,
) -> Result<RuntimeNativeInvocation> {
    resolve_runtime_native_invocation_in_type_view(
        interpreter,
        projection.type_view(),
        current_addr,
        env,
        call,
        target,
    )
}

fn resolve_runtime_native_invocation_in_type_view(
    interpreter: &Interpreter,
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    env: &Env,
    call: &CallIr,
    target: &NativeTarget,
) -> Result<RuntimeNativeInvocation> {
    let (target_name, binding_key) =
        match NativeSignatureRegistry::builtins().validate_native_dispatch_target(target) {
            NativeDispatchTarget::Resolved {
                target_name,
                binding_key,
            } => (target_name, binding_key),
            NativeDispatchTarget::MissingExternalBinding { target_name } => {
                return Err(interpreter.native_registry.unsupported(&target_name).into());
            }
            NativeDispatchTarget::Invalid(message) => {
                return Err(RuntimeError::InvalidArtifact(message));
            }
        };
    let actor_metadata = resolve_actor_native_metadata(program, binding_key, &target_name, call)?;
    let resource_owner = (runtime_shared_native_route(binding_key)
        == Some(RuntimeNativeRoute::Resource))
    .then(|| current_addr.unit.clone());
    let plan = match resolve_runtime_native_call_plan(
        program,
        current_addr,
        env,
        call,
        binding_key,
        &target_name,
    ) {
        Ok(plan) => plan,
        Err(error)
            if binding_key == "std.json.encode" && is_unresolved_native_type_arg_error(&error) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    Ok(RuntimeNativeInvocation::new(
        target_name,
        binding_key,
        plan,
        actor_metadata,
        resource_owner,
    ))
}

pub fn resolve_config_builtin_type_arg_plan(
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    type_substitutions: &BTreeMap<String, LinkedTypeRef>,
    call: &CallIr,
    target: &str,
) -> Result<Option<RuntimeTypePlan>> {
    match target {
        "config.require" | "config.optional" => {
            let plan = linked_type_plan::program_call_first_type_arg_plan(
                program,
                current_addr,
                call,
                type_substitutions,
            )?
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!("{target} call is missing typeArgs[0]"))
            })?;
            Ok(Some(plan))
        }
        "config.has" => Ok(None),
        _ => Ok(None),
    }
}

fn resolve_actor_native_metadata(
    program: ProgramTypeView<'_>,
    binding_key: &str,
    diagnostic_target: &str,
    call: &CallIr,
) -> Result<Option<RuntimeActorNativeMetadata>> {
    if runtime_shared_native_route(binding_key) != Some(RuntimeNativeRoute::Actor) {
        return Ok(None);
    }
    validate_actor_native_call_arg_count(binding_key, diagnostic_target, call)?;
    let linked_metadata = call.actor_metadata.as_ref().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} call is missing linked actor declaration metadata"
        ))
    })?;
    let declaration = actor_declaration_for_owner(program, &linked_metadata.declaration_owner)?;
    if declaration.actor_abi_identity != linked_metadata.actor_abi_identity {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} actor declaration ABI identity does not match call metadata"
        )));
    }
    let expected_actor_type = LinkedTypeRef::ServiceSymbol {
        symbol: declaration.actor_type.clone(),
    };
    if call.type_args.get("T0") != Some(&expected_actor_type) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} actor typeArgs[0] does not match linked actor declaration"
        )));
    }
    if call.type_args.get("T1") != Some(&declaration.actor_id_type) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} actor typeArgs[1] does not match linked actor id declaration"
        )));
    }
    if matches!(binding_key, "std.actor.getOrCreate" | "std.actor.replace") {
        let expected_bootstrap = LinkedTypeRef::Record {
            fields: declaration
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        };
        if call.type_args.get("T2") != Some(&expected_bootstrap) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{diagnostic_target} actor typeArgs[2] does not match linked bootstrap declaration"
            )));
        }
    } else if call.type_args.contains_key("T2") {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} actor call must not carry bootstrap typeArgs[2]"
        )));
    }
    Ok(Some(RuntimeActorNativeMetadata::new(
        type_identity(&expected_actor_type)?,
        type_identity(&declaration.actor_id_type)?,
        declaration.actor_abi_identity.as_str().to_string(),
    )))
}

fn actor_declaration_for_owner<'a>(
    program: ProgramTypeView<'a>,
    owner: &LinkedActorDeclarationOwner,
) -> Result<&'a LinkedActorDeclaration> {
    let files = match &owner.unit {
        UnitAddr::Service => program.service_files,
        UnitAddr::Package(slot) => program.package_files.get(*slot).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "actor declaration owner package slot {slot} is not loaded"
            ))
        })?,
    };
    let file = match &owner.file {
        FileAddr::LoadedFileIndex(index) => files.get(*index),
        FileAddr::FileIrIdentity(identity) => {
            files.iter().find(|file| file.file_ir_identity == *identity)
        }
    }
    .ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "actor declaration owner file {:?} is not loaded",
            owner.file
        ))
    })?;
    let mut matches = file.actor_declarations.iter().filter(|declaration| {
        declaration.implementation_owner.as_ref() == Some(owner)
            && declaration.actor_type.symbol == owner.actor_symbol
    });
    let declaration = matches.next().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "actor declaration owner {:?} does not resolve to an actor declaration",
            owner
        ))
    })?;
    if matches.next().is_some() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "actor declaration owner {:?} resolves ambiguously",
            owner
        )));
    }
    Ok(declaration)
}

#[cfg(test)]
mod actor_declaration_resolution_tests {
    use std::sync::Arc;

    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ACTOR_RUNTIME_ABI_VERSION_V1,
    };
    use skiff_runtime_linked_program::{
        ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorDeclaration,
        LinkedActorDeclarationOwner, LinkedFileUnit, RuntimeTypeContext, ServiceSymbolRef,
        SourceMapDto,
    };

    use super::*;

    fn actor_file(owner: &LinkedActorDeclarationOwner) -> LinkedFileUnit {
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:actors".to_string(),
            source_ast_hash: "source:actors".to_string(),
            module_path: "actors".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: vec![LinkedActorDeclaration {
                actor_type: ServiceSymbolRef {
                    module_path: "actors".to_string(),
                    symbol: "DocHub".to_string(),
                },
                implementation_owner: Some(owner.clone()),
                actor_abi_identity: ActorAbiIdentity::new("actor-abi:doc-hub"),
                actor_implementation_identity: ActorImplementationIdentity::new(
                    "actor-implementation:doc-hub",
                ),
                actor_name: "DocHub".to_string(),
                actor_id_type: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
                fields: Vec::new(),
                public_methods: Vec::new(),
                actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
            }],
            types: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        }
    }

    #[test]
    fn actor_declaration_owner_resolves_exact_loaded_file_and_symbol() {
        let owner = LinkedActorDeclarationOwner {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            actor_symbol: "DocHub".to_string(),
        };
        let package_files = vec![vec![Arc::new(actor_file(&owner))]];
        let packages = Vec::new();
        let service_files = Vec::new();
        let overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let program =
            ProgramTypeView::new(&service_files, &packages, &package_files, &overlay, &types);

        let declaration =
            actor_declaration_for_owner(program, &owner).expect("exact owner resolves");
        assert_eq!(declaration.actor_name, "DocHub");

        let forged = LinkedActorDeclarationOwner {
            actor_symbol: "Other".to_string(),
            ..owner
        };
        assert!(actor_declaration_for_owner(program, &forged).is_err());
    }
}

fn validate_actor_native_call_arg_count(
    binding_key: &str,
    diagnostic_target: &str,
    call: &CallIr,
) -> Result<()> {
    let Some(spec) = NativeSignatureRegistry::builtins().binding_spec(binding_key) else {
        return Ok(());
    };
    validate_native_call_arg_count(spec.signature, call.args.len()).map_err(|message| {
        RuntimeError::InvalidArtifact(format!("{diagnostic_target} call {message}"))
    })
}

fn type_identity(type_ref: &LinkedTypeRef) -> Result<String> {
    serde_json::to_string(&type_ref_to_value(type_ref)).map_err(RuntimeError::from)
}

fn resolve_runtime_native_call_plan(
    program: ProgramTypeView<'_>,
    current_addr: &ExecutableAddr,
    env: &Env,
    call: &CallIr,
    binding_key: &str,
    diagnostic_target: &str,
) -> Result<Option<NativeCallPlan>> {
    let mut substitutions = BTreeMap::new();
    for (name, ty) in env.type_substitutions.as_linked_map() {
        substitutions.insert(
            name.clone(),
            normalize_native_signature_type_arg(program, current_addr, ty),
        );
    }
    Ok(linked_type_plan::resolve_native_call_plan(
        binding_key,
        diagnostic_target,
        call,
        program,
        current_addr,
        &substitutions,
    )?)
}

fn is_unresolved_native_type_arg_error(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::InvalidArtifact(message) if message.contains("unresolved typeArgs[")
    )
}

fn normalize_native_signature_type_arg<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    current_addr: &ExecutableAddr,
    type_ref: &LinkedTypeRef,
) -> LinkedTypeRef {
    let program = program.into();
    match type_ref {
        LinkedTypeRef::LocalType { type_index } => LinkedTypeRef::Address {
            addr: TypeAddr {
                unit: current_addr.unit.clone(),
                file: current_addr.file.clone(),
                type_index: *type_index,
            },
        },
        LinkedTypeRef::PublicationType {
            module_path,
            type_index,
        } => program_publication_type_addr(program, current_addr, module_path, *type_index)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::ServiceSymbol { symbol } => program
            .types
            .exported_service_type(&symbol.module_path, &symbol.symbol)
            .cloned()
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::PackageSymbol { symbol } => program_package_type_addr(program, symbol)
            .map(|addr| LinkedTypeRef::Address { addr })
            .unwrap_or_else(|| type_ref.clone()),
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| normalize_native_signature_type_arg(program, current_addr, arg))
                .collect(),
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| normalize_native_signature_type_arg(program, current_addr, item))
                .collect(),
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(normalize_native_signature_type_arg(
                program,
                current_addr,
                inner,
            )),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| normalize_native_signature_type_arg(program, current_addr, arg))
                    .collect(),
            },
        },
        LinkedTypeRef::Record { .. }
        | LinkedTypeRef::Function { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::TypeParam { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::Address { .. } => type_ref.clone(),
    }
}

#[cfg(all(test, any()))]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use crate as runtime_root;
    use crate::error::RuntimeError;
    use crate::program::types::ServiceSymbolKey;
    use crate::program::{
        anonymous_type_decl, CallIr, EvalProgramImage, ExecutableAddr, ExprRefIr, FileAddr,
        LinkedCallTarget, LinkedTypeDescriptor, LinkedTypeRef, NativeTarget, RuntimeTypeContext,
        ServiceSymbolRef, TypeAddr, UnitAddr,
    };
    use crate::{Env, EvalRuntimeProgram, Interpreter};
    use skiff_runtime_linked_type_plan::ProgramTypeView;

    use super::{
        normalize_native_signature_type_arg, resolve_config_builtin_type_arg_plan,
        resolve_runtime_native_invocation,
    };

    fn service_type_export_program(
        module_path: &str,
        symbol: &str,
    ) -> (EvalProgramImage, TypeAddr) {
        let addr = TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 42,
        };
        let mut types = RuntimeTypeContext::default();
        types.descriptors.insert(
            addr.clone(),
            anonymous_type_decl(
                format!("{module_path}.{symbol}"),
                LinkedTypeDescriptor::Record {
                    fields: std::collections::BTreeMap::new(),
                },
            ),
        );
        types
            .exported_types
            .insert_service(ServiceSymbolKey::new(module_path, symbol), addr.clone());
        let image = EvalProgramImage {
            service_files: Vec::new(),
            packages: Vec::new(),
            package_files: Vec::new(),
            service_resources: Default::default(),
            package_resources: Vec::new(),
            routes: std::collections::HashMap::new(),
            spawn_routes: std::collections::HashMap::new(),
            operations: std::collections::HashMap::new(),
            operation_receivers: std::collections::HashMap::new(),
            link_overlay: Default::default(),
            types,
        };
        (image, addr)
    }

    fn empty_program() -> EvalProgramImage {
        EvalProgramImage {
            service_files: Vec::new(),
            packages: Vec::new(),
            package_files: Vec::new(),
            routes: std::collections::HashMap::new(),
            spawn_routes: std::collections::HashMap::new(),
            operations: std::collections::HashMap::new(),
            operation_receivers: std::collections::HashMap::new(),
            link_overlay: Default::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn eval_program_for_test(program: &EvalProgramImage) -> EvalRuntimeProgram {
        EvalRuntimeProgram::new(
            "skiff.test/native",
            program.service_files.clone(),
            program.packages.clone(),
            program.package_files.clone(),
            program.service_resources.clone(),
            program.package_resources.clone(),
            program.spawn_routes.clone(),
            program.link_overlay.clone(),
            program.types.clone(),
        )
    }

    fn empty_interpreter() -> Interpreter {
        Interpreter::with_program(
            Arc::new(eval_program_for_test(&empty_program())),
            runtime_root::eval_capability_adapter::runtime_factory(),
        )
    }

    fn actor_find_target() -> NativeTarget {
        NativeTarget {
            namespace: "std.actor".to_string(),
            symbol: "find".to_string(),
            binding_key: Some("std.actor.find".to_string()),
            metadata: BTreeMap::new(),
        }
    }

    fn actor_find_call(type_args: BTreeMap<String, LinkedTypeRef>) -> CallIr {
        let target = actor_find_target();
        CallIr {
            target: LinkedCallTarget::Native { target },
            args: vec![ExprRefIr { expression: 0 }],
            type_args,
            metadata: BTreeMap::new(),
        }
    }

    fn builtin_type(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn invalid_artifact_message(error: RuntimeError) -> String {
        match error {
            RuntimeError::InvalidArtifact(message) => message,
            other => panic!("expected InvalidArtifact, got {other}"),
        }
    }

    #[test]
    fn actor_native_metadata_reports_missing_actor_type_arg() {
        let interpreter = empty_interpreter();
        let addr = ExecutableAddr::service(0, 0);
        let env = Env::default();
        let target = actor_find_target();
        let call = actor_find_call(BTreeMap::new());

        let error =
            match resolve_runtime_native_invocation(&interpreter, &addr, &env, &call, &target) {
                Err(error) => error,
                Ok(_) => panic!("actor metadata resolution should reject missing actor type"),
            };

        assert_eq!(
            invalid_artifact_message(error),
            "std.actor.find call is missing actor typeArgs[0]"
        );
    }

    #[test]
    fn actor_native_metadata_reports_missing_actor_id_type_arg() {
        let interpreter = empty_interpreter();
        let addr = ExecutableAddr::service(0, 0);
        let env = Env::default();
        let target = actor_find_target();
        let call = actor_find_call(BTreeMap::from([("T0".to_string(), builtin_type("Json"))]));

        let error =
            match resolve_runtime_native_invocation(&interpreter, &addr, &env, &call, &target) {
                Err(error) => error,
                Ok(_) => panic!("actor metadata resolution should reject missing actor id type"),
            };

        assert_eq!(
            invalid_artifact_message(error),
            "std.actor.find call is missing actor id typeArgs[1]"
        );
    }

    #[test]
    fn config_type_arg_plan_reports_missing_type_arg_before_native_dispatch() {
        let program = empty_program();
        let addr = ExecutableAddr::service(0, 0);
        let call = CallIr {
            target: LinkedCallTarget::Builtin {
                op: "config.require".to_string(),
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };

        let error = resolve_config_builtin_type_arg_plan(
            ProgramTypeView::from_linked_image(&program),
            &addr,
            &BTreeMap::new(),
            &call,
            "config.require",
        )
        .expect_err("config caller-side plan resolution should reject missing type arg");

        assert_eq!(
            invalid_artifact_message(error),
            "config.require call is missing typeArgs[0]"
        );
    }

    #[test]
    fn case25_service_symbol_normalizes_via_structured_lookup_not_display_path() {
        let (program, expected_addr) = service_type_export_program("svc.main", "User");
        let caller_addr = ExecutableAddr::service(0, 0);

        let service_ref = LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "svc.main".to_string(),
                symbol: "User".to_string(),
            },
        };
        let result = normalize_native_signature_type_arg(&program, &caller_addr, &service_ref);

        assert_eq!(
            result,
            LinkedTypeRef::Address {
                addr: expected_addr
            },
            "ServiceSymbol must resolve to Address via structured (module_path, symbol) lookup"
        );
    }

    #[test]
    fn case25_service_symbol_with_no_matching_key_falls_back_to_original_ref() {
        let (program, _) = service_type_export_program("svc.main", "User");
        let caller_addr = ExecutableAddr::service(0, 0);

        let mismatched_ref = LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "svc".to_string(),
                symbol: "main.User".to_string(),
            },
        };
        let result = normalize_native_signature_type_arg(&program, &caller_addr, &mismatched_ref);

        assert_eq!(
            result, mismatched_ref,
            "ServiceSymbol with no matching structured key must fall back to original ref, not parse display path"
        );
    }

    #[test]
    fn case25_local_type_is_resolved_in_caller_file_context_not_cross_file() {
        let program = EvalProgramImage {
            service_files: Vec::new(),
            packages: Vec::new(),
            package_files: Vec::new(),
            routes: std::collections::HashMap::new(),
            spawn_routes: std::collections::HashMap::new(),
            operations: std::collections::HashMap::new(),
            operation_receivers: std::collections::HashMap::new(),
            link_overlay: Default::default(),
            types: RuntimeTypeContext::default(),
        };
        let caller_addr = ExecutableAddr::service(3, 0);
        let local_ref = LinkedTypeRef::LocalType { type_index: 7 };

        let result = normalize_native_signature_type_arg(&program, &caller_addr, &local_ref);

        assert_eq!(
            result,
            LinkedTypeRef::Address {
                addr: TypeAddr {
                    unit: UnitAddr::Service,
                    file: FileAddr::LoadedFileIndex(3),
                    type_index: 7,
                }
            },
            "LocalType type_index must be bound to its owner file context before cross-boundary normalization"
        );
    }
}
