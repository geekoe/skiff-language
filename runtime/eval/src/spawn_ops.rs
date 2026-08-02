use serde_json::Value;
use skiff_artifact_model::MetadataValue;
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::payload::{PayloadBoundary, PayloadBoundaryKind, PayloadServiceRef};
use skiff_runtime_boundary::{json::RuntimeBoundaryCodec, plan::BoundaryUse};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorInvocationDeclarationOwner, ActorInvocationOwnerFile,
    ActorInvocationOwnerUnit, ActorMethodSpawnTargetControl, SpawnSubmitControlRequest,
};
use skiff_runtime_linked_program::{
    CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, FileAddr, LinkedActorMethodImplementation,
    LinkedCallTarget, LinkedExprIr, UnitAddr,
};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlan, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    recoverable::RuntimeRecoverableExpectedTypePlan, runtime_value::RuntimeValue, value::HeapNode,
};

use crate::{
    actor_instance::resolve_actor_declaration,
    assembly_execution::RuntimeExecutionProjection,
    error::{Result, RuntimeError},
    heap_access::HeapAccess,
    invocation::EvalProgramProjection,
    program_execution::ProgramExecutionContext,
    recoverable_behavior::EvalRecoverableBehaviorHooks,
    recoverable_spawn_payload::{
        decode_spawn_args_payload, executable_request_recoverable_expected_plan,
    },
    Interpreter, RuntimeAssemblyEvalTarget,
};

use super::{eval_context::EvalContext, program_ir::program_expression_ref};

const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SERVICE_BUILD_IDENTITY_PREFIX: &str = "skiff-service-build-v1:sha256:";
const PACKAGE_TEST_BUILD_IDENTITY_PREFIX: &str = "skiff-package-test-build-v1:sha256:";
const PACKAGE_BUILD_IDENTITY_PREFIX: &str = "skiff-package-build-v10:sha256:";

/// Resolves one Router-authenticated direct-spawn target only from the immutable route index
/// validated while the exact active assembly image was linked.
pub fn resolve_runtime_assembly_spawn_target(
    eval_target: &RuntimeAssemblyEvalTarget,
    target: &str,
) -> Result<ExecutableAddr> {
    let projection = eval_target.execution_projection().clone();
    projection
        .image()
        .spawn_route(target)
        .cloned()
        .ok_or_else(|| RuntimeError::Protocol {
            target: target.to_string(),
            message: "canonical spawn target is not registered in the active assembly".to_string(),
        })
}

/// Decodes one direct-spawn recoverable args payload against the exact executable plan and runs
/// it in the caller-provided independent request context.
pub async fn execute_runtime_assembly_spawn_target(
    interpreter: &Interpreter,
    context: ProgramExecutionContext<'_>,
    eval_target: &RuntimeAssemblyEvalTarget,
    addr: &ExecutableAddr,
    payload: &[u8],
) -> Result<()> {
    let projection = eval_target.execution_projection().clone();
    let resolved = projection.resolve_executable(addr)?;
    if resolved.executable.kind != ExecutableKind::Function {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical spawn target {} is not a function",
            resolved.executable.symbol
        )));
    }
    let expected = executable_request_recoverable_expected_plan(
        projection.type_view(),
        &resolved.addr,
        resolved.executable,
    )?;
    let execution_projection = RuntimeExecutionProjection::Assembly(projection.clone());
    let behavior_hooks = EvalRecoverableBehaviorHooks::new_for_execution(&execution_projection)?;
    let activation = eval_target.activation_context();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
        .with_origin_service(
            PayloadServiceRef::new(activation.identity().deployment.service_id.clone())
                .with_version(activation.identity().deployment.contract_version.clone())
                .with_build_id(
                    activation
                        .implementation_package_build_id()
                        .as_str()
                        .to_string(),
                ),
        );
    let mut heap = context.request_heap();
    let decoded =
        decode_spawn_args_payload(payload, &expected, &boundary, &mut heap, &behavior_hooks)?;
    let RuntimeValue::Heap(args_handle) = decoded else {
        return Err(RuntimeError::InvalidArtifact(
            "canonical spawn args payload did not decode to an object".to_string(),
        ));
    };
    let HeapNode::Object(args_object) = heap.get(args_handle)? else {
        return Err(RuntimeError::InvalidArtifact(
            "canonical spawn args payload did not decode to an object".to_string(),
        ));
    };
    let args = resolved
        .executable
        .params
        .iter()
        .map(|parameter| {
            args_object
                .fields()
                .get(&parameter.name)
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "canonical spawn args payload is missing parameter {}",
                        parameter.name
                    ))
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut access = HeapAccess::Exclusive(&mut heap);
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut access, &resolved.addr, args)
        .await?;
    if value != RuntimeValue::Null {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical spawn target {} returned a value",
            resolved.executable.symbol
        )));
    }
    Ok(())
}

pub async fn submit_spawn_statement(
    context: &mut EvalContext<'_, '_>,
    call_ref: ExprRefIr,
) -> Result<()> {
    let expression = program_expression_ref(context.executable, call_ref)?;
    let LinkedExprIr::Call { call } = expression else {
        return Err(RuntimeError::InvalidArtifact(
            "spawn statement must reference a call expression".to_string(),
        ));
    };

    let projection = context.execution_projection().clone();
    let request_context = context.context.request_context().clone();
    let execution_control = context.execution.owned();
    let invocation = encode_spawn_request_payload(context, call, projection).await?;

    let submit = request_context.submit_spawn(
        SpawnSubmitControlRequest {
            rpc_id: String::new(),
            runtime_id: String::new(),
            target_kind: invocation.target_kind,
            service_id: request_context.service_id().to_string(),
            service_version: request_context.service_version().to_string(),
            service_protocol_identity: request_context
                .spawn_service_protocol_identity()
                .to_string(),
            target: invocation.target,
            spawn_id: None,
            build_id: spawn_submit_build_id(request_context.request_build_id()),
            activation_identity: current_activation_identity(
                request_context.activation_identity(),
            )?,
            caller_request_id: Some(request_context.request_id().to_string()),
            trace_id: request_context.trace_id().map(str::to_string),
            caller_target: Some(request_context.request_target().to_string()),
            max_queue_wait_ms: None,
            actor_method: invocation.actor_method,
        },
        invocation.args_payload,
        execution_control,
    );
    context.await_actual_pending(submit).await??;
    Ok(())
}

