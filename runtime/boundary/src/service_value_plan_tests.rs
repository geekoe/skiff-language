use std::{collections::BTreeMap, sync::Arc};

use serde_json::{json, Value};
use skiff_artifact_model::{
    BoundaryCallbackOperation, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaCanonicalDescriptor, PackageSchemaTypeId, PackageSchemaTypeRecord,
    WEBSOCKET_CONNECT_RESULT_TYPE, WEBSOCKET_INGRESS_EVENT_TYPE,
};
use skiff_runtime_model::value::{
    CallbackCapabilityCarrier, HeapNode, InterfaceCarrier, InterfaceValue, RuntimeObject,
    RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
};

use crate::{
    date_value::{MAX_EPOCH_MILLIS, MIN_EPOCH_MILLIS},
    payload::{PayloadBoundary, PayloadBoundaryKind},
    request_heap::RequestHeap,
    service_linkable::{
        FailClosedServiceLinkableCapabilityHooks, ServiceLinkableContractPlan,
        ServiceLinkableMaterializationError, ServiceLinkableMaterializationScope,
    },
    service_value_plan::ServiceValuePlan,
};

const CONTEXT_ID: &str = "contract-type:Context";
const USER_ID: &str = "contract-type:UserId";
const PACKAGE_ID: &str = "test.boundary";
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

fn generic(name: &str, arguments: Vec<ContractTypeRef>) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: name.to_string(),
        arguments,
    }
}

fn websocket_event(context: ContractTypeRef) -> ContractTypeRef {
    generic(WEBSOCKET_INGRESS_EVENT_TYPE, vec![context])
}

fn websocket_result(context: ContractTypeRef) -> ContractTypeRef {
    generic(WEBSOCKET_CONNECT_RESULT_TYPE, vec![context])
}

fn schema_type(
    id: &str,
    stable_key: &str,
    descriptor: ContractTypeDescriptor,
) -> (PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>) {
    let id = PackageSchemaTypeId::new(id);
    (
        id.clone(),
        Arc::new(PackageSchemaTypeRecord {
            package_id: PACKAGE_ID.to_string(),
            package_schema_type_id: id,
            stable_schema_key: stable_key.to_string(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor,
            },
        }),
    )
}

fn rich_context_schema() -> BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>> {
    let user_id = PackageSchemaTypeId::new(USER_ID);
    BTreeMap::from([
        schema_type(
            USER_ID,
            "UserId",
            ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin("string"),
            },
        ),
        schema_type(
            CONTEXT_ID,
            "Context",
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    (
                        "attributes".to_string(),
                        ContractTypeRef::builtin("JsonObject"),
                    ),
                    ("createdAt".to_string(), ContractTypeRef::builtin("Date")),
                    ("empty".to_string(), ContractTypeRef::builtin("bytes")),
                    (
                        "labels".to_string(),
                        generic(
                            "Map",
                            vec![package_ref(user_id), ContractTypeRef::builtin("string")],
                        ),
                    ),
                    ("ttl".to_string(), ContractTypeRef::builtin("Duration")),
                ]),
            },
        ),
    ])
}

#[test]
fn shared_schema_record_is_reused_by_multiple_contract_plans_without_payload_clones() {
    let (id, record) = schema_type(
        CONTEXT_ID,
        "Context",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("name".to_string(), ContractTypeRef::builtin("string"))]),
        },
    );
    let schema = Arc::new(BTreeMap::from([(id.clone(), Arc::clone(&record))]));
    let first_contract = package_ref(id.clone());
    let second_contract = package_ref(id);
    let record_owners_before = Arc::strong_count(&record);

    let first = ServiceValuePlan::compile(&first_contract, schema.as_ref())
        .expect("first plan should borrow the admitted shared record");
    let second = ServiceValuePlan::compile(&second_contract, schema.as_ref())
        .expect("second plan should borrow the same admitted shared record");

    assert_eq!(Arc::strong_count(&record), record_owners_before);
    let _compiled_plans = (first, second);
    assert!(Arc::ptr_eq(
        schema
            .get(&PackageSchemaTypeId::new(CONTEXT_ID))
            .expect("shared record should remain admitted"),
        &record,
    ));
}

fn rich_context_ref() -> ContractTypeRef {
    package_ref(PackageSchemaTypeId::new(CONTEXT_ID))
}

fn package_ref(id: PackageSchemaTypeId) -> ContractTypeRef {
    let stable_key = id
        .as_str()
        .rsplit(':')
        .next()
        .expect("test package schema id has a stable-key suffix");
    ContractTypeRef::package_schema(PACKAGE_ID, stable_key, id.clone())
}

