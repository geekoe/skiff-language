use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractOperationId, ContractTypeDescriptor,
    ContractTypeRef, PackageSchemaCanonicalDescriptor, PackageSchemaTypeId,
    PackageSchemaTypeRecord,
};
use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, HeapNode, RuntimeValue},
    service_error::{InternalErrorPayload, OpaqueServiceError, ServiceErrorEnvelope},
};

use super::*;

#[test]
fn ordinary_in_process_uses_shared_planner_for_detached_parameters_and_return() {
    let array_type = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    let descriptor = operation(vec![array_type.clone()], array_type);
    let schema = BTreeMap::new();
    let mut caller_heap = RequestHeap::default();
    let source = caller_heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller array should allocate");
    let args = vec![RuntimeValue::Heap(source)];
    let planner = CanonicalServiceBoundaryPlan::new(&descriptor, &schema, args.len())
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

    let arg_error = CanonicalServiceBoundaryPlan::new(&descriptor, &schema, 0)
        .err()
        .expect("argument mismatch must fail shared preflight");
    assert!(matches!(arg_error, RuntimeError::InvalidArtifact(_)));

    let missing_type = ContractTypeRef::package_schema(
        "example.missing",
        "api.Missing",
        PackageSchemaTypeId::new("schema:missing"),
    );
    let invalid_operation = operation(vec![missing_type], ContractTypeRef::builtin("void"));
    let schema_error = CanonicalServiceBoundaryPlan::new(&invalid_operation, &schema, 1)
        .err()
        .expect("schema mismatch must fail shared preflight");
    assert!(matches!(schema_error, RuntimeError::InvalidArtifact(_)));

    let mut invalid_plan_operation = operation(
        vec![ContractTypeRef::builtin("string")],
        ContractTypeRef::builtin("void"),
    );
    invalid_plan_operation.contract.parameters[0].value_plan =
        detached_plan(BoundaryValueOwner::Provider);
    let plan_error = CanonicalServiceBoundaryPlan::new(&invalid_plan_operation, &schema, 1)
        .err()
        .expect("owner mismatch must fail shared plan preflight");
    assert!(matches!(plan_error, RuntimeError::InvalidArtifact(_)));
}

#[test]
fn package_named_parameter_and_return_keep_full_owner_identity() {
    let first = package_record("example.first", "api.Payload", "schema:first");
    let second = package_record("example.second", "api.Payload", "schema:second");
    let first_ref = ContractTypeRef::package_schema(
        first.package_id.clone(),
        first.stable_schema_key.clone(),
        first.package_schema_type_id.clone(),
    );
    let second_ref = ContractTypeRef::package_schema(
        second.package_id.clone(),
        second.stable_schema_key.clone(),
        second.package_schema_type_id.clone(),
    );
    let descriptor = operation(vec![first_ref], second_ref);
    let schema = BTreeMap::from([
        (
            first.package_schema_type_id.clone(),
            Arc::new(first.clone()),
        ),
        (
            second.package_schema_type_id.clone(),
            Arc::new(second.clone()),
        ),
    ]);
    CanonicalServiceBoundaryPlan::new(&descriptor, &schema, 1)
        .expect("same stable key from different Package owners must remain isolated");

    let mut wrong_owner = second;
    wrong_owner.package_id = "example.first".to_string();
    let invalid = BTreeMap::from([
        (first.package_schema_type_id.clone(), Arc::new(first)),
        (
            wrong_owner.package_schema_type_id.clone(),
            Arc::new(wrong_owner),
        ),
    ]);
    assert!(matches!(
        CanonicalServiceBoundaryPlan::new(&descriptor, &invalid, 1),
        Err(RuntimeError::InvalidArtifact(_))
    ));
}

#[test]
fn provider_result_accepts_only_an_already_fixed_failure_carrier() {
    let descriptor = operation(Vec::new(), ContractTypeRef::builtin("void"));
    let schema = BTreeMap::new();
    let planner = CanonicalServiceBoundaryPlan::new(&descriptor, &schema, 0)
        .expect("ordinary no-argument plan");
    let mut provider_heap = RequestHeap::default();
    let mut caller_heap = RequestHeap::default();

    let generic = planner
        .materialize_provider_result(
            Err(RuntimeError::ProviderUnavailable {
                target: "provider".to_string(),
                reason: "generic failure".to_string(),
            }),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect_err("generic provider error cannot bypass fixed classification");
    assert!(matches!(generic, RuntimeError::InvalidArtifact(_)));

    let envelope = ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "Internal service error".to_string(),
            trace_id: "trace-fixed".to_string(),
            error_id: "error-fixed".to_string(),
        },
    };
    let fixed = OpaqueServiceError::decode(
        skiff_canonical_json::canonical_json_bytes(&envelope).expect("fixed carrier bytes"),
    )
    .expect("fixed carrier");
    let forwarded = planner
        .materialize_provider_result(
            Err(RuntimeError::FixedServiceFailure(fixed.clone())),
            &mut provider_heap,
            &mut caller_heap,
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect_err("fixed failure remains on the error path");
    assert!(matches!(
        forwarded,
        RuntimeError::FixedServiceFailure(actual) if actual == fixed
    ));
}

fn operation(
    parameters: Vec<ContractTypeRef>,
    return_type: ContractTypeRef,
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

fn package_record(
    package_id: &str,
    stable_schema_key: &str,
    type_id: &str,
) -> PackageSchemaTypeRecord {
    PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id: PackageSchemaTypeId::new(type_id),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        },
    }
}

fn assert_array_item(heap: &RequestHeap, handle: HeapHandle, value: &str) {
    let HeapNode::Array(items) = heap.get(handle).expect("array handle should resolve") else {
        panic!("heap value should remain an array")
    };
    assert_eq!(items, &[RuntimeValue::String(value.to_string())]);
}
