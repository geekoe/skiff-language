use std::{collections::BTreeMap, sync::Arc};

use serde_json::json;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractOperationId, ContractTypeDescriptor, ContractTypeRef, PackageSchemaCanonicalDescriptor,
    PackageSchemaTypeId, PackageSchemaTypeRecord,
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
    let invalid_operation = operation(
        vec![missing_type],
        ContractTypeRef::builtin("void"),
        BoundaryErrorContract::None,
    );
    let schema_error = CanonicalServiceBoundaryPlan::new(&invalid_operation, &schema, 1)
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
    let plan_error = CanonicalServiceBoundaryPlan::new(&invalid_plan_operation, &schema, 1)
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
    let schema = BTreeMap::new();
    let planner = CanonicalServiceBoundaryPlan::new(&descriptor, &schema, 0)
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
    let no_error_planner = CanonicalServiceBoundaryPlan::new(&no_error_operation, &schema, 0)
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

#[test]
fn package_named_parameter_return_and_typed_error_keep_full_owner_identity() {
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
    let descriptor = operation(
        vec![first_ref.clone()],
        second_ref,
        BoundaryErrorContract::Typed {
            payload_type: first_ref,
            value_plan: detached_plan(BoundaryValueOwner::Provider),
        },
    );
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
