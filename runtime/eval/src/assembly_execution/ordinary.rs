use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackLifetime, BoundaryCancellationContract,
    BoundaryErrorContract, BoundaryOperationDescriptor, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractSchemaType, ContractTypeId, ContractTypeRef,
};
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableCapabilityHooks, ServiceLinkableContractPlan,
    ServiceLinkableMaterializationError, ServiceLinkableMaterializationScope,
};
use skiff_runtime_linked_program::{CallIr, LinkedPackageDirectCall};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::callback_native::CallbackNativeCapabilityHooks;
use crate::{
    env::Env,
    error::{Result, RuntimeError, UserException},
    eval_context::EvalContext,
    exceptions::user_exception_for_catch,
    runtime_ops::{runtime_from_wire, runtime_to_wire},
    RuntimeAssemblyServiceCallTarget,
};

pub(crate) async fn execute_package_direct(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: &LinkedPackageDirectCall,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    context
        .interpreter
        .call_program_executable(
            context.context.clone(),
            context.heap,
            context.env,
            context.addr,
            target.executable_addr(),
            &call.type_args,
            args,
        )
        .await
}

pub(crate) async fn execute_service_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let boundary =
        OrdinaryServiceBoundary::new(target.descriptor(), target.contract(), call, &args)?;
    let mut provider_heap = context.context.request_heap();
    let caller_hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &caller_hooks)?;

    let provider_eval_target = context
        .context
        .runtime_assembly_target()?
        .with_request_activation(target.provider_request().clone())?;
    let provider_context = context
        .context
        .clone()
        .with_runtime_assembly_target(provider_eval_target);
    let provider_env = Env::new();
    let provider_type_args = Default::default();
    let provider_result = context
        .interpreter
        .call_program_executable(
            provider_context.clone(),
            &mut provider_heap,
            &provider_env,
            target.executable_addr(),
            target.executable_addr(),
            &provider_type_args,
            provider_args,
        )
        .await;
    let provider_hooks = CallbackNativeCapabilityHooks::new(&provider_context);

    match provider_result {
        Ok(value) => {
            boundary
                .return_plan
                .materialize(&value, &provider_heap, context.heap, &provider_hooks)
        }
        Err(error) => {
            boundary.materialize_error(error, &mut provider_heap, context.heap, &provider_hooks)
        }
    }
}

struct OrdinaryServiceBoundary<'a> {
    operation: &'a BoundaryOperationDescriptor,
    parameter_plans: Vec<DirectionalMaterializationPlan<'a>>,
    return_plan: DirectionalMaterializationPlan<'a>,
    error_plan: Option<DirectionalMaterializationPlan<'a>>,
}