fn rich_context_json() -> Value {
    json!({
        "attributes": {
            "nested": [null, true, 3.5, {"message": "ok"}]
        },
        "createdAt": "1970-01-01T00:00:00.000Z",
        "empty": {"__skiffBytesBase64": ""},
        "labels": {"user-1": "owner"},
        "ttl": 250
    })
}

fn connect_event_json() -> Value {
    json!({
        "tag": "connect",
        "connectRequest": {
            "connectionId": "connection-1",
            "url": "wss://example.test/chat?room=one",
            "query": [{"name": "room", "value": "one"}],
            "headers": [{"name": "x-request-id", "value": "request-1"}],
            "cookies": [{"name": "session", "value": "abc"}],
            "version": null
        }
    })
}

fn receive_event_json(context: Value) -> Value {
    json!({
        "tag": "receive",
        "receiveEvent": {
            "connection": {
                "id": "connection-1",
                "businessIdentity": null,
                "context": context
            },
            "message": {"tag": "binary", "base64": "AAEC"}
        }
    })
}

fn receive_text_event_json(context: Value) -> Value {
    json!({
        "tag": "receive",
        "receiveEvent": {
            "connection": {
                "id": "connection-1",
                "businessIdentity": "tenant-1",
                "context": context
            },
            "message": {"tag": "text", "text": "hello"}
        }
    })
}

fn accept_result_json(context: Value) -> Value {
    json!({
        "tag": "accept",
        "context": context,
        "businessIdentity": "tenant-1",
        "connectionPolicy": {
            "maxConnections": 4,
            "overflow": "close-oldest",
            "closeCode": null,
            "closeReason": null
        }
    })
}

fn reject_result_json() -> Value {
    json!({"tag": "reject", "code": 1008, "reason": "policy"})
}

fn websocket_boundary() -> PayloadBoundary {
    PayloadBoundary::external_untrusted(PayloadBoundaryKind::WebsocketRequest)
}