fn spawn_submit_build_id(request_build_id: &str) -> Option<String> {
    (request_build_id.starts_with(SERVICE_BUILD_IDENTITY_PREFIX)
        || request_build_id.starts_with(PACKAGE_TEST_BUILD_IDENTITY_PREFIX)
        || request_build_id.starts_with(PACKAGE_BUILD_IDENTITY_PREFIX))
    .then(|| request_build_id.to_string())
}

fn current_activation_identity(
    activation_identity: Option<&ActivationIdentityControl>,
) -> Result<ActivationIdentityControl> {
    activation_identity
        .cloned()
        .ok_or_else(|| RuntimeError::Protocol {
            target: "spawn.submit.request".to_string(),
            message: "spawn.submit requires a current pinned ActivationContext".to_string(),
        })
}

#[cfg(test)]
mod spawn_activation_identity_tests {
    use super::{current_activation_identity, spawn_submit_build_id, RuntimeError};

    #[test]
    fn spawn_submit_rejects_missing_current_activation_before_control_send() {
        assert!(matches!(
            current_activation_identity(None),
            Err(RuntimeError::Protocol { .. })
        ));
    }

    #[test]
    fn spawn_submit_preserves_canonical_assembly_package_build_identity() {
        let build_id =
            "skiff-package-build-v10:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert_eq!(spawn_submit_build_id(build_id).as_deref(), Some(build_id));
        assert_eq!(spawn_submit_build_id("legacy-build"), None);
    }
}

struct SpawnEncodedCall {
    target_kind: String,
    target: String,
    args_payload: Vec<u8>,
    actor_method: Option<ActorMethodSpawnTargetControl>,
}

async fn encode_spawn_request_payload(
    context: &mut EvalContext<'_, '_>,
    call: &CallIr,
    projection: RuntimeExecutionProjection<'_>,
) -> Result<SpawnEncodedCall> {
    let target = spawn_submit_target(call)?;
    match target.kind.as_str() {
        "function" => encode_spawn_function_payload(context, call, projection, target).await,
        "actorMethod" => encode_spawn_actor_method_payload(context, call, projection, target).await,
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "spawnSubmit metadata targetKind {} is unsupported",
            target.kind
        ))),
    }
}

async fn encode_spawn_function_payload(
    context: &mut EvalContext<'_, '_>,
    call: &CallIr,
    projection: RuntimeExecutionProjection<'_>,
    target: SpawnSubmitTarget,
) -> Result<SpawnEncodedCall> {
    let addr = match &projection {
        RuntimeExecutionProjection::Legacy(_) => {
            let LinkedCallTarget::Executable { addr } = &call.target else {
                return Err(RuntimeError::InvalidArtifact(
                    "spawn function target was not linked to an executable".to_string(),
                ));
            };
            addr
        }
        RuntimeExecutionProjection::Assembly(_) => canonical_spawn_executable_addr(call)?,
    };
    let resolved = projection.resolve_executable(addr)?;
    if resolved.executable.params.len() != call.args.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawn target {} expects {} argument(s), got {}",
            resolved.executable.symbol,
            resolved.executable.params.len(),
            call.args.len()
        )));
    }
    let route_target = match &projection {
        RuntimeExecutionProjection::Legacy(program) => {
            spawn_function_route_target(*program, addr, &target.name)?
        }
        RuntimeExecutionProjection::Assembly(_) => canonical_spawn_function_target(
            call,
            &target,
            &resolved.executable.kind,
            &resolved.executable.symbol,
        )?,
    };
    let mut fields = std::collections::BTreeMap::new();
    for (param, arg_ref) in resolved.executable.params.iter().zip(&call.args) {
        let value = context.eval_program_expr_ref(*arg_ref).await?;
        fields.insert(param.name.clone(), value);
    }
    let args_handle = context.heap.heap_mut().alloc_object_carriers(fields)?;
    let recoverable_expected = executable_request_recoverable_expected_plan(
        projection.type_view(),
        &resolved.addr,
        resolved.executable,
    )?;
    let request_context = context.context.request_context();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
        .with_origin_service(
            PayloadServiceRef::new(request_context.service_id())
                .with_version(request_context.service_version())
                .with_build_id(request_context.request_build_id()),
        );
    let args_payload = encode_spawn_args_payload(
        &RuntimeValue::Heap(args_handle),
        &recoverable_expected,
        &boundary,
        context.heap.heap_mut(),
        &EvalRecoverableBehaviorHooks::new_for_execution(&projection)?,
    )?;
    Ok(SpawnEncodedCall {
        target_kind: "function".to_string(),
        target: route_target,
        args_payload,
        actor_method: None,
    })
}