impl<'a> OrdinaryServiceBoundary<'a> {
    fn new(
        operation: &'a BoundaryOperationDescriptor,
        contract: &'a skiff_artifact_model::ServiceContract,
        call: &CallIr,
        args: &[RuntimeValue],
    ) -> Result<Self> {
        validate_ordinary_operation(operation, call, args.len())?;
        let schema = &contract.boundary_schema;
        let parameter_plans = operation
            .contract
            .parameters
            .iter()
            .map(|parameter| {
                DirectionalMaterializationPlan::new(
                    &parameter.ty,
                    schema,
                    &parameter.value_plan,
                    BoundaryValueOwner::Caller,
                    &operation.operation_id.to_string(),
                    "parameter",
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let return_plan = DirectionalMaterializationPlan::new(
            &operation.contract.return_value.ty,
            schema,
            &operation.contract.return_value.value_plan,
            BoundaryValueOwner::Provider,
            &operation.operation_id.to_string(),
            "return",
        )?;
        let error_plan = match &operation.contract.errors {
            BoundaryErrorContract::None => None,
            BoundaryErrorContract::Typed {
                payload_type,
                value_plan,
            } => Some(DirectionalMaterializationPlan::new(
                payload_type,
                schema,
                value_plan,
                BoundaryValueOwner::Provider,
                &operation.operation_id.to_string(),
                "typed error",
            )?),
            BoundaryErrorContract::Unsupported { reason } => {
                return Err(RuntimeError::Unsupported(format!(
                    "canonical service operation {} has unsupported error semantics: {reason:?}",
                    operation.operation_id
                )));
            }
        };
        Ok(Self {
            operation,
            parameter_plans,
            return_plan,
            error_plan,
        })
    }

    fn materialize_parameters(
        &self,
        args: &[RuntimeValue],
        caller_heap: &RequestHeap,
        provider_heap: &mut RequestHeap,
        hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<Vec<RuntimeValue>> {
        self.parameter_plans
            .iter()
            .zip(args)
            .map(|(plan, value)| plan.materialize(value, caller_heap, provider_heap, hooks))
            .collect()
    }

    fn materialize_error(
        &self,
        error: RuntimeError,
        provider_heap: &mut RequestHeap,
        caller_heap: &mut RequestHeap,
        hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        let Some(exception) = user_exception_for_catch(&error).cloned() else {
            return Err(error);
        };
        let Some(error_plan) = &self.error_plan else {
            return Err(RuntimeError::Protocol {
                target: self.operation.operation_id.to_string(),
                message:
                    "provider threw a typed business error but the contract declares no typed error"
                        .to_string(),
            });
        };
        let mut envelope = exception.envelope();
        let payload = envelope
            .as_object()
            .and_then(|object| object.get("error"))
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol {
                target: self.operation.operation_id.to_string(),
                message: "provider typed error has no payload".to_string(),
            })?;
        let provider_value = runtime_from_wire(&payload, provider_heap)?;
        let caller_value =
            error_plan.materialize(&provider_value, provider_heap, caller_heap, hooks)?;
        let detached_payload = runtime_to_wire(&caller_value, caller_heap)?;
        envelope
            .as_object_mut()
            .expect("validated user exception envelope is an object")
            .insert("error".to_string(), detached_payload);
        let detached_exception =
            UserException::from_runtime_parts(exception.actual_payload_type().clone(), envelope);
        Err(replace_user_exception(error, detached_exception))
    }
}

fn replace_user_exception(error: RuntimeError, exception: UserException) -> RuntimeError {
    match error {
        RuntimeError::UserException(_) => RuntimeError::UserException(exception),
        RuntimeError::WithSource {
            source_id,
            frame,
            error,
        } => RuntimeError::WithSource {
            source_id,
            frame,
            error: Box::new(replace_user_exception(*error, exception)),
        },
        RuntimeError::WithDiagnosticFrame { frame, error } => RuntimeError::WithDiagnosticFrame {
            frame,
            error: Box::new(replace_user_exception(*error, exception)),
        },
        other => other,
    }
}

struct DirectionalMaterializationPlan<'a> {
    operation: String,
    role: &'static str,
    plan: ServiceLinkableContractPlan<'a>,
    scope: ServiceLinkableMaterializationScope,
}

impl<'a> DirectionalMaterializationPlan<'a> {
    fn new(
        ty: &'a ContractTypeRef,
        schema: &'a std::collections::BTreeMap<ContractTypeId, ContractSchemaType>,
        value_plan: &'a BoundaryValuePlan,
        detached_owner: BoundaryValueOwner,
        operation: &str,
        role: &'static str,
    ) -> Result<Self> {
        let plan = ServiceLinkableContractPlan::new(ty, schema, value_plan)
            .map_err(|error| invalid_materialization_plan(operation, role, error))?;
        let scope = directional_scope(value_plan, detached_owner).map_err(|message| {
            RuntimeError::InvalidArtifact(format!(
                "canonical service operation {operation} {role} value plan is invalid: {message}"
            ))
        })?;
        Ok(Self {
            operation: operation.to_string(),
            role,
            plan,
            scope,
        })
    }

    fn materialize(
        &self,
        value: &RuntimeValue,
        source_heap: &RequestHeap,
        destination_heap: &mut RequestHeap,
        hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        self.plan
            .materialize(value, source_heap, destination_heap, self.scope, hooks)
            .map_err(|error| RuntimeError::Protocol {
                target: self.operation.clone(),
                message: format!("{} materialization failed: {error}", self.role),
            })
    }
}

fn validate_ordinary_operation(
    operation: &BoundaryOperationDescriptor,
    call: &CallIr,
    arg_count: usize,
) -> Result<()> {
    if arg_count != operation.contract.parameters.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} expected {} parameters, got {arg_count}",
            operation.operation_id,
            operation.contract.parameters.len()
        )));
    }
    if !call.type_args.is_empty() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} cannot carry package-local type arguments",
            operation.operation_id
        )));
    }
    if !matches!(operation.contract.stream, BoundaryStreamContract::Unary)
        || !matches!(
            operation.contract.cancellation,
            BoundaryCancellationContract::NotCancellable
        )
        || operation.contract.may_suspend
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} is not an ordinary unary operation",
            operation.operation_id
        )));
    }
    match &operation.contract.callbacks {
        BoundaryCallbackContract::None => {}
        BoundaryCallbackContract::RequestScoped { lifetime, .. }
            if *lifetime == BoundaryCallbackLifetime::TopLevelRequest => {}
        BoundaryCallbackContract::RequestScoped { .. } => {
            return Err(RuntimeError::InvalidArtifact(format!(
                "canonical ordinary service operation {} cannot use stream-scoped callbacks",
                operation.operation_id
            )));
        }
        BoundaryCallbackContract::Unsupported { reason } => {
            return Err(RuntimeError::Unsupported(format!(
                "canonical service operation {} has unsupported callback semantics: {reason:?}",
                operation.operation_id
            )));
        }
    }
    let guarantee = &operation.contract.effect_guarantee;
    if !(guarantee.detached_parameters
        && guarantee.detached_return
        && guarantee.detached_error
        && guarantee.no_caller_reachable_mutation
        && guarantee.no_caller_value_escape
        && guarantee.no_same_heap_identity)
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} lacks detached in-process boundary guarantees",
            operation.operation_id
        )));
    }
    Ok(())
}

