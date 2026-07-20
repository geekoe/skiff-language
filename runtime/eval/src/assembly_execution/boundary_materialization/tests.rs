use std::collections::BTreeMap;

use serde_json::json;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractOperationId, ContractSchemaType, ContractTypeId,
    ContractTypeRef, ServiceContract, ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
use skiff_runtime_model::{
    error::TypeIdentity,
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, HeapNode, RuntimeValue},
};

use super::*;

#[test]
fn ordinary_in_process_uses_shared_planner_for_detached_parameters_and_return() {
    let array_type = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    let descriptor = operation(
        vec![array_type.clone()],
        array_type,
        BoundaryErrorContract::None,
    );
    let service_contract = contract(&descriptor, BTreeMap::new());
    let mut caller_heap = RequestHeap::default();
    let source = caller_heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller array should allocate");
    let args = vec![RuntimeValue::Heap(source)];
    let planner = CanonicalServiceBoundaryPlan::new(&descriptor, &service_contract, args.len())
        .expect("canonical descriptor plans should pass shared preflight");
    let mut provider_heap = planner.fresh_provider_heap(Default::default());

    let provider_args = planner
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

    let returned = planner
        .materialize_provider_result(
            Ok(RuntimeValue::Heap(provider_value)),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect("provider return should detach back into the caller heap");
    let RuntimeValue::Heap(returned) = returned else {
        panic!("materialized return should remain an array")
    };
    assert_ne!(returned, source, "return must not alias caller parameter");
    caller_heap
        .set_array_item(returned, 0, RuntimeValue::String("receiver".to_string()))
        .expect("receiver copy should be independently mutable");
    assert_array_item(&provider_heap, provider_value, "provider");
    assert_array_item(&caller_heap, source, "caller");

    let arg_error = CanonicalServiceBoundaryPlan::new(&descriptor, &service_contract, 0)
        .err()
        .expect("argument mismatch must fail shared preflight");
    assert!(matches!(arg_error, RuntimeError::InvalidArtifact(_)));

    let missing_type = ContractTypeRef::contract(ContractTypeId::new("contract:missing"));
    let invalid_operation = operation(
        vec![missing_type],
        ContractTypeRef::builtin("void"),
        BoundaryErrorContract::None,
    );
    let invalid_contract = contract(&invalid_operation, BTreeMap::new());
    let schema_error = CanonicalServiceBoundaryPlan::new(&invalid_operation, &invalid_contract, 1)
        .err()
        .expect("schema mismatch must fail shared preflight");
    assert!(matches!(schema_error, RuntimeError::InvalidArtifact(_)));

    let mut invalid_plan_operation = operation(
        vec![ContractTypeRef::builtin("string")],
        ContractTypeRef::builtin("void"),
        BoundaryErrorContract::None,
    );
    invalid_plan_operation.contract.parameters[0].value_plan =
        detached_plan(BoundaryValueOwner::Provider);
    let invalid_plan_contract = contract(&invalid_plan_operation, BTreeMap::new());
    let plan_error =
        CanonicalServiceBoundaryPlan::new(&invalid_plan_operation, &invalid_plan_contract, 1)
            .err()
            .expect("owner mismatch must fail shared plan preflight");
    assert!(matches!(plan_error, RuntimeError::InvalidArtifact(_)));
}

#[test]
fn service_error_boundary_classification_is_shared_across_lanes() {
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
    let descriptor = operation(
        Vec::new(),
        ContractTypeRef::builtin("void"),
        BoundaryErrorContract::Typed {
            payload_type,
            value_plan: detached_plan(BoundaryValueOwner::Provider),
        },
    );
    let service_contract = contract(&descriptor, BTreeMap::new());
    let planner = CanonicalServiceBoundaryPlan::new(&descriptor, &service_contract, 0)
        .expect("typed error plan should pass shared preflight");
    let identity = TypeIdentity::builtin("ProviderProblem");
    let mut provider_heap = planner.fresh_provider_heap(Default::default());
    let mut caller_heap = RequestHeap::default();

    let declared = typed_exception(
        json!({ "message": "rejected", "trace": ["provider"] }),
        identity.clone(),
    );
    let diagnostic_frame = json!({ "provider": "diagnostic-frame" });
    let source_frame = json!({ "sourceId": 17, "module": "provider" });
    let error = planner
        .materialize_provider_result(
            Err(RuntimeError::WithDiagnosticFrame {
                frame: Box::new(diagnostic_frame.clone()),
                error: Box::new(RuntimeError::WithSource {
                    source_id: 17,
                    frame: Box::new(source_frame.clone()),
                    error: Box::new(RuntimeError::UserException(declared)),
                }),
            }),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .err()
        .expect("declared typed error should remain an error");
    let RuntimeError::WithDiagnosticFrame {
        frame,
        error: source_error,
    } = &error
    else {
        panic!("diagnostic wrapper should remain outermost")
    };
    assert_eq!(frame.as_ref(), &diagnostic_frame);
    let RuntimeError::WithSource {
        source_id,
        frame,
        error: user_error,
    } = source_error.as_ref()
    else {
        panic!("source wrapper should remain nested inside diagnostic wrapper")
    };
    assert_eq!(*source_id, 17);
    assert_eq!(frame.as_ref(), &source_frame);
    assert!(matches!(
        user_error.as_ref(),
        RuntimeError::UserException(_)
    ));
    let caught = user_exception_for_catch(&error).expect("typed error should be caller-catchable");
    assert_eq!(caught.actual_payload_type(), &identity);
    assert_eq!(
        caught.error_payload(),
        json!({ "message": "rejected", "trace": ["provider"] }).as_object()
    );
    assert!(provider_heap.len() > 0 && caller_heap.len() > 0);

    let runtime_error = planner
        .materialize_provider_result(
            Err(RuntimeError::Cancelled),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .err()
        .expect("runtime error should propagate");
    assert!(matches!(runtime_error, RuntimeError::Cancelled));

    let invalid_payload = typed_exception(
        json!({ "message": "wrong shape", "trace": "not-an-array" }),
        identity.clone(),
    );
    let shape_error = planner
        .materialize_provider_result(
            Err(RuntimeError::UserException(invalid_payload)),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .err()
        .expect("typed payload shape mismatch should be protocol error");
    assert!(matches!(
        shape_error,
        RuntimeError::Protocol { ref target, .. }
            if target == "operation:shared-boundary-test"
    ));

    let no_error_operation = operation(
        Vec::new(),
        ContractTypeRef::builtin("void"),
        BoundaryErrorContract::None,
    );
    let no_error_contract = contract(&no_error_operation, BTreeMap::new());
    let no_error_planner =
        CanonicalServiceBoundaryPlan::new(&no_error_operation, &no_error_contract, 0)
            .expect("no-error descriptor should pass shared preflight");
    let undeclared = no_error_planner
        .materialize_provider_result(
            Err(RuntimeError::UserException(typed_exception(
                json!({ "message": "undeclared" }),
                identity,
            ))),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .err()
        .expect("undeclared typed throw should be protocol error");
    assert!(matches!(
        undeclared,
        RuntimeError::Protocol { ref target, .. }
            if target == "operation:shared-boundary-test"
    ));
}

fn operation(
    parameters: Vec<ContractTypeRef>,
    return_type: ContractTypeRef,
    errors: BoundaryErrorContract,
) -> BoundaryOperationDescriptor {
    BoundaryOperationDescriptor {
        operation_id: ContractOperationId::new("operation:shared-boundary-test"),
        stable_key: "sharedBoundaryTest".to_string(),
        contract: BoundaryOperationContract {
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

fn contract(
    operation: &BoundaryOperationDescriptor,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
) -> ServiceContract {
    ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.shared-boundary-test".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("protocol:shared-boundary-test"),
        operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: "shared boundary test".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    }
}

fn typed_exception(payload: serde_json::Value, identity: TypeIdentity) -> UserException {
    UserException::from_typed_payload(payload, identity.clone(), Some(identity))
        .expect("typed test exception should build")
}

fn assert_array_item(heap: &RequestHeap, handle: HeapHandle, value: &str) {
    let HeapNode::Array(items) = heap.get(handle).expect("array handle should resolve") else {
        panic!("heap value should remain an array")
    };
    assert_eq!(items, &[RuntimeValue::String(value.to_string())]);
}