async fn encode_spawn_actor_method_payload(
    context: &mut EvalContext<'_, '_>,
    call: &CallIr,
    projection: RuntimeExecutionProjection<'_>,
    target: SpawnSubmitTarget,
) -> Result<SpawnEncodedCall> {
    let LinkedCallTarget::ActorDispatch { plan } = &call.target else {
        return Err(RuntimeError::InvalidArtifact(
            "canonical spawn actor method target is not a linked actor dispatch".to_string(),
        ));
    };
    let expected_target = format!(
        "actorMethod:{}:{}",
        plan.declaration_owner.actor_symbol,
        plan.method_identity.as_str()
    );
    if target.name != expected_target {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawnSubmit metadata target {} does not match linked actor method {}",
            target.name, expected_target
        )));
    }

    let declaration = resolve_actor_declaration(projection.type_view(), &plan.declaration_owner)?;
    let method = exact_actor_method(&declaration.public_methods, &plan.method_identity)?;
    let expected_arguments = method.parameters.len() + 1;
    if call.args.len() != expected_arguments {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawn actor method {} expects {} argument(s) including receiver, got {}",
            method.name,
            expected_arguments,
            call.args.len()
        )));
    }
    let mut values = Vec::with_capacity(call.args.len());
    for arg in &call.args {
        values.push(context.eval_program_expr_ref(*arg).await?);
    }
    let receiver = values.first().ok_or_else(|| {
        RuntimeError::InvalidArtifact("spawn actor method call is missing its receiver".to_string())
    })?;
    let actor_ref = match receiver.value() {
        RuntimeValue::ActorRef(actor_ref) => actor_ref.clone(),
        _ => {
            // `self` in an actor method lowers to a slot whose runtime value is
            // not materialized as an ActorRef; derive it from the current actor
            // execution frame. Any other non-Actor receiver is invalid.
            context
                .context
                .actor_execution_frame()
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "spawn actor method receiver is not an Actor reference".to_string(),
                    )
                })?
                .current_actor_ref()?
        }
    };
    if actor_ref.epoch().is_none() {
        return Err(RuntimeError::InvalidArtifact(
            "spawn actor method receiver is missing its pinned epoch".to_string(),
        ));
    }

    let request_context = context.context.request_context();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
        .with_origin_service(
            PayloadServiceRef::new(request_context.service_id())
                .with_version(request_context.service_version())
                .with_build_id(request_context.request_build_id()),
        );
    let behavior_hooks = EvalRecoverableBehaviorHooks::new_for_execution(&projection)?;

    // Recoverable policy gate: every spawn argument must survive the
    // owner-internal recoverable boundary. The wire payload itself reuses the
    // actor arguments encoding so the owner executor decodes it unchanged.
    let method_addr = actor_method_executable_addr(plan, &method.implementation);
    let resolved_method = projection.resolve_executable(&method_addr)?;
    let recoverable_expected = executable_request_recoverable_expected_plan(
        projection.type_view(),
        &resolved_method.addr,
        resolved_method.executable,
    )?;
    let mut gate_fields = std::collections::BTreeMap::new();
    for (parameter, value) in method.parameters.iter().zip(values.iter().skip(1)) {
        gate_fields.insert(parameter.name.clone(), value.clone());
    }
    let gate_handle = context.heap.heap_mut().alloc_object_carriers(gate_fields)?;
    encode_spawn_args_payload(
        &RuntimeValue::Heap(gate_handle),
        &recoverable_expected,
        &boundary,
        context.heap.heap_mut(),
        &behavior_hooks,
    )?;

    let type_context = PlanContext::from_type_view(projection.type_view(), context.addr);
    let wire_arguments = values
        .iter()
        .skip(1)
        .zip(&method.parameters)
        .enumerate()
        .map(|(index, (value, parameter))| {
            let type_plan = RuntimeTypePlan::from_linked(&parameter.ty, &type_context)?;
            RuntimeBoundaryCodec::new(
                &type_plan,
                BoundaryUse::NativeArg,
                format!("Actor argument {index}"),
            )
            .to_wire_json(value.value(), context.heap.heap_mut())
            .map_err(RuntimeError::from)
        })
        .collect::<Result<Vec<Value>>>()?;
    let arguments_payload = canonical_json_bytes(&Value::Array(wire_arguments))
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;

    Ok(SpawnEncodedCall {
        target_kind: "actorMethod".to_string(),
        target: target.name,
        args_payload: arguments_payload,
        actor_method: Some(ActorMethodSpawnTargetControl {
            actor_ref: actor_ref.clone(),
            declaration_owner: ActorInvocationDeclarationOwner {
                unit: match &plan.declaration_owner.unit {
                    UnitAddr::Service => ActorInvocationOwnerUnit::Service,
                    UnitAddr::Package(slot) => ActorInvocationOwnerUnit::Package(*slot as u64),
                },
                file: match &plan.declaration_owner.file {
                    FileAddr::LoadedFileIndex(index) => {
                        ActorInvocationOwnerFile::LoadedFileIndex(*index as u64)
                    }
                    FileAddr::FileIrIdentity(identity) => {
                        ActorInvocationOwnerFile::FileIrIdentity(identity.clone())
                    }
                },
                actor_symbol: plan.declaration_owner.actor_symbol.clone(),
            },
            actor_abi_identity: plan.actor_abi_identity.clone(),
            actor_implementation_identity: plan.actor_implementation_identity.clone(),
            method_identity: plan.method_identity.clone(),
        }),
    })
}

fn exact_actor_method<'a>(
    methods: &'a [skiff_runtime_linked_program::LinkedActorPublicMethod],
    identity: &skiff_artifact_model::ActorMethodIdentity,
) -> Result<&'a skiff_runtime_linked_program::LinkedActorPublicMethod> {
    let mut matches = methods
        .iter()
        .filter(|method| method.method_identity == *identity);
    let method = matches
        .next()
        .ok_or_else(|| RuntimeError::InvalidArtifact("Actor method is absent".to_string()))?;
    if matches.next().is_some() {
        return Err(RuntimeError::InvalidArtifact(
            "Actor method identity is ambiguous".to_string(),
        ));
    }
    Ok(method)
}

fn actor_method_executable_addr(
    plan: &skiff_runtime_linked_program::LinkedActorMethodDispatchPlan,
    implementation: &LinkedActorMethodImplementation,
) -> ExecutableAddr {
    match implementation {
        LinkedActorMethodImplementation::LocalExecutable { executable_index } => ExecutableAddr {
            unit: plan.declaration_owner.unit.clone(),
            file: plan.declaration_owner.file.clone(),
            executable: *executable_index as usize,
        },
        LinkedActorMethodImplementation::Executable { addr } => addr.clone(),
    }
}

fn canonical_spawn_executable_addr(call: &CallIr) -> Result<&ExecutableAddr> {
    match &call.target {
        LinkedCallTarget::Executable { addr } => Ok(addr),
        LinkedCallTarget::PackageDirect { call } => Ok(call.executable_addr()),
        _ => Err(RuntimeError::InvalidArtifact(
            "canonical spawn function target is not an exact linked executable".to_string(),
        )),
    }
}