#[test]
fn canonical_http_value_plans_preserve_exact_detached_fields() {
    let schema = BTreeMap::new();
    let request_type =
        ContractTypeRef::builtin(skiff_artifact_model::http_boundary::HTTP_REQUEST_TYPE);
    let response_type =
        ContractTypeRef::builtin(skiff_artifact_model::http_boundary::HTTP_RESPONSE_TYPE);
    let stream_type = ContractTypeRef::builtin(
        skiff_artifact_model::http_boundary::HTTP_RESPONSE_STREAM_EVENT_TYPE,
    );
    let request = ServiceValuePlan::compile(&request_type, &schema).unwrap();
    let response = ServiceValuePlan::compile(&response_type, &schema).unwrap();
    let stream = ServiceValuePlan::compile(&stream_type, &schema).unwrap();

    let mut caller_heap = RequestHeap::default();
    let request_wire = json!({
        "method": "POST",
        "url": "https://example.test/items?id=7",
        "path": "/items",
        "query": [{"name": "id", "value": "7"}],
        "headers": [{"name": "content-type", "value": "application/octet-stream"}],
        "body": {"__skiffBytesBase64": "AQID"}
    });
    let request_value = request
        .decode_json_value(&request_wire, &mut caller_heap)
        .unwrap();
    assert_eq!(
        request
            .encode_json_value(&request_value, &mut caller_heap)
            .unwrap(),
        request_wire
    );
    let request_contract = detached_plan(BoundaryValueOwner::Caller);
    let request_materializer =
        ServiceLinkableContractPlan::new(&request_type, &schema, &request_contract).unwrap();
    let mut provider_heap = RequestHeap::default();
    let provider_request = request_materializer
        .materialize(
            &request_value,
            &caller_heap,
            &mut provider_heap,
            detached_scope(BoundaryValueOwner::Caller),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();
    caller_heap
        .set_object_field(
            request_value.as_heap_handle().unwrap(),
            "method".to_string(),
            RuntimeValue::String("DELETE".to_string()),
        )
        .unwrap();
    assert_eq!(
        request
            .encode_json_value(&provider_request, &mut provider_heap)
            .unwrap(),
        request_wire,
        "provider request must be detached from caller heap mutation"
    );

    let response_wire = json!({
        "status": 201,
        "headers": [{"name": "x-result", "value": "created"}],
        "body": {"__skiffBytesBase64": "BAUG"}
    });
    let response_value = response
        .decode_json_value(&response_wire, &mut caller_heap)
        .unwrap();
    assert_eq!(
        response
            .encode_json_value(&response_value, &mut caller_heap)
            .unwrap(),
        response_wire
    );

    for event in [
        json!({
            "tag": "start",
            "status": 200,
            "headers": [{"name": "content-type", "value": "application/octet-stream"}]
        }),
        json!({"tag": "chunk", "value": {"__skiffBytesBase64": "BwgJ"}}),
        json!({"tag": "end"}),
    ] {
        let value = stream.decode_json_value(&event, &mut caller_heap).unwrap();
        assert_eq!(
            stream.encode_json_value(&value, &mut caller_heap).unwrap(),
            event
        );
    }

    for malformed in [
        json!({"method": "GET", "url": "/", "path": "/", "query": [], "headers": []}),
        json!({"status": 200, "headers": [], "body": {"capability": "socket"}}),
        json!({"tag": "end", "capability": "file"}),
    ] {
        assert!(
            request
                .decode_json_value(&malformed, &mut caller_heap)
                .is_err()
                && response
                    .decode_json_value(&malformed, &mut caller_heap)
                    .is_err()
                && stream
                    .decode_json_value(&malformed, &mut caller_heap)
                    .is_err()
        );
    }
}

fn assert_json_binary_round_trip(
    plan: &ServiceValuePlan<'_>,
    expected: &Value,
) -> (RequestHeap, RuntimeValue) {
    let mut source = RequestHeap::default();
    let value = plan
        .decode_json_value(expected, &mut source)
        .expect("canonical JSON should decode from the service-value plan");
    assert_eq!(
        plan.encode_json_value(&value, &mut source)
            .expect("canonical JSON should encode from the same plan"),
        *expected
    );

    let bytes = plan
        .encode_binary(&value, &websocket_boundary(), &source)
        .expect("canonical binary should encode from the same plan");
    let mut decoded_heap = RequestHeap::default();
    let decoded = plan
        .decode_binary(&bytes, &websocket_boundary(), &mut decoded_heap)
        .expect("canonical binary should decode from the same plan");
    assert_eq!(
        plan.encode_json_value(&decoded, &mut decoded_heap)
            .expect("binary round trip should retain canonical JSON"),
        *expected
    );
    (decoded_heap, decoded)
}

#[test]
fn service_value_plan_closes_websocket_connect_receive_accept_and_reject() {
    let schema = rich_context_schema();
    let context = rich_context_ref();
    let event_type = websocket_event(context.clone());
    let result_type = websocket_result(context);
    let event_plan = ServiceValuePlan::compile(&event_type, &schema).unwrap();
    let result_plan = ServiceValuePlan::compile(&result_type, &schema).unwrap();

    for event in [
        connect_event_json(),
        receive_event_json(rich_context_json()),
        receive_text_event_json(rich_context_json()),
    ] {
        assert_json_binary_round_trip(&event_plan, &event);
    }
    for result in [
        accept_result_json(rich_context_json()),
        reject_result_json(),
    ] {
        assert_json_binary_round_trip(&result_plan, &result);
    }

    let null_schema = BTreeMap::new();
    let null_event_type = websocket_event(ContractTypeRef::builtin("null"));
    let null_result_type = websocket_result(ContractTypeRef::builtin("null"));
    let null_event_plan = ServiceValuePlan::compile(&null_event_type, &null_schema).unwrap();
    let null_result_plan = ServiceValuePlan::compile(&null_result_type, &null_schema).unwrap();
    assert_json_binary_round_trip(&null_event_plan, &receive_event_json(Value::Null));
    assert_json_binary_round_trip(&null_result_plan, &accept_result_json(Value::Null));
}

fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn detached_scope(owner: BoundaryValueOwner) -> ServiceLinkableMaterializationScope {
    ServiceLinkableMaterializationScope {
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

#[test]
fn service_value_plan_detaches_nominal_record_both_directions_with_erased_values() {
    let schema = rich_context_schema();
    let context_type = rich_context_ref();
    let value_plan = ServiceValuePlan::compile(&context_type, &schema).unwrap();
    let mut caller_heap = RequestHeap::default();
    let caller_value = value_plan
        .decode_json_value(&rich_context_json(), &mut caller_heap)
        .unwrap();
    let caller_root = caller_value.as_heap_handle().unwrap();

    let caller_contract = detached_plan(BoundaryValueOwner::Caller);
    let caller_to_provider =
        ServiceLinkableContractPlan::new(&context_type, &schema, &caller_contract).unwrap();
    let mut provider_heap = RequestHeap::default();
    let provider_value = caller_to_provider
        .materialize(
            &caller_value,
            &caller_heap,
            &mut provider_heap,
            detached_scope(BoundaryValueOwner::Caller),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();
    let provider_root = provider_value.as_heap_handle().unwrap();
    provider_heap
        .set_object_field(
            provider_root,
            "ttl".to_string(),
            RuntimeValue::Number(999.0),
        )
        .unwrap();
    assert_eq!(
        value_plan
            .encode_json_value(&caller_value, &mut caller_heap)
            .unwrap(),
        rich_context_json(),
        "provider mutation must not reach the caller heap"
    );
    provider_heap
        .set_object_field(
            provider_root,
            "ttl".to_string(),
            RuntimeValue::Number(250.0),
        )
        .unwrap();
    assert!(matches!(
        caller_heap.get(caller_root).unwrap(),
        HeapNode::Object(_)
    ));

    let provider_contract = detached_plan(BoundaryValueOwner::Provider);
    let provider_to_caller =
        ServiceLinkableContractPlan::new(&context_type, &schema, &provider_contract).unwrap();
    let mut returned_heap = RequestHeap::default();
    let returned_value = provider_to_caller
        .materialize(
            &provider_value,
            &provider_heap,
            &mut returned_heap,
            detached_scope(BoundaryValueOwner::Provider),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();
    assert_eq!(
        value_plan
            .encode_json_value(&returned_value, &mut returned_heap)
            .unwrap(),
        rich_context_json()
    );

    let HeapNode::Object(context) = returned_heap
        .get(returned_value.as_heap_handle().unwrap())
        .unwrap()
    else {
        panic!("nominal Context must remain a detached record object");
    };
    assert_eq!(context.fields()["ttl"], RuntimeValue::Number(250.0));
    assert_eq!(context.fields()["createdAt"], RuntimeValue::Date(0));
    let RuntimeValue::Heap(empty) = context.fields()["empty"] else {
        panic!("zero-byte field must remain present as bytes");
    };
    assert!(matches!(
        returned_heap.get(empty).unwrap(),
        HeapNode::Bytes(bytes) if bytes.is_empty()
    ));
    let RuntimeValue::Heap(attributes) = context.fields()["attributes"] else {
        panic!("JsonObject must remain present as a map");
    };
    assert!(matches!(
        returned_heap.get(attributes).unwrap(),
        HeapNode::Map(_)
    ));
    let RuntimeValue::Heap(labels) = context.fields()["labels"] else {
        panic!("representation-keyed Map must remain present");
    };
    let HeapNode::Map(labels) = returned_heap.get(labels).unwrap() else {
        panic!("representation-keyed value must be a runtime map");
    };
    assert_eq!(
        labels.get(&RuntimeValueKey::string("user-1")),
        Some(&RuntimeValue::String("owner".to_string()))
    );
}

#[test]
fn service_value_plan_rejects_websocket_shape_and_value_mutations() {
    let schema = rich_context_schema();
    let event_type = websocket_event(rich_context_ref());
    let result_type = websocket_result(rich_context_ref());
    let event_plan = ServiceValuePlan::compile(&event_type, &schema).unwrap();
    let result_plan = ServiceValuePlan::compile(&result_type, &schema).unwrap();

    let mut mutations = Vec::new();

    let mut missing_nullable = connect_event_json();
    missing_nullable["connectRequest"]
        .as_object_mut()
        .unwrap()
        .remove("version");
    mutations.push((&event_plan, missing_nullable, "missing nullable field"));

    let mut extra = connect_event_json();
    extra["connectRequest"]
        .as_object_mut()
        .unwrap()
        .insert("legacy".to_string(), json!(true));
    mutations.push((&event_plan, extra, "extra nested field"));

    let mut wrong_tag = connect_event_json();
    wrong_tag["tag"] = json!("legacy-connect");
    mutations.push((&event_plan, wrong_tag, "wrong event tag"));

    let mut wrong_message_tag = receive_event_json(rich_context_json());
    wrong_message_tag["receiveEvent"]["message"]["tag"] = json!("text");
    mutations.push((
        &event_plan,
        wrong_message_tag,
        "message tag/payload mismatch",
    ));

    let mut missing_context = receive_event_json(rich_context_json());
    missing_context["receiveEvent"]["connection"]
        .as_object_mut()
        .unwrap()
        .remove("context");
    mutations.push((&event_plan, missing_context, "missing nominal Context"));

    let mut reserved_json = receive_event_json(rich_context_json());
    reserved_json["receiveEvent"]["connection"]["context"]["attributes"]
        .as_object_mut()
        .unwrap()
        .insert("__skiffType".to_string(), json!("legacy.Context"));
    mutations.push((&event_plan, reserved_json, "reserved legacy JSON metadata"));

    let mut wrong_date = receive_event_json(rich_context_json());
    wrong_date["receiveEvent"]["connection"]["context"]["createdAt"] =
        json!("10000-01-01T00:00:00.000Z");
    mutations.push((&event_plan, wrong_date, "Date outside canonical range"));

    let mut unsafe_duration = accept_result_json(rich_context_json());
    unsafe_duration["context"]["ttl"] = json!(9_007_199_254_740_992_u64);
    mutations.push((&result_plan, unsafe_duration, "unsafe Duration"));

    let mut fractional_integer = reject_result_json();
    fractional_integer["code"] = json!(1008.5);
    mutations.push((&result_plan, fractional_integer, "fractional integer"));

    let mut unsafe_integer = reject_result_json();
    unsafe_integer["code"] = json!(9_007_199_254_740_992_u64);
    mutations.push((&result_plan, unsafe_integer, "unsafe integer"));

    let mut wrong_policy_tag = accept_result_json(rich_context_json());
    wrong_policy_tag["connectionPolicy"]["overflow"] = json!("drop-new");
    mutations.push((&result_plan, wrong_policy_tag, "wrong policy literal"));

    let mut wrong_result_tag = reject_result_json();
    wrong_result_tag["tag"] = json!("legacy-reject");
    mutations.push((&result_plan, wrong_result_tag, "wrong result tag"));

    let mut missing_result_nullable = accept_result_json(rich_context_json());
    missing_result_nullable["connectionPolicy"]
        .as_object_mut()
        .unwrap()
        .remove("closeReason");
    mutations.push((
        &result_plan,
        missing_result_nullable,
        "missing result nullable field",
    ));

    let mut extra_result = reject_result_json();
    extra_result
        .as_object_mut()
        .unwrap()
        .insert("legacy".to_string(), json!(true));
    mutations.push((&result_plan, extra_result, "extra result field"));

    for (plan, mutation, label) in mutations {
        let mut heap = RequestHeap::default();
        let before = heap.len();
        assert!(
            plan.decode_json_value(&mutation, &mut heap).is_err(),
            "{label} must fail closed"
        );
        assert_eq!(heap.len(), before, "{label} must roll back allocations");
    }

    let mut valid_heap = RequestHeap::default();
    let valid = result_plan
        .decode_json_value(&reject_result_json(), &mut valid_heap)
        .unwrap();
    let binary = result_plan
        .encode_binary(&valid, &websocket_boundary(), &valid_heap)
        .unwrap();
    let binary_mutations = [
        {
            let mut bytes = binary.clone();
            bytes[5] = u8::MAX;
            bytes
        },
        {
            let mut bytes = binary.clone();
            bytes.push(0);
            bytes
        },
        binary[..binary.len() - 1].to_vec(),
    ];
    for mutation in binary_mutations {
        let mut heap = RequestHeap::default();
        assert!(result_plan
            .decode_binary(&mutation, &websocket_boundary(), &mut heap)
            .is_err());
        assert_eq!(heap.len(), 0, "binary mutation must roll back allocations");
    }

    let integer_type = ContractTypeRef::builtin("integer");
    let integer_plan = ServiceValuePlan::compile(&integer_type, &BTreeMap::new()).unwrap();
    let mut unsafe_binary = b"SKPV".to_vec();
    unsafe_binary.extend_from_slice(&[2, 3]);
    unsafe_binary.extend_from_slice(&(MAX_SAFE_INTEGER + 1.0).to_le_bytes());
    assert!(matches!(
        integer_plan.decode_binary(
            &unsafe_binary,
            &websocket_boundary(),
            &mut RequestHeap::default()
        ),
        Err(ServiceLinkableMaterializationError::TypeMismatch)
    ));
}

#[test]
fn service_value_plan_rejects_alias_callback_cycle_foreign_and_invalid_map_key() {
    let alias_id = PackageSchemaTypeId::new("contract-type:Alias");
    let alias_schema = BTreeMap::from([schema_type(
        alias_id.as_str(),
        "Alias",
        ContractTypeDescriptor::Alias {
            target: ContractTypeRef::builtin("string"),
        },
    )]);
    assert!(matches!(
        ServiceValuePlan::compile(&package_ref(alias_id), &alias_schema),
        Err(ServiceLinkableMaterializationError::AliasSchema { .. })
    ));

    let callback_id = PackageSchemaTypeId::new("contract-type:Callback");
    let callback_schema = BTreeMap::from([schema_type(
        callback_id.as_str(),
        "Callback",
        ContractTypeDescriptor::CallbackInterface {
            operations: BTreeMap::from([(
                "read".to_string(),
                BoundaryCallbackOperation {
                    parameters: Vec::new(),
                    return_type: ContractTypeRef::builtin("string"),
                    may_suspend: false,
                },
            )]),
        },
    )]);
    assert!(matches!(
        ServiceValuePlan::compile(&package_ref(callback_id), &callback_schema),
        Err(ServiceLinkableMaterializationError::CallbackInterfaceSchema { .. })
    ));

    let cycle_id = PackageSchemaTypeId::new("contract-type:Cycle");
    let cycle_schema = BTreeMap::from([schema_type(
        cycle_id.as_str(),
        "Cycle",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("self".to_string(), package_ref(cycle_id.clone()))]),
        },
    )]);
    assert!(matches!(
        ServiceValuePlan::compile(&package_ref(cycle_id), &cycle_schema),
        Err(ServiceLinkableMaterializationError::CyclicSchema { .. })
    ));

    assert!(matches!(
        ServiceValuePlan::compile(
            &package_ref(PackageSchemaTypeId::new("contract-type:Foreign")),
            &BTreeMap::new()
        ),
        Err(ServiceLinkableMaterializationError::MissingSchema { .. })
    ));

    let requested = PackageSchemaTypeId::new("contract-type:Requested");
    let identity_mismatch = BTreeMap::from([(
        requested.clone(),
        schema_type(
            "contract-type:Actual",
            "Actual",
            ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        )
        .1,
    )]);
    assert!(matches!(
        ServiceValuePlan::compile(&package_ref(requested), &identity_mismatch),
        Err(ServiceLinkableMaterializationError::SchemaIdentityMismatch { .. })
    ));

    let exact_id = PackageSchemaTypeId::new("contract-type:Exact");
    let exact_schema = BTreeMap::from([schema_type(
        exact_id.as_str(),
        "Exact",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    )]);
    for mismatched_ref in [
        ContractTypeRef::package_schema("other.package", "Exact", exact_id.clone()),
        ContractTypeRef::package_schema(PACKAGE_ID, "Renamed", exact_id.clone()),
    ] {
        assert!(matches!(
            ServiceValuePlan::compile(&mismatched_ref, &exact_schema),
            Err(ServiceLinkableMaterializationError::SchemaOwnerOrKeyMismatch { .. })
        ));
    }
    let cached_then_mismatched = ContractTypeRef::Record {
        fields: BTreeMap::from([
            ("aExact".to_string(), package_ref(exact_id.clone())),
            (
                "zWrongOwner".to_string(),
                ContractTypeRef::package_schema("other.package", "Exact", exact_id.clone()),
            ),
        ]),
    };
    assert!(matches!(
        ServiceValuePlan::compile(&cached_then_mismatched, &exact_schema),
        Err(ServiceLinkableMaterializationError::SchemaOwnerOrKeyMismatch { .. })
    ));
    let exact_ref = package_ref(exact_id.clone());
    let exact_plan = ServiceValuePlan::compile(&exact_ref, &exact_schema).unwrap();
    let expected_identity = format!("package-schema:{PACKAGE_ID}:Exact:{exact_id}");
    assert_eq!(
        exact_plan.runtime_type_plan().identity.nominal.as_deref(),
        Some(expected_identity.as_str()),
        "runtime nominal identity must retain Package owner, stable key and type id"
    );

    let invalid_map = generic(
        "Map",
        vec![
            ContractTypeRef::builtin("number"),
            ContractTypeRef::builtin("string"),
        ],
    );
    assert!(matches!(
        ServiceValuePlan::compile(&invalid_map, &BTreeMap::new()),
        Err(ServiceLinkableMaterializationError::InvalidContractPlan { .. })
    ));

    let numeric_key_id = PackageSchemaTypeId::new("contract-type:NumericKey");
    let numeric_key_schema = BTreeMap::from([schema_type(
        numeric_key_id.as_str(),
        "NumericKey",
        ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin("number"),
        },
    )]);
    let invalid_nominal_map = generic(
        "Map",
        vec![
            package_ref(numeric_key_id),
            ContractTypeRef::builtin("string"),
        ],
    );
    assert!(ServiceValuePlan::compile(&invalid_nominal_map, &numeric_key_schema).is_err());

    assert!(ServiceValuePlan::compile(
        &ContractTypeRef::builtin("legacy.Unknown"),
        &BTreeMap::new()
    )
    .is_err());
    assert!(ServiceValuePlan::compile(
        &websocket_event(ContractTypeRef::builtin("string")),
        &BTreeMap::new()
    )
    .is_err());
}

#[test]
fn service_value_plan_preserves_exact_any_interface_identity_and_fails_wire_closed() {
    let interface_id = PackageSchemaTypeId::new("contract-type:Reader");
    let interface_schema = BTreeMap::from([schema_type(
        interface_id.as_str(),
        "Reader",
        ContractTypeDescriptor::CallbackInterface {
            operations: BTreeMap::from([(
                "read".to_string(),
                BoundaryCallbackOperation {
                    parameters: Vec::new(),
                    return_type: ContractTypeRef::builtin("string"),
                    may_suspend: false,
                },
            )]),
        },
    )]);
    let existential = ContractTypeRef::AnyInterface {
        interface: Box::new(package_ref(interface_id)),
        arguments: Vec::new(),
    };

    let plan = ServiceValuePlan::compile(&existential, &interface_schema)
        .expect("exact callback interface existential should compile");
    assert_eq!(
        plan.runtime_type_plan().identity.interface.as_deref(),
        Some("package-schema:test.boundary:Reader:contract-type:Reader")
    );
    assert!(matches!(
        plan.runtime_type_plan().node(),
        skiff_runtime_model::type_plan::RuntimeTypeNode::Unknown
    ));
    assert!(plan
        .decode_json_value(&json!({}), &mut RequestHeap::default())
        .is_err());

    let non_interface_id = PackageSchemaTypeId::new("contract-type:NotReader");
    let non_interface_schema = BTreeMap::from([schema_type(
        non_interface_id.as_str(),
        "NotReader",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    )]);
    let invalid = ContractTypeRef::AnyInterface {
        interface: Box::new(package_ref(non_interface_id)),
        arguments: Vec::new(),
    };
    assert!(matches!(
        ServiceValuePlan::compile(&invalid, &non_interface_schema),
        Err(ServiceLinkableMaterializationError::InvalidContractPlan { .. })
    ));
}

#[test]
fn service_value_plan_uses_expected_type_for_null_nominal_and_zero_byte_values() {
    let empty_id = PackageSchemaTypeId::new("contract-type:EmptyContext");
    let empty_schema = BTreeMap::from([schema_type(
        empty_id.as_str(),
        "EmptyContext",
        ContractTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    )]);
    let nominal_type = package_ref(empty_id);
    let nominal_plan = ServiceValuePlan::compile(&nominal_type, &empty_schema).unwrap();
    let null_type = ContractTypeRef::builtin("null");
    let null_plan = ServiceValuePlan::compile(&null_type, &BTreeMap::new()).unwrap();

    assert!(nominal_plan
        .decode_json_value(&json!({}), &mut RequestHeap::default())
        .is_ok());
    assert!(nominal_plan
        .decode_json_value(&Value::Null, &mut RequestHeap::default())
        .is_err());
    assert!(null_plan
        .decode_json_value(&Value::Null, &mut RequestHeap::default())
        .is_ok());
    assert!(null_plan
        .decode_json_value(&json!({}), &mut RequestHeap::default())
        .is_err());

    let bytes_type = ContractTypeRef::builtin("bytes");
    let bytes_plan = ServiceValuePlan::compile(&bytes_type, &BTreeMap::new()).unwrap();
    let mut bytes_heap = RequestHeap::default();
    let zero_bytes = bytes_plan
        .decode_json_value(&json!({"__skiffBytesBase64": ""}), &mut bytes_heap)
        .unwrap();
    assert!(bytes_plan.value_matches(&zero_bytes, &bytes_heap).unwrap());
    assert!(!null_plan.value_matches(&zero_bytes, &bytes_heap).unwrap());
    let encoded = bytes_plan
        .encode_binary(&zero_bytes, &websocket_boundary(), &bytes_heap)
        .unwrap();
    let decoded = bytes_plan
        .decode_binary(&encoded, &websocket_boundary(), &mut RequestHeap::default())
        .unwrap();
    assert!(matches!(decoded, RuntimeValue::Heap(_)));
}

#[test]
fn service_value_plan_enforces_safe_integer_duration_date_and_recursive_json() {
    let empty_schema = BTreeMap::new();
    let integer_type = ContractTypeRef::builtin("integer");
    let duration_type = ContractTypeRef::builtin("Duration");
    let date_type = ContractTypeRef::builtin("Date");
    let integer = ServiceValuePlan::compile(&integer_type, &empty_schema).unwrap();
    let duration = ServiceValuePlan::compile(&duration_type, &empty_schema).unwrap();
    let date = ServiceValuePlan::compile(&date_type, &empty_schema).unwrap();
    let heap = RequestHeap::default();

    for value in [-MAX_SAFE_INTEGER, 0.0, MAX_SAFE_INTEGER] {
        assert!(integer
            .value_matches(&RuntimeValue::Number(value), &heap)
            .unwrap());
        assert!(duration
            .value_matches(&RuntimeValue::Number(value), &heap)
            .unwrap());
    }
    for value in [MAX_SAFE_INTEGER + 1.0, 1.5, f64::INFINITY] {
        assert!(!integer
            .value_matches(&RuntimeValue::Number(value), &heap)
            .unwrap());
        assert!(!duration
            .value_matches(&RuntimeValue::Number(value), &heap)
            .unwrap());
    }
    for value in [MIN_EPOCH_MILLIS, 0, MAX_EPOCH_MILLIS] {
        assert!(date
            .value_matches(&RuntimeValue::Date(value), &heap)
            .unwrap());
    }
    for value in [MIN_EPOCH_MILLIS - 1, MAX_EPOCH_MILLIS + 1] {
        assert!(!date
            .value_matches(&RuntimeValue::Date(value), &heap)
            .unwrap());
    }

    let json_type = ContractTypeRef::builtin("Json");
    let json_plan = ServiceValuePlan::compile(&json_type, &empty_schema).unwrap();
    let nested = json!([
        null,
        {"level1": [{"level2": [1, 2, 3]}, false]},
        "done"
    ]);
    assert_json_binary_round_trip(&json_plan, &nested);

    let mut reserved = nested;
    reserved[1]["__skiffType"] = json!("legacy.Json");
    assert!(json_plan
        .decode_json_value(&reserved, &mut RequestHeap::default())
        .is_err());

    let ambiguous_type = ContractTypeRef::structural_union(vec![
        ContractTypeRef::builtin("number"),
        ContractTypeRef::builtin("integer"),
    ]);
    let ambiguous = ServiceValuePlan::compile(&ambiguous_type, &empty_schema).unwrap();
    assert!(matches!(
        ambiguous.value_matches(&RuntimeValue::Number(1.0), &heap),
        Err(ServiceLinkableMaterializationError::AmbiguousStructuralUnion)
    ));
}

#[test]
fn service_value_plan_accepts_acyclic_contract_recursion_and_rejects_runtime_cycles_and_interfaces()
{
    let child_id = PackageSchemaTypeId::new("contract-type:Child");
    let parent_id = PackageSchemaTypeId::new("contract-type:Parent");
    let schema = BTreeMap::from([
        schema_type(
            child_id.as_str(),
            "Child",
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([("name".to_string(), ContractTypeRef::builtin("string"))]),
            },
        ),
        schema_type(
            parent_id.as_str(),
            "Parent",
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    (
                        "children".to_string(),
                        generic(
                            "Array",
                            vec![ContractTypeRef::Nullable {
                                inner: Box::new(package_ref(child_id.clone())),
                            }],
                        ),
                    ),
                    ("left".to_string(), package_ref(child_id.clone())),
                    ("right".to_string(), package_ref(child_id)),
                ]),
            },
        ),
    ]);
    let parent_type = package_ref(parent_id);
    let parent_plan = ServiceValuePlan::compile(&parent_type, &schema).unwrap();
    let mut heap = RequestHeap::default();
    let shared_child = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::String("shared".to_string()),
        )])))
        .unwrap();
    let children = heap
        .alloc_array(vec![RuntimeValue::Heap(shared_child), RuntimeValue::Null])
        .unwrap();
    let parent = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("children".to_string(), RuntimeValue::Heap(children)),
            ("left".to_string(), RuntimeValue::Heap(shared_child)),
            ("right".to_string(), RuntimeValue::Heap(shared_child)),
        ])))
        .unwrap();
    assert!(parent_plan
        .value_matches(&RuntimeValue::Heap(parent), &heap)
        .unwrap());

    let json_type = ContractTypeRef::builtin("Json");
    let json_plan = ServiceValuePlan::compile(&json_type, &BTreeMap::new()).unwrap();
    let mut cyclic_heap = RequestHeap::default();
    let cycle = cyclic_heap.alloc_array(Vec::new()).unwrap();
    cyclic_heap
        .push_array_item_without_cycle_check_for_test(cycle, RuntimeValue::Heap(cycle))
        .unwrap();
    assert!(matches!(
        json_plan.value_matches(&RuntimeValue::Heap(cycle), &cyclic_heap),
        Err(ServiceLinkableMaterializationError::CyclicValueGraph)
    ));

    let mut interface_heap = RequestHeap::default();
    let callback = interface_heap
        .alloc_interface(InterfaceValue::new(
            "contract.Callback".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-1",
                "activation-1",
                1,
                "contract.Callback",
                "capability-1",
            )),
        ))
        .unwrap();
    assert!(matches!(
        json_plan.value_matches(&RuntimeValue::Heap(callback), &interface_heap),
        Err(
            ServiceLinkableMaterializationError::DetachedInterfaceCarrier {
                carrier: "callback capability"
            }
        )
    ));
}