fn directional_scope(
    plan: &BoundaryValuePlan,
    detached_owner: BoundaryValueOwner,
) -> std::result::Result<ServiceLinkableMaterializationScope, &'static str> {
    match plan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            owner,
            lifetime: BoundaryValueLifetime::Call,
            ..
        } if *owner == detached_owner => Ok(ServiceLinkableMaterializationScope {
            owner: detached_owner,
            lifetime: BoundaryValueLifetime::Call,
        }),
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
            ..
        } => Ok(ServiceLinkableMaterializationScope {
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
        }),
        BoundaryValuePlan::Unsupported { .. } => Err("unsupported value plan"),
        _ => Err("owner or lifetime does not match the ordinary service direction"),
    }
}

fn invalid_materialization_plan(
    operation: &str,
    role: &str,
    error: ServiceLinkableMaterializationError,
) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "canonical service operation {operation} {role} value plan is invalid: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use skiff_artifact_model::{
        BoundaryEffectGuarantee, BoundaryParameter, BoundaryReturn, BoundaryValueEncoding,
        ContractDiagnosticText, ContractOperationId, ServiceContract, ServiceProtocolIdentity,
        SERVICE_CONTRACT_SCHEMA_VERSION,
    };
    use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
    use skiff_runtime_linked_program::{LinkedCallTarget, LinkedTypeRef};
    use skiff_runtime_model::{
        error::TypeIdentity,
        runtime_value::{HeapHandle, HeapNode},
    };

    use super::*;

    #[test]
    fn ordinary_in_process_detaches_parameters_and_return_aliases() {
        let array_type = ContractTypeRef::Builtin {
            name: "Array".to_string(),
            arguments: vec![ContractTypeRef::builtin("string")],
        };
        let operation = ordinary_operation(
            vec![array_type.clone()],
            array_type,
            BoundaryErrorContract::None,
        );
        let contract = service_contract(&operation, BTreeMap::new());
        let call = test_call();
        let mut caller_heap = RequestHeap::default();
        let source = caller_heap
            .alloc_array(vec![RuntimeValue::String("caller".to_string())])
            .expect("caller array should allocate");
        let args = vec![RuntimeValue::Heap(source)];
        let boundary = OrdinaryServiceBoundary::new(&operation, &contract, &call, &args)
            .expect("ordinary descriptor plans should validate before provider execution");
        let mut provider_heap = RequestHeap::default();

        let provider_args = boundary
            .materialize_parameters(
                &args,
                &caller_heap,
                &mut provider_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .expect("caller parameter should detach into the provider heap");
        let RuntimeValue::Heap(provider_value) = provider_args[0] else {
            panic!("provider argument should remain an array")
        };
        provider_heap
            .set_array_item(
                provider_value,
                0,
                RuntimeValue::String("provider".to_string()),
            )
            .expect("provider copy should be independently mutable");
        assert_array_item(&caller_heap, source, "caller");

        let returned = boundary
            .return_plan
            .materialize(
                &RuntimeValue::Heap(provider_value),
                &provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .expect("provider return should detach back into the caller heap");
        let RuntimeValue::Heap(returned) = returned else {
            panic!("materialized return should remain an array")
        };
        assert_ne!(
            returned, source,
            "service return must not alias the caller parameter graph"
        );
        caller_heap
            .set_array_item(returned, 0, RuntimeValue::String("receiver".to_string()))
            .expect("caller return copy should be independently mutable");
        assert_array_item(&provider_heap, provider_value, "provider");
        assert_array_item(&caller_heap, source, "caller");

        let arg_count_error = OrdinaryServiceBoundary::new(&operation, &contract, &call, &[])
            .err()
            .expect("argument-count mismatch must fail before provider execution");
        assert!(matches!(arg_count_error, RuntimeError::InvalidArtifact(_)));

        let missing_type = ContractTypeRef::contract(ContractTypeId::new("contract:missing"));
        let invalid_operation = ordinary_operation(
            vec![missing_type],
            ContractTypeRef::builtin("void"),
            BoundaryErrorContract::None,
        );
        let invalid_contract = service_contract(&invalid_operation, BTreeMap::new());
        let schema_error = OrdinaryServiceBoundary::new(
            &invalid_operation,
            &invalid_contract,
            &call,
            &[RuntimeValue::Null],
        )
        .err()
        .expect("missing contract schema must fail before provider execution");
        assert!(matches!(schema_error, RuntimeError::InvalidArtifact(_)));

        let mut generic_call = test_call();
        generic_call.type_args.insert(
            "T".to_string(),
            LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        );
        let generic_error =
            OrdinaryServiceBoundary::new(&operation, &contract, &generic_call, &args)
                .err()
                .expect("service call type arguments must fail before provider execution");
        assert!(matches!(generic_error, RuntimeError::InvalidArtifact(_)));
    }

    #[test]
    fn service_error_boundary_detaches_typed_errors_and_preserves_runtime_failures() {
        let payload_type = ContractTypeRef::Record {
            fields: BTreeMap::from([
                ("message".to_string(), ContractTypeRef::builtin("string")),
                (
                    "trace".to_string(),
                    ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    },
                ),
            ]),
        };
        let operation = ordinary_operation(
            Vec::new(),
            ContractTypeRef::builtin("void"),
            BoundaryErrorContract::Typed {
                payload_type,
                value_plan: detached_plan(BoundaryValueOwner::Provider),
            },
        );
        let contract = service_contract(&operation, BTreeMap::new());
        let boundary = OrdinaryServiceBoundary::new(&operation, &contract, &test_call(), &[])
            .expect("typed error plan should validate before provider execution");
        let identity = TypeIdentity::builtin("ProviderProblem");
        let exception = UserException::from_typed_payload(
            json!({ "message": "rejected", "trace": ["provider"] }),
            identity.clone(),
            Some(identity.clone()),
        )
        .expect("typed provider exception should build");
        let mut provider_heap = RequestHeap::default();
        let mut caller_heap = RequestHeap::default();
        let error = boundary
            .materialize_error(
                RuntimeError::UserException(exception),
                &mut provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .err()
            .expect("typed provider error should remain an error in caller context");
        let caught = user_exception_for_catch(&error)
            .expect("materialized typed error must remain caller-catchable");
        assert_eq!(caught.actual_payload_type(), &identity);
        assert_eq!(
            caught.error_payload(),
            json!({ "message": "rejected", "trace": ["provider"] }).as_object()
        );
        assert!(
            provider_heap.len() > 0 && caller_heap.len() > 0,
            "typed payload must be materialized through distinct provider and caller heaps"
        );

        let wrapped_exception = UserException::from_typed_payload(
            json!({ "message": "wrapped", "trace": ["provider"] }),
            identity.clone(),
            Some(identity.clone()),
        )
        .expect("wrapped typed provider exception should build");
        let wrapped_error = boundary
            .materialize_error(
                RuntimeError::WithDiagnosticFrame {
                    frame: Box::new(json!({ "provider": "frame" })),
                    error: Box::new(RuntimeError::UserException(wrapped_exception)),
                },
                &mut provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .err()
            .expect("typed provider diagnostics should remain attached");
        assert!(matches!(
            &wrapped_error,
            RuntimeError::WithDiagnosticFrame { .. }
        ));
        assert!(user_exception_for_catch(&wrapped_error).is_some());

        let runtime_error = boundary
            .materialize_error(
                RuntimeError::Cancelled,
                &mut provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .err()
            .expect("provider runtime failure should propagate");
        assert!(matches!(runtime_error, RuntimeError::Cancelled));

        let no_error_operation = ordinary_operation(
            Vec::new(),
            ContractTypeRef::builtin("void"),
            BoundaryErrorContract::None,
        );
        let no_error_contract = service_contract(&no_error_operation, BTreeMap::new());
        let no_error_boundary = OrdinaryServiceBoundary::new(
            &no_error_operation,
            &no_error_contract,
            &test_call(),
            &[],
        )
        .expect("ordinary no-error descriptor should validate");
        let unexpected = UserException::from_typed_payload(
            json!({ "message": "not declared" }),
            identity.clone(),
            Some(identity),
        )
        .expect("unexpected provider exception should build");
        let protocol_error = no_error_boundary
            .materialize_error(
                RuntimeError::UserException(unexpected),
                &mut provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .err()
            .expect("undeclared typed error must fail as protocol, not business error");
        assert!(matches!(protocol_error, RuntimeError::Protocol { .. }));

        let invalid_payload = UserException::from_typed_payload(
            json!({ "message": "wrong shape", "trace": "not-an-array" }),
            TypeIdentity::builtin("ProviderProblem"),
            Some(TypeIdentity::builtin("ProviderProblem")),
        )
        .expect("invalid provider payload envelope should still build");
        let invalid_payload_error = boundary
            .materialize_error(
                RuntimeError::UserException(invalid_payload),
                &mut provider_heap,
                &mut caller_heap,
                &FailClosedServiceLinkableCapabilityHooks,
            )
            .err()
            .expect("typed payload shape mismatch must fail as protocol");
        assert!(matches!(
            invalid_payload_error,
            RuntimeError::Protocol { .. }
        ));
    }

    #[test]
    fn package_direct_same_heap_preserves_handle_identity_and_mutation() {
        let mut request_heap = RequestHeap::default();
        let caller_value = request_heap
            .alloc_array(vec![RuntimeValue::String("caller".to_string())])
            .expect("package argument should allocate");

        // `execute_package_direct` forwards this value vector and this same heap without a
        // service materialization step. The callee therefore observes the exact handle.
        let package_args = vec![RuntimeValue::Heap(caller_value)];
        let RuntimeValue::Heap(callee_value) = package_args[0] else {
            panic!("package argument should remain a heap handle")
        };
        assert_eq!(callee_value, caller_value);
        request_heap
            .set_array_item(
                callee_value,
                0,
                RuntimeValue::String("package-callee".to_string()),
            )
            .expect("package callee should mutate the shared request heap");
        assert_array_item(&request_heap, caller_value, "package-callee");
    }

    fn ordinary_operation(
        parameters: Vec<ContractTypeRef>,
        return_type: ContractTypeRef,
        errors: BoundaryErrorContract,
    ) -> BoundaryOperationDescriptor {
        BoundaryOperationDescriptor {
            operation_id: ContractOperationId::new("operation:ordinary-test"),
            stable_key: "ordinaryTest".to_string(),
            contract: skiff_artifact_model::BoundaryOperationContract {
                parameters: parameters
                    .into_iter()
                    .enumerate()
                    .map(|(index, ty)| BoundaryParameter {
                        name: format!("arg{index}"),
                        ty,
                        value_plan: detached_plan(BoundaryValueOwner::Caller),
                    })
                    .collect(),
                return_value: BoundaryReturn {
                    ty: return_type,
                    value_plan: detached_plan(BoundaryValueOwner::Provider),
                },
                errors,
                stream: BoundaryStreamContract::Unary,
                cancellation: BoundaryCancellationContract::NotCancellable,
                callbacks: BoundaryCallbackContract::None,
                may_suspend: false,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        }
    }

    fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime: BoundaryValueLifetime::Call,
        }
    }

    fn service_contract(
        operation: &BoundaryOperationDescriptor,
        boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    ) -> ServiceContract {
        ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: "example.ordinary-test".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol:ordinary-test"),
            operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
            boundary_schema,
            diagnostic_text: ContractDiagnosticText {
                service: "ordinary test".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        }
    }

    fn test_call() -> CallIr {
        CallIr {
            target: LinkedCallTarget::Builtin {
                op: "ordinary-test".to_string(),
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn assert_array_item(heap: &RequestHeap, handle: HeapHandle, value: &str) {
        let HeapNode::Array(items) = heap.get(handle).expect("array handle should resolve") else {
            panic!("heap value should remain an array")
        };
        assert_eq!(items, &[RuntimeValue::String(value.to_string())]);
    }
}