fn canonical_spawn_function_target(
    call: &CallIr,
    metadata: &SpawnSubmitTarget,
    executable_kind: &skiff_runtime_linked_program::ExecutableKind,
    executable_symbol: &str,
) -> Result<String> {
    if executable_kind != &skiff_runtime_linked_program::ExecutableKind::Function {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical spawn target {} is not a function",
            executable_symbol
        )));
    }
    let expected_metadata_target = match &call.target {
        LinkedCallTarget::Executable { .. } => format!("function:{executable_symbol}"),
        LinkedCallTarget::PackageDirect { call } => {
            format!("package:{}", call.package_callable_id())
        }
        _ => {
            return Err(RuntimeError::InvalidArtifact(
                "canonical spawn function target is not an exact linked executable".to_string(),
            ))
        }
    };
    if metadata.name != expected_metadata_target {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical spawnSubmit metadata target {} does not match linked executable {}",
            metadata.name, expected_metadata_target
        )));
    }
    Ok(format!("function:{executable_symbol}"))
}

fn spawn_function_route_target(
    program: EvalProgramProjection<'_>,
    addr: &ExecutableAddr,
    metadata_target: &str,
) -> Result<String> {
    if program
        .spawn_route(metadata_target)
        .is_some_and(|candidate| candidate == addr)
    {
        return Ok(metadata_target.to_string());
    }

    let mut candidates = program
        .spawn_route_targets_for(addr)
        .into_iter()
        .filter(|target| target.starts_with("package.") || target.starts_with("function:"))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates
        .first()
        .map(|target| (*target).to_string())
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "spawn function target {metadata_target} is not registered as a runtime route"
            ))
        })
}

fn encode_spawn_args_payload(
    value: &RuntimeValue,
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &skiff_runtime_model::request_heap::RequestHeap,
    behavior_hooks: &dyn skiff_runtime_boundary::recoverable::RecoverableBehaviorHooks,
) -> Result<Vec<u8>> {
    crate::recoverable_spawn_payload::encode_spawn_args_payload(
        value,
        expected,
        boundary,
        heap,
        behavior_hooks,
    )
}

struct SpawnSubmitTarget {
    kind: String,
    name: String,
}

fn spawn_submit_target(call: &CallIr) -> Result<SpawnSubmitTarget> {
    // LinkedStmtIr::Spawn currently carries only a call expression. The runtime
    // must not infer queue identity from that lossy shape: compiler metadata
    // needs to name the target and, later, provide the stable arg codec.
    let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
        return Err(RuntimeError::InvalidArtifact(
            "spawn statement is missing compiler spawnSubmit metadata for router queue submit"
                .to_string(),
        ));
    };
    let metadata = metadata_to_json(metadata);
    let object = metadata.as_object().ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "spawnSubmit metadata must be an object with targetKind and target fields".to_string(),
        )
    })?;
    let kind = object
        .get("targetKind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "spawnSubmit metadata targetKind must be a string".to_string(),
            )
        })?;
    if kind != "function" && kind != "actorMethod" {
        return Err(RuntimeError::InvalidArtifact(format!(
            "spawnSubmit metadata targetKind {kind} is unsupported"
        )));
    }
    let name = object
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "spawnSubmit metadata target must be a string".to_string(),
            )
        })?;
    Ok(SpawnSubmitTarget {
        kind: kind.to_string(),
        name: name.to_string(),
    })
}

fn metadata_to_json(value: &MetadataValue) -> Value {
    match value {
        MetadataValue::Null => Value::Null,
        MetadataValue::Bool(value) => Value::Bool(*value),
        MetadataValue::Number(value) => Value::Number(value.clone()),
        MetadataValue::String(value) => Value::String(value.clone()),
        MetadataValue::Array(items) => Value::Array(items.iter().map(metadata_to_json).collect()),
        MetadataValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), metadata_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod recoverable_spawn_payload_tests {
    use crate::heap_access::HeapAccess;

    use std::{
        collections::{BTreeMap, HashMap},
        sync::Arc,
    };

    use skiff_artifact_identity::{abi_type_id_from_source_anchor, abi_type_id_key};
    use skiff_artifact_model::{
        AbiDeclarationKind, AbiSourceDeclarationAnchor, InstructionSourceSite,
        SyntheticInstructionSiteReason,
    };
    use skiff_runtime_boundary::{
        error::RecoverableBoundaryErrorCode,
        payload::{PayloadBoundary, PayloadBoundaryKind},
    };
    use skiff_runtime_capability_context::DbCapabilityContext;
    use skiff_runtime_linked_program::linked::TypeDeclarationIr;
    use skiff_runtime_linked_program::{
        BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, FileDeclarations,
        FileLinkTargets, LinkOverlay, LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutable,
        LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
        LinkedInterfaceMethodSlotPlanIr, LinkedInterfaceMethodSlotSignatureIr,
        LinkedInterfaceMethodSlotTargetIr, LinkedInterfaceMethodTablePlanIr, LinkedStmtIr,
        LinkedTypeDescriptor, LinkedTypeRef, ParamIr, ReceiverCallAbi, RuntimeExecutionPackage,
        RuntimeTypeContext, SlotIr, SlotLayoutIr, StmtRefIr, TypeAddr, TypeDeclIr, UnitAddr,
    };
    use skiff_runtime_linked_type_plan::{
        linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
    };
    use skiff_runtime_model::{
        request_heap::{RequestHeap, RequestHeapLimits},
        runtime_value::{
            HeapNode, InterfaceCarrier, InterfaceMethodTarget, InterfaceReceiverCallAbi,
            InterfaceValue, RuntimeObject, RuntimeObjectFields, RuntimeValue,
        },
    };

    use super::encode_spawn_args_payload;
    use crate::{
        assembly_execution::ordinary::tests::test_runtime,
        capabilities::TimeCapabilityContext,
        env::Env,
        error::RuntimeError,
        invocation::EvalProgramProjection,
        program_execution::{ProgramExecutionContext, ProgramExecutionInput},
        recoverable_behavior::{
            interface_method_table_from_linked, runtime_interface_method_table_id,
            EvalRecoverableBehaviorHooks,
        },
        recoverable_spawn_payload::{
            decode_spawn_args_payload, executable_request_recoverable_expected_plan,
        },
        EvalRuntimeProgram, Interpreter,
    };

    const ARTIFACT_IDENTITY: &str =
        "skiff-service-protocol-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BUILD_ID: &str = "skiff-service-build-v1:sha256:test";
    const SERVICE_ID: &str = "skiff.test/provider";
    const INTERFACE_ABI: &str = "pkg.ToolProvider";
    const METHOD_ABI: &str = "pkg.ToolProvider.call";
    const CANONICAL_METHOD_ABI: &str = "method:pkg.ToolProvider:call";

    struct TestProgram {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<RuntimeExecutionPackage>>,
        spawn_routes: HashMap<String, ExecutableAddr>,
        link_overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl TestProgram {
        fn with_interface_box() -> Self {
            let file = Arc::new(linked_file_with_interface_box());
            let provider_addr = provider_type_addr();
            Self {
                service_files: vec![file.clone()],
                packages: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([(provider_addr, file.types[0].clone())]),
                    exported_types: Default::default(),
                },
            }
        }

        fn with_duplicate_restore_key() -> Self {
            let first = Arc::new(linked_file_with_interface_box_for_file(0));
            let second = Arc::new(linked_file_with_interface_box_for_file(1));
            Self {
                service_files: vec![first.clone(), second.clone()],
                packages: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([
                        (provider_type_addr_for_file(0), first.types[0].clone()),
                        (provider_type_addr_for_file(1), second.types[0].clone()),
                    ]),
                    exported_types: Default::default(),
                },
            }
        }

        fn with_generic_interface_box() -> Self {
            let mut file = linked_file_with_interface_box();
            file.types[0].type_params = vec!["T".to_string()];
            let file = Arc::new(file);
            let provider_addr = provider_type_addr();
            Self {
                service_files: vec![file.clone()],
                packages: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext {
                    descriptors: HashMap::from([(provider_addr, file.types[0].clone())]),
                    exported_types: Default::default(),
                },
            }
        }

        fn empty() -> Self {
            Self {
                service_files: Vec::new(),
                packages: Vec::new(),
                spawn_routes: HashMap::new(),
                link_overlay: LinkOverlay::default(),
                types: RuntimeTypeContext::default(),
            }
        }

        fn projection(&self) -> EvalProgramProjection<'_> {
            EvalProgramProjection::new(
                SERVICE_ID,
                &self.service_files,
                &self.packages,
                &self.spawn_routes,
                &self.link_overlay,
                &self.types,
            )
        }
    }

    fn linked_file_with_interface_box() -> LinkedFileUnit {
        linked_file_with_interface_box_for_file(0)
    }

    fn linked_file_with_interface_box_for_file(file_index: usize) -> LinkedFileUnit {
        let mut declarations = FileDeclarations::default();
        declarations.types.insert(
            "ProviderImpl".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "ProviderImpl".to_string(),
                source_span: None,
            },
        );
        LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: format!("file:test:{file_index}"),
            source_ast_hash: "source:test".to_string(),
            module_path: "pkg".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations,
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: vec![TypeDeclIr {
                name: "ProviderImpl".to_string(),
                descriptor: LinkedTypeDescriptor::Alias {
                    target: string_type(),
                },
                type_params: Vec::new(),
                implements: vec![LinkedTypeRef::AnyInterface {
                    interface: tool_provider_interface(),
                }],
                source_span: None,
            }],
            constants: Vec::new(),
            executables: vec![
                box_owner_executable(file_index),
                provider_method_executable(file_index),
                spawn_target_executable(),
                runtime_bindings_spawn_target_executable(),
                provider_dispatch_probe_executable(),
            ],
            external_refs: Default::default(),
        }
    }

    fn box_owner_executable(file_index: usize) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "boxOwner".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody {
                blocks: Vec::new(),
                statements: Vec::new(),
                expressions: vec![LinkedExprIr::InterfaceBox {
                    value: ExprRefIr { expression: 0 },
                    interface: tool_provider_interface(),
                    source: LinkedBoxSourceIr::Local {
                        concrete_type: provider_concrete_type_for_file(file_index),
                        method_table: method_table_plan_for_file(file_index),
                    },
                }],
            },
        }
    }

    fn provider_method_executable(file_index: usize) -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::ImplMethod,
            symbol: "ProviderImpl.call".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(string_type()),
            self_type: Some(provider_concrete_type_for_file(file_index)),
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "self".to_string(),
                    kind: "selfValue".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }],
                }],
                statements: vec![LinkedStmtIr::Return {
                    value: Some(ExprRefIr { expression: 0 }),
                }],
                expressions: vec![LinkedExprIr::LoadSlot { slot: 0 }],
            },
        }
    }

    fn provider_dispatch_probe_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "providerDispatchProbe".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "provider".to_string(),
                slot: 0,
                ty: provider_any_type(),
            }],
            return_type: Some(string_type()),
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "provider".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }],
                }],
                statements: vec![LinkedStmtIr::Return {
                    value: Some(ExprRefIr { expression: 1 }),
                }],
                expressions: vec![
                    LinkedExprIr::LoadSlot { slot: 0 },
                    LinkedExprIr::Call {
                        call: CallIr {
                            target: LinkedCallTarget::InterfaceMethod {
                                interface: tool_provider_interface(),
                                method_abi_id: CANONICAL_METHOD_ABI.to_string(),
                                slot: 0,
                            },
                            site: InstructionSourceSite::Synthetic {
                                reason:
                                    SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                            },
                            args: vec![ExprRefIr { expression: 0 }],
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::new(),
                            actor_metadata: None,
                        },
                    },
                ],
            },
        }
    }

    fn spawn_target_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "spawnTarget".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "provider".to_string(),
                slot: 0,
                ty: LinkedTypeRef::AnyInterface {
                    interface: tool_provider_interface(),
                },
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "provider".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn runtime_bindings_spawn_target_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "spawnRuntimeBindingsTarget".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "bindings".to_string(),
                slot: 0,
                ty: runtime_bindings_type(),
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "bindings".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn plain_string_executable() -> LinkedExecutable {
        LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "plainTarget".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "name".to_string(),
                slot: 0,
                ty: string_type(),
            }],
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "name".to_string(),
                    kind: "param".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        }
    }

    fn provider_any_type() -> LinkedTypeRef {
        LinkedTypeRef::AnyInterface {
            interface: tool_provider_interface(),
        }
    }

    fn array_type(item: LinkedTypeRef) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "Array".to_string(),
            args: vec![item],
        }
    }

    fn runtime_bindings_type() -> LinkedTypeRef {
        LinkedTypeRef::Record {
            fields: BTreeMap::from([
                ("events".to_string(), provider_any_type()),
                ("llm".to_string(), provider_any_type()),
                ("providers".to_string(), array_type(provider_any_type())),
            ]),
        }
    }

    fn tool_provider_interface() -> LinkedInterfaceInstantiationRef {
        LinkedInterfaceInstantiationRef {
            interface_abi_id: INTERFACE_ABI.to_string(),
            canonical_type_args: Vec::new(),
        }
    }

    fn provider_concrete_type() -> LinkedTypeRef {
        provider_concrete_type_for_file(0)
    }

    fn provider_concrete_type_for_file(file_index: usize) -> LinkedTypeRef {
        LinkedTypeRef::Address {
            addr: provider_type_addr_for_file(file_index),
        }
    }

    fn provider_type_addr() -> TypeAddr {
        provider_type_addr_for_file(0)
    }

    fn provider_type_addr_for_file(file_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(file_index),
            type_index: 0,
        }
    }

    fn provider_stable_restore_key() -> String {
        let input = AbiSourceDeclarationAnchor {
            publication_id: SERVICE_ID.to_string(),
            abi_epoch: 0,
            module_path: vec!["pkg".to_string()],
            symbol: "ProviderImpl".to_string(),
            kind: AbiDeclarationKind::Type,
        };
        let type_id = abi_type_id_from_source_anchor(&input, &[]);
        format!("abi-type:{}", abi_type_id_key(&type_id))
    }

    fn string_type() -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }
    }

    fn method_table_plan() -> LinkedInterfaceMethodTablePlanIr {
        method_table_plan_for_file(0)
    }

    fn method_table_plan_for_file(file_index: usize) -> LinkedInterfaceMethodTablePlanIr {
        LinkedInterfaceMethodTablePlanIr {
            interface: tool_provider_interface(),
            concrete_type: provider_concrete_type_for_file(file_index),
            slots: vec![LinkedInterfaceMethodSlotPlanIr {
                slot: 0,
                method_name: "call".to_string(),
                method_abi_id: METHOD_ABI.to_string(),
                signature: LinkedInterfaceMethodSlotSignatureIr {
                    params: Vec::new(),
                    return_type: string_type(),
                },
                target: LinkedInterfaceMethodSlotTargetIr {
                    executable_index: 1,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            }],
        }
    }

    fn spawn_target_addr() -> ExecutableAddr {
        ExecutableAddr::service(0, 2)
    }

    fn runtime_bindings_spawn_target_addr() -> ExecutableAddr {
        ExecutableAddr::service(0, 3)
    }

    fn provider_value(heap: &mut RequestHeap) -> RuntimeValue {
        let method_table = interface_method_table_from_linked(
            &ExecutableAddr::service(0, 0),
            &method_table_plan(),
        )
        .expect("method table should build");
        RuntimeValue::Heap(
            heap.alloc_interface(InterfaceValue::new(
                linked_interface_instantiation_runtime_id(&tool_provider_interface()),
                InterfaceCarrier::Local {
                    concrete_type: linked_type_ref_runtime_key(&provider_concrete_type()),
                    method_table,
                    payload: RuntimeValue::String("state".to_string()),
                },
            ))
            .expect("provider interface should allocate"),
        )
    }

    fn runtime_bindings_value(heap: &mut RequestHeap) -> RuntimeValue {
        let first_provider = provider_value(heap);
        let second_provider = provider_value(heap);
        let providers = RuntimeValue::Heap(
            heap.alloc_array(vec![first_provider, second_provider])
                .expect("providers array should allocate"),
        );
        let llm = provider_value(heap);
        let events = provider_value(heap);
        RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("llm".to_string(), llm),
                ("events".to_string(), events),
                ("providers".to_string(), providers),
            ])))
            .expect("runtime bindings object should allocate"),
        )
    }

    fn args_record(heap: &mut RequestHeap, field: &str, value: RuntimeValue) -> RuntimeValue {
        RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                field.to_string(),
                value,
            )])))
            .expect("args object should allocate"),
        )
    }

    fn assert_decoded_provider_value(heap: &RequestHeap, value: &RuntimeValue, label: &str) {
        let RuntimeValue::Heap(provider_handle) = value else {
            panic!("{label} should be a heap value");
        };
        let HeapNode::Interface(provider) = heap.get(*provider_handle).expect("provider resolves")
        else {
            panic!("{label} should decode as InterfaceValue");
        };
        let InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } = provider.carrier()
        else {
            panic!("{label} should decode as local carrier");
        };
        assert_eq!(provider.interface(), INTERFACE_ABI);
        let runtime_key = linked_type_ref_runtime_key(&provider_concrete_type());
        assert_eq!(concrete_type, &runtime_key);
        assert_ne!(concrete_type, &provider_stable_restore_key());
        assert_eq!(payload, &RuntimeValue::String("state".to_string()));
        assert_eq!(
            method_table.id(),
            runtime_interface_method_table_id(INTERFACE_ABI, &runtime_key)
        );
        assert_eq!(method_table.interface_abi_id(), INTERFACE_ABI);
        assert_eq!(
            method_table.slots()[0].method_abi_id(),
            CANONICAL_METHOD_ABI
        );
    }

    #[test]
    fn spawn_submit_args_helper_encodes_recoverable_envelope_and_plain_roundtrip() {
        let program = TestProgram::empty();
        let projection = program.projection();
        let executable = plain_string_executable();
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &ExecutableAddr::service(0, 0),
            &executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let value = args_record(&mut heap, "name", RuntimeValue::String("Ada".to_string()));

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("spawn args should encode as recoverable envelope");

        assert_eq!(&bytes[..4], b"SKRE");

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("spawn recoverable args should decode");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(object) = decode_heap.get(handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        assert_eq!(
            object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
    }

    #[test]
    fn spawn_submit_args_helper_roundtrips_local_interface_with_production_hooks() {
        let program = TestProgram::with_interface_box();
        let projection = program.projection();
        let executable = &program.service_files[0].executables[2];
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let provider = provider_value(&mut heap);
        let value = args_record(&mut heap, "provider", provider);

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("local interface should encode before spawn submit");
        assert_eq!(&bytes[..4], b"SKRE");
        let payload_text = String::from_utf8_lossy(&bytes);
        assert!(!payload_text.contains(ARTIFACT_IDENTITY));
        assert!(!payload_text.contains(BUILD_ID));

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("local interface should decode on spawn worker");
        let RuntimeValue::Heap(args_handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(args) = decode_heap.get(args_handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        let RuntimeValue::Heap(provider_handle) = args
            .fields()
            .get("provider")
            .expect("provider arg should exist")
        else {
            panic!("provider should be a heap value");
        };
        let HeapNode::Interface(provider) = decode_heap
            .get(*provider_handle)
            .expect("provider resolves")
        else {
            panic!("provider should decode as InterfaceValue");
        };
        let InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } = provider.carrier()
        else {
            panic!("provider should decode as local carrier");
        };
        assert_eq!(provider.interface(), INTERFACE_ABI);
        let runtime_key = linked_type_ref_runtime_key(&provider_concrete_type());
        assert_eq!(concrete_type, &runtime_key);
        assert_ne!(concrete_type, &provider_stable_restore_key());
        assert_eq!(payload, &RuntimeValue::String("state".to_string()));
        assert_eq!(
            method_table.id(),
            runtime_interface_method_table_id(INTERFACE_ABI, &runtime_key)
        );
        assert_eq!(method_table.interface_abi_id(), INTERFACE_ABI);
        assert_eq!(
            method_table.slots()[0].method_abi_id(),
            CANONICAL_METHOD_ABI
        );
        let InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi,
        } = method_table.slots()[0].target();
        assert_eq!(executable, &ExecutableAddr::service(0, 1));
        assert_eq!(
            receiver_call_abi,
            &InterfaceReceiverCallAbi::ExplicitSelfFirst
        );

        let reencoded =
            encode_spawn_args_payload(&decoded, &expected, &boundary, &decode_heap, &hooks)
                .expect("decoded local interface should re-encode on the spawn worker");
        assert_eq!(&reencoded[..4], b"SKRE");
    }

    #[test]
    fn spawn_submit_args_helper_roundtrips_runtime_bindings_record_with_interface_array() {
        let program = TestProgram::with_interface_box();
        let projection = program.projection();
        let executable = &program.service_files[0].executables[3];
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &runtime_bindings_spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let bindings = runtime_bindings_value(&mut heap);
        let value = args_record(&mut heap, "bindings", bindings);

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("runtime bindings should encode before spawn submit");
        assert_eq!(&bytes[..4], b"SKRE");
        let payload_text = String::from_utf8_lossy(&bytes);
        assert!(!payload_text.contains(ARTIFACT_IDENTITY));
        assert!(!payload_text.contains(BUILD_ID));

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("runtime bindings should decode on spawn worker");
        let RuntimeValue::Heap(args_handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(args) = decode_heap.get(args_handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        let RuntimeValue::Heap(bindings_handle) = args
            .fields()
            .get("bindings")
            .expect("bindings arg should exist")
        else {
            panic!("bindings should be a heap value");
        };
        let HeapNode::Object(bindings) = decode_heap
            .get(*bindings_handle)
            .expect("bindings object resolves")
        else {
            panic!("bindings should decode as an object");
        };

        assert_decoded_provider_value(
            &decode_heap,
            bindings.fields().get("llm").expect("llm field exists"),
            "llm",
        );
        assert_decoded_provider_value(
            &decode_heap,
            bindings
                .fields()
                .get("events")
                .expect("events field exists"),
            "events",
        );

        let RuntimeValue::Heap(providers_handle) = bindings
            .fields()
            .get("providers")
            .expect("providers field exists")
        else {
            panic!("providers should be a heap value");
        };
        let HeapNode::Array(providers) = decode_heap
            .get(*providers_handle)
            .expect("providers array resolves")
        else {
            panic!("providers should decode as an array");
        };
        assert_eq!(providers.len(), 2);
        assert_decoded_provider_value(&decode_heap, &providers[0], "providers[0]");
        assert_decoded_provider_value(&decode_heap, &providers[1], "providers[1]");

        let reencoded =
            encode_spawn_args_payload(&decoded, &expected, &boundary, &decode_heap, &hooks)
                .expect("decoded runtime bindings should re-encode on the spawn worker");
        assert_eq!(&reencoded[..4], b"SKRE");
    }

    fn probe_caller_addr() -> ExecutableAddr {
        ExecutableAddr::service(0, 4)
    }

    fn probe_program_context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
        let stream_runtime = interpreter.stream_runtime.clone();
        let effects = test_runtime::effects_context();
        let execution = test_runtime::execution_control();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: test_runtime::config_context(),
            db: DbCapabilityContext::unavailable(),
            file: test_runtime::file_context(),
            file_source_stream: test_runtime::file_source_stream_context(stream_runtime.clone()),
            time: TimeCapabilityContext::new(execution),
            websocket: test_runtime::websocket_context(),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                stream_runtime,
                interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: interpreter.test_effect_double_context(),
            actor: test_runtime::actor_context(),
            request: test_runtime::request_context(),
            request_heap_limits: RequestHeapLimits::default(),
        })
    }

    #[tokio::test]
    async fn decoded_local_interface_provider_dispatches_program_method() {
        let program = TestProgram::with_interface_box();
        let projection = program.projection();
        let executable = &program.service_files[0].executables[2];
        let expected = executable_request_recoverable_expected_plan(
            projection.type_view(),
            &spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let hooks = EvalRecoverableBehaviorHooks::new(projection, ARTIFACT_IDENTITY, BUILD_ID)
            .expect("production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let provider = provider_value(&mut heap);
        let value = args_record(&mut heap, "provider", provider);

        let bytes = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect("provider should encode before spawn submit");
        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_spawn_args_payload(&bytes, &expected, &boundary, &mut decode_heap, &hooks)
                .expect("provider should decode on spawn worker");
        let RuntimeValue::Heap(args_handle) = decoded else {
            panic!("decoded args should be a heap object");
        };
        let HeapNode::Object(args) = decode_heap.get(args_handle).expect("args object resolves")
        else {
            panic!("decoded args should be an object");
        };
        let decoded_provider = args
            .fields()
            .get("provider")
            .cloned()
            .expect("provider arg should exist");

        let runtime_program = Arc::new(EvalRuntimeProgram {
            service_id: SERVICE_ID.to_string(),
            service_files: vec![Arc::clone(&program.service_files[0])],
            packages: Vec::new(),
            service_resources: Default::default(),
            spawn_routes: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: program.types.clone(),
        });
        let interpreter =
            Interpreter::with_program(runtime_program, test_runtime::runtime_factory());
        let context = probe_program_context(&interpreter);
        let caller_addr = probe_caller_addr();
        let mut access = HeapAccess::Exclusive(&mut decode_heap);
        let result = interpreter
            .call_program_executable(
                context,
                &mut access,
                &Env::new(),
                &caller_addr,
                &caller_addr,
                &BTreeMap::new(),
                vec![decoded_provider],
            )
            .await
            .expect("decoded provider should dispatch through the linked program method table");

        assert_eq!(result, RuntimeValue::String("state".to_string()));
    }

    #[test]
    fn recoverable_hooks_reject_duplicate_local_concrete_restore_key_candidates() {
        let program = TestProgram::with_duplicate_restore_key();

        let result =
            EvalRecoverableBehaviorHooks::new(program.projection(), ARTIFACT_IDENTITY, BUILD_ID);

        match result {
            Err(RuntimeError::InvalidArtifact(message)) => assert!(
                message.contains("conflicting restore metadata"),
                "unexpected invalid artifact message: {message}"
            ),
            Err(error) => panic!("expected invalid artifact error, got {error}"),
            Ok(_) => panic!("duplicate stable local concrete restore key should fail closed"),
        }
    }

    #[test]
    fn recoverable_hooks_reject_generic_local_concrete_without_stable_type_args() {
        let program = TestProgram::with_generic_interface_box();

        let result =
            EvalRecoverableBehaviorHooks::new(program.projection(), ARTIFACT_IDENTITY, BUILD_ID);

        match result {
            Err(RuntimeError::InvalidArtifact(message)) => assert!(
                message.contains("generic")
                    && message.contains("stable restore keys for concrete type arguments"),
                "unexpected invalid artifact message: {message}"
            ),
            Err(error) => panic!("expected invalid artifact error, got {error}"),
            Ok(_) => panic!("generic local concrete restore key should fail closed"),
        }
    }

    #[test]
    fn spawn_submit_args_helper_fails_behavior_without_linked_method_table_before_bytes() {
        let program = TestProgram::with_interface_box();
        let executable = &program.service_files[0].executables[2];
        let expected = executable_request_recoverable_expected_plan(
            program.projection().type_view(),
            &spawn_target_addr(),
            executable,
        )
        .expect("recoverable expected plan should build");
        let empty_program = TestProgram::empty();
        let hooks = EvalRecoverableBehaviorHooks::new(
            empty_program.projection(),
            ARTIFACT_IDENTITY,
            BUILD_ID,
        )
        .expect("empty production hooks should build");
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let provider = provider_value(&mut heap);
        let value = args_record(&mut heap, "provider", provider);

        let error = encode_spawn_args_payload(&value, &expected, &boundary, &heap, &hooks)
            .expect_err("unsupported local interface must fail before submit bytes are returned");

        let payload = error
            .ordinary_payload()
            .expect("recoverable error remains ordinary");
        assert_eq!(payload.code, "recoverable_code_identity_missing");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected carried boundary recoverable diagnostic, got {error}");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CodeIdentityMissing
        );
    }
}

#[cfg(test)]
mod legacy_spawn_tests {
    use std::{collections::HashMap, sync::Arc};

    use skiff_runtime_linked_program::{
        ExecutableAddr, LinkOverlay, LinkedFileUnit, RuntimeExecutionPackage, RuntimeTypeContext,
    };

    use crate::invocation::EvalProgramProjection;

    use super::{spawn_function_route_target, spawn_submit_build_id};

    #[test]
    fn spawn_submit_build_id_keeps_package_test_builds() {
        let build_id = "skiff-package-test-build-v1:sha256:aaaaaaaa";
        assert_eq!(spawn_submit_build_id(build_id).as_deref(), Some(build_id));
    }

    #[test]
    fn spawn_submit_build_id_keeps_service_builds() {
        let build_id = "skiff-service-build-v1:sha256:aaaaaaaa";
        assert_eq!(spawn_submit_build_id(build_id).as_deref(), Some(build_id));
    }

    #[test]
    fn spawn_function_route_target_falls_back_to_package_route_for_linked_addr() {
        let addr = ExecutableAddr::package(0, 0, 0);
        let mut routes = HashMap::new();
        routes.insert(
            "package.example%2Ecom%2Fagent.runDrain".to_string(),
            addr.clone(),
        );
        let service_files = Vec::<Arc<LinkedFileUnit>>::new();
        let packages = Vec::<Arc<RuntimeExecutionPackage>>::new();
        let link_overlay = LinkOverlay::default();
        let types = RuntimeTypeContext::default();
        let program = EvalProgramProjection::new(
            "skiff.test/spawn",
            &service_files,
            &packages,
            &routes,
            &link_overlay,
            &types,
        );

        let target =
            spawn_function_route_target(program, &addr, "package:runDrain").expect("route target");
        assert_eq!(target, "package.example%2Ecom%2Fagent.runDrain");
    }
}

#[cfg(test)]
mod tests;
