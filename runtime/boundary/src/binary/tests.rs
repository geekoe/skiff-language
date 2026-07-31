use std::cell::Cell;

use serde_json::json;
use skiff_runtime_model::addr::ExecutableAddr;
use skiff_runtime_model::recoverable::{
    InterfaceValueState, LocalConcreteOwner, NativeAdapterOwner, NativeHandleState,
    NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope, RecoverableField,
    RecoverableNode, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
    RecoverableVariantIdentity, RuntimeRecoverableBoundaryKind, RuntimeRecoverableExpectedTypePlan,
    RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
};

use super::*;
use crate::payload::PayloadBoundaryKind;
use crate::recoverable::{
    FailClosedRecoverableBehaviorHooks, RecoverableBehaviorHooks,
    RecoverableEncodedLocalInterfaceSelf, RecoverableInterfaceConformanceRequest,
    RecoverableInterfaceMethodTableRequest, RecoverableLocalInterfaceEncodeRequest,
    RecoverableLocalInterfaceRestoreRequest, RecoverableRestoredLocalInterfaceSelf,
};
use crate::runtime_value::{
    CallbackCapabilityCarrier, InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable,
    InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue, RuntimeBytes,
};
use crate::type_descriptor::{RuntimeTypeNode, RuntimeTypePlanDescriptorExt};

fn test_boundary() -> PayloadBoundary {
    PayloadBoundary::runtime_internal()
}

fn any_interface_plan() -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "anyInterface".to_string(),
        named_type_name: None,
        identity: Default::default(),
        node: RuntimeTypeNode::Unknown,
    }
}

const READER_INTERFACE: &str = "pkg.Reader";
const READER_PROJECTION: &str = "projection:pkg.Reader:pkg.ReaderImpl";
const READER_METHOD: &str = "method:pkg.Reader:read";
const READER_IMPL: &str = "pkg.ReaderImpl";

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(&json!({
        "kind": "builtin",
        "name": "string",
        "args": []
    }))
    .expect("string plan should build")
}

fn any_reader_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan::any_interface(
        "any pkg.Reader",
        READER_INTERFACE,
        READER_PROJECTION,
    )
}

fn recoverable_unresolved_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan::unresolved("recoverable")
}

fn test_method_table(interface_identity: &str, projection_identity: &str) -> InterfaceMethodTable {
    InterfaceMethodTable::new(
        projection_identity.to_string(),
        interface_identity.to_string(),
        vec![InterfaceMethodSlot::new(
            0,
            READER_METHOD.to_string(),
            InterfaceMethodTarget::LocalExecutable {
                executable: ExecutableAddr::service(0, 7),
                receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
            },
        )],
    )
}

fn local_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            READER_INTERFACE.to_string(),
            InterfaceCarrier::Local {
                concrete_type: READER_IMPL.to_string(),
                method_table: test_method_table(READER_INTERFACE, READER_PROJECTION),
                payload: RuntimeValue::String("Ada".to_string()),
            },
        ))
        .expect("local interface should allocate"),
    )
}

fn recoverable_string_node(value: &str) -> RecoverableNode {
    RecoverableNode::plain(
        RecoverableValueKind::String,
        RecoverableState::String(value.to_string()),
    )
}

fn local_concrete_self_node(value: &str) -> RecoverableNode {
    RecoverableNode {
        value_kind: RecoverableValueKind::NominalObject,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::LocalConcrete {
            owner: LocalConcreteOwner::Service,
            concrete_type_identity: READER_IMPL.to_string(),
        },
        state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
            fields: vec![RecoverableField {
                field_identity: "value".to_string(),
                value: recoverable_string_node(value),
            }],
        }),
    }
}

fn interface_node() -> RecoverableNode {
    RecoverableNode::plain(
        RecoverableValueKind::InterfaceValue,
        RecoverableState::InterfaceValue(InterfaceValueState::Local {
            self_node: Box::new(local_concrete_self_node("Ada")),
        }),
    )
}

fn native_handle_node() -> RecoverableNode {
    RecoverableNode {
        value_kind: RecoverableValueKind::NativeHandle,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::NativeAdapter {
            adapter_identity: "std.FileHandleAdapter".to_string(),
            adapter_schema_version: "1".to_string(),
            owner: NativeAdapterOwner::Builtin,
            native_type_identity: "std.FileHandle".to_string(),
        },
        state: RecoverableState::NativeHandle(NativeHandleState {
            durable_state: Box::new(recoverable_string_node("durable")),
        }),
    }
}

fn native_adapter_plain_node() -> RecoverableNode {
    RecoverableNode {
        value_kind: RecoverableValueKind::String,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::NativeAdapter {
            adapter_identity: "std.StringAdapter".to_string(),
            adapter_schema_version: "1".to_string(),
            owner: NativeAdapterOwner::Builtin,
            native_type_identity: "std.StringLike".to_string(),
        },
        state: RecoverableState::String("native-adapter".to_string()),
    }
}

fn record_node(field_identity: &str, value: RecoverableNode) -> RecoverableNode {
    RecoverableNode::plain(
        RecoverableValueKind::Record,
        RecoverableState::Record(vec![RecoverableField {
            field_identity: field_identity.to_string(),
            value,
        }]),
    )
}

fn canonical_envelope_bytes(node: RecoverableNode) -> Vec<u8> {
    RecoverableEnvelope::new(node)
        .to_canonical_bytes(&RecoverableValidationLimits::default())
        .expect("recoverable envelope should canonical encode")
}

#[derive(Default)]
struct TestBehaviorHooks {
    encode_calls: Cell<usize>,
    restore_calls: Cell<usize>,
    conformance_calls: Cell<usize>,
    table_calls: Cell<usize>,
}

impl RecoverableBehaviorHooks for TestBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        _heap: &RequestHeap,
    ) -> Result<Option<RecoverableEncodedLocalInterfaceSelf>> {
        self.encode_calls.set(self.encode_calls.get() + 1);
        let value = match request.payload {
            RuntimeValue::String(value) => value.as_str(),
            RuntimeValue::Null => "null",
            _ => "unsupported",
        };
        Ok(Some(RecoverableEncodedLocalInterfaceSelf {
            method_projection_identity: request.method_table.id().to_string(),
            self_node: local_concrete_self_node(value),
        }))
    }

    fn restore_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceRestoreRequest<'_>,
        _heap: &mut RequestHeap,
    ) -> Result<Option<RecoverableRestoredLocalInterfaceSelf>> {
        self.restore_calls.set(self.restore_calls.get() + 1);
        let RecoverableCodeIdentity::LocalConcrete {
            concrete_type_identity,
            ..
        } = &request.self_node.code_identity
        else {
            return Ok(None);
        };
        let RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) =
            &request.self_node.state
        else {
            return Ok(None);
        };
        let value = fields
            .iter()
            .find(|field| field.field_identity == "value")
            .and_then(|field| match &field.value.state {
                RecoverableState::String(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Ok(Some(RecoverableRestoredLocalInterfaceSelf {
            concrete_type_identity: concrete_type_identity.clone(),
            payload: RuntimeValue::String(value),
        }))
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> Result<bool> {
        self.conformance_calls.set(self.conformance_calls.get() + 1);
        Ok(request.concrete_type_identity == READER_IMPL
            && request.interface_identity == READER_INTERFACE
            && request.method_projection_identity == READER_PROJECTION)
    }

    fn rebuild_local_interface_method_table(
        &self,
        _request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> Result<Option<InterfaceMethodTable>> {
        self.table_calls.set(self.table_calls.get() + 1);
        Ok(Some(test_method_table(READER_INTERFACE, READER_PROJECTION)))
    }
}

fn assert_interface_recoverable_envelope_error(
    error: RuntimeError,
    code: RecoverableBoundaryErrorCode,
) {
    let RuntimeError::Recoverable(error) = error else {
        panic!("expected recoverable error, got {error}");
    };
    assert_eq!(error.code(), code);
    assert_eq!(
        error.context().kind,
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload
    );
    assert_eq!(
        error.context().storage_lane,
        RuntimeRecoverableStorageLane::RecoverableEnvelope
    );
    assert!(error.context().explicit_recoverable_slot);
    let message = error.message();
    assert!(
        message.contains("recoverable envelope")
            && message.contains("real envelope encoding is not implemented"),
        "unexpected error: {message}"
    );
}

#[test]
fn payload_boundary_does_not_change_encoded_bytes() {
    let descriptor = json!({ "kind": "builtin", "name": "string", "args": [] });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
    let heap = RequestHeap::default();
    let value = RuntimeValue::String("Ada".to_string());
    let owner_internal = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
    let cross_service = PayloadBoundary::cross_service(
        PayloadBoundaryKind::OutboundServiceCall,
        crate::payload::PayloadServiceRef::new("skiff.run/account").with_version("0.1.0"),
    );

    let owner_bytes = encode_payload_plan(&value, &plan, &owner_internal, &heap)
        .expect("owner-internal payload should encode");
    let cross_service_bytes = encode_payload_plan(&value, &plan, &cross_service, &heap)
        .expect("cross-service payload should encode");

    assert_eq!(owner_bytes, cross_service_bytes);
}

#[test]
fn payload_codec_errors_include_boundary_context() {
    let descriptor = json!({ "kind": "builtin", "name": "string", "args": [] });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
    let heap = RequestHeap::default();
    let boundary = PayloadBoundary::cross_service(
        PayloadBoundaryKind::OutboundServiceCall,
        crate::payload::PayloadServiceRef::new("skiff.run/registry").with_version("0.1.0"),
    );

    let error = encode_payload_plan(&RuntimeValue::Number(7.0), &plan, &boundary, &heap)
        .expect_err("number must not encode as string");
    let message = error.to_string();

    assert!(message.contains("kind=OutboundServiceCall"));
    assert!(message.contains("target=skiff.run/registry@0.1.0"));
}

#[test]
fn spawn_and_queue_recoverable_payload_helpers_share_canonical_envelope() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] },
            "score": { "kind": "builtin", "name": "number", "args": [] }
        }
    });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
    let mut heap = RequestHeap::default();
    let object = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("name".to_string(), RuntimeValue::String("Ada".to_string())),
            ("score".to_string(), RuntimeValue::Number(98.5)),
        ])))
        .expect("record should allocate");
    let value = RuntimeValue::Heap(object);
    let service =
        PayloadServiceRef::new("skiff.run/account").with_build_id("skiff-service-build-a");
    let spawn_boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
        .with_origin_service(service.clone());
    let queue_boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::QueueWorkItemPayload)
        .with_origin_service(service);

    let spawn_bytes = encode_recoverable_payload_plan(&value, &plan, &spawn_boundary, &heap)
        .expect("spawn recoverable payload should encode");
    let queue_bytes = encode_recoverable_payload_plan(&value, &plan, &queue_boundary, &heap)
        .expect("queue recoverable payload should encode");

    assert_eq!(spawn_bytes, queue_bytes);

    let mut decode_heap = RequestHeap::default();
    let decoded =
        decode_recoverable_payload_plan(&spawn_bytes, &plan, &spawn_boundary, &mut decode_heap)
            .expect("spawn recoverable payload should decode");
    let RuntimeValue::Heap(decoded_handle) = decoded else {
        panic!("decoded value should be a heap object");
    };
    let HeapNode::Object(decoded_object) = decode_heap
        .get(decoded_handle)
        .expect("decoded object resolves")
    else {
        panic!("decoded value should be an object");
    };
    assert_eq!(
        decoded_object.fields().get("name"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
    assert_eq!(
        decoded_object.fields().get("score"),
        Some(&RuntimeValue::Number(98.5))
    );
}

#[test]
fn ordinary_payload_decode_rejects_recoverable_envelope_magic() {
    let plan = string_plan();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
    let heap = RequestHeap::default();
    let bytes = encode_recoverable_payload_plan(
        &RuntimeValue::String("Ada".to_string()),
        &plan,
        &boundary,
        &heap,
    )
    .expect("recoverable payload should encode");

    let error = decode_payload_plan(&bytes, &plan, &boundary, &mut RequestHeap::default())
        .expect_err("ordinary runtime binary payload must not accept SKRE");

    assert!(error.to_string().contains("missing SKPV magic"));
}

#[test]
fn public_cross_service_and_exported_materialization_plain_envelopes_roundtrip() {
    let plan = string_plan();
    let heap = RequestHeap::default();
    let value = RuntimeValue::String("plain".to_string());
    let boundaries = [
        PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
            .with_target_service(PayloadServiceRef::new("skiff.run/runtime-target")),
        PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            PayloadServiceRef::new("skiff.run/registry"),
        ),
        PayloadBoundary::external_untrusted(PayloadBoundaryKind::PublicApiPayload),
        PayloadBoundary::external_untrusted(PayloadBoundaryKind::MaterializationPayload),
    ];

    for boundary in boundaries {
        let bytes = encode_recoverable_payload_plan(&value, &plan, &boundary, &heap)
            .expect("plain recoverable envelope should encode");
        let decoded =
            decode_recoverable_payload_plan(&bytes, &plan, &boundary, &mut RequestHeap::default())
                .expect("plain recoverable envelope should decode");
        assert_eq!(decoded, value);
    }
}

#[test]
fn owner_internal_service_explicit_slot_roundtrips_local_interface_with_hooks() {
    let mut heap = RequestHeap::default();
    let value = local_interface_runtime_value(&mut heap);
    let expected = any_reader_expected();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::InboundServiceCall)
        .with_origin_service(PayloadServiceRef::new("skiff.run/account"));
    let hooks = TestBehaviorHooks::default();

    let bytes =
        encode_recoverable_payload_with_behavior(&value, &expected, &boundary, &heap, &hooks)
            .expect("local interface should encode through explicit owner-internal slot");
    assert_eq!(hooks.encode_calls.get(), 1);

    let mut decode_heap = RequestHeap::default();
    let decoded = decode_recoverable_payload_with_behavior(
        &bytes,
        &expected,
        &boundary,
        &mut decode_heap,
        &hooks,
    )
    .expect("local interface should decode through explicit owner-internal slot");

    let RuntimeValue::Heap(handle) = decoded else {
        panic!("decoded interface should be a heap value");
    };
    let HeapNode::Interface(interface) = decode_heap.get(handle).expect("interface resolves")
    else {
        panic!("decoded value should be an interface");
    };
    let InterfaceCarrier::Local {
        concrete_type,
        method_table,
        payload,
    } = interface.carrier()
    else {
        panic!("decoded interface should use local carrier");
    };
    assert_eq!(interface.interface(), READER_INTERFACE);
    assert_eq!(concrete_type, READER_IMPL);
    assert_eq!(method_table.id(), READER_PROJECTION);
    assert_eq!(method_table.interface_abi_id(), READER_INTERFACE);
    assert_eq!(method_table.slots()[0].method_abi_id(), READER_METHOD);
    assert_eq!(payload, &RuntimeValue::String("Ada".to_string()));
    assert_eq!(hooks.restore_calls.get(), 1);
    assert_eq!(hooks.conformance_calls.get(), 1);
    assert_eq!(hooks.table_calls.get(), 1);
}

#[test]
fn behavior_helper_encode_failures_return_no_bytes_before_submission() {
    let expected = any_reader_expected();
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
    let mut heap = RequestHeap::default();
    let local_value = local_interface_runtime_value(&mut heap);

    let missing_hook = FailClosedRecoverableBehaviorHooks;
    let error = encode_recoverable_payload_with_behavior(
        &local_value,
        &expected,
        &boundary,
        &heap,
        &missing_hook,
    )
    .expect_err("missing production hook must fail before bytes are returned");
    let RuntimeError::Recoverable(error) = error else {
        panic!("expected recoverable error");
    };
    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::CodeIdentityMissing
    );
}

#[test]
fn runtime_wire_target_service_rejects_behavior_and_maps_context() {
    let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
        .with_target_service(PayloadServiceRef::new("skiff.run/registry"));
    let context = recoverable_payload_context(&boundary);
    assert_eq!(
        context.kind,
        RuntimeRecoverableBoundaryKind::RuntimeWirePayload
    );
    assert_eq!(
        context.trust_boundary,
        RuntimeRecoverableTrustBoundary::CrossService
    );
    assert_eq!(
        context.storage_lane,
        RuntimeRecoverableStorageLane::RecoverableEnvelope
    );
    assert!(context.explicit_recoverable_slot);

    let hooks = TestBehaviorHooks::default();
    let bytes = canonical_envelope_bytes(record_node("value", interface_node()));
    let error = decode_recoverable_payload_with_behavior(
        &bytes,
        &recoverable_unresolved_expected(),
        &boundary,
        &mut RequestHeap::default(),
        &hooks,
    )
    .expect_err("runtime wire cross-service behavior must fail before hooks");
    let RuntimeError::Recoverable(error) = error else {
        panic!("expected recoverable error");
    };
    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
    );
    assert_eq!(hooks.restore_calls.get(), 0);
}

#[test]
fn non_owner_explicit_slots_reject_nested_behavior_before_hooks() {
    let boundaries = [
        PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            PayloadServiceRef::new("skiff.run/registry"),
        ),
        PayloadBoundary::external_untrusted(PayloadBoundaryKind::PublicApiPayload),
        PayloadBoundary::external_untrusted(PayloadBoundaryKind::MaterializationPayload),
        PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
            .with_target_service(PayloadServiceRef::new("skiff.run/registry")),
    ];
    let behavior_nodes = [
        interface_node(),
        local_concrete_self_node("Ada"),
        native_adapter_plain_node(),
        native_handle_node(),
    ];

    for boundary in boundaries {
        for node in behavior_nodes.clone() {
            let hooks = TestBehaviorHooks::default();
            let bytes = canonical_envelope_bytes(record_node("value", node));
            let error = decode_recoverable_payload_with_behavior(
                &bytes,
                &recoverable_unresolved_expected(),
                &boundary,
                &mut RequestHeap::default(),
                &hooks,
            )
            .expect_err("non-owner behavior envelope must fail closed before hooks");
            let RuntimeError::Recoverable(error) = error else {
                panic!("expected recoverable error");
            };
            assert_eq!(
                error.code(),
                RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
            );
            assert_eq!(hooks.restore_calls.get(), 0);
            assert_eq!(hooks.encode_calls.get(), 0);
        }
    }
}

#[test]
fn cross_service_local_carrier_encode_uses_callback_unavailable_error() {
    let boundary = PayloadBoundary::cross_service(
        PayloadBoundaryKind::OutboundServiceCall,
        PayloadServiceRef::new("skiff.run/registry"),
    );
    let expected = any_reader_expected();
    let mut heap = RequestHeap::default();
    let value = local_interface_runtime_value(&mut heap);
    let hooks = TestBehaviorHooks::default();

    let error =
        encode_recoverable_payload_with_behavior(&value, &expected, &boundary, &heap, &hooks)
            .expect_err("cross-service local carrier must fail before hooks");

    let RuntimeError::Recoverable(error) = error else {
        panic!("expected recoverable error");
    };
    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::CrossServiceInterfaceCallbackUnavailable
    );
    assert_eq!(hooks.encode_calls.get(), 0);
}

#[test]
fn payload_codec_round_trips_record_with_raw_bytes_without_base64_metadata() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] },
            "body": { "kind": "builtin", "name": "bytes", "args": [] }
        }
    });
    let mut heap = RequestHeap::default();
    let bytes = vec![0, 1, 2, 250, 255];
    let bytes_handle = heap
        .alloc_bytes(RuntimeBytes::from(bytes.clone()))
        .expect("bytes should allocate");
    let object_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("name".to_string(), RuntimeValue::String("Ada".to_string())),
            ("body".to_string(), RuntimeValue::Heap(bytes_handle)),
        ])))
        .expect("record should allocate");
    let encoded = encode_payload(&RuntimeValue::Heap(object_handle), &descriptor, &heap)
        .expect("payload should encode");

    assert!(!String::from_utf8_lossy(&encoded).contains("__skiffBytesBase64"));

    let mut decoded_heap = RequestHeap::default();
    let decoded =
        decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("payload should decode");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("decoded payload should be heap object");
    };
    let HeapNode::Object(object) = decoded_heap.get(handle).expect("object should exist") else {
        panic!("decoded payload should be object");
    };
    assert_eq!(
        object.fields().get("name"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
    let RuntimeValue::Heap(body_handle) = object.fields().get("body").unwrap() else {
        panic!("body should be heap bytes");
    };
    let HeapNode::Bytes(decoded_bytes) =
        decoded_heap.get(*body_handle).expect("bytes should exist")
    else {
        panic!("body should decode as bytes");
    };
    assert_eq!(decoded_bytes.as_slice(), bytes.as_slice());
}

#[test]
fn payload_codec_round_trips_date_as_epoch_milliseconds_tag() {
    let descriptor = json!({ "kind": "builtin", "name": "Date", "args": [] });
    let heap = RequestHeap::default();
    let encoded =
        encode_payload(&RuntimeValue::Date(0), &descriptor, &heap).expect("Date should encode");

    assert!(
        !String::from_utf8_lossy(&encoded).contains("1970-01-01"),
        "payload Date should not materialize as an ISO string"
    );

    let mut decoded_heap = RequestHeap::default();
    let decoded =
        decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("Date should decode");

    assert_eq!(decoded, RuntimeValue::Date(0));
}

#[test]
fn payload_codec_round_trips_duration_as_integer_milliseconds_payload() {
    let descriptor = json!({
        "kind": "representation",
        "name": "std.time.Duration",
        "representation": { "kind": "builtin", "name": "integer", "args": [] }
    });
    let heap = RequestHeap::default();
    let encoded = encode_payload(&RuntimeValue::Number(2_000.0), &descriptor, &heap)
        .expect("Duration should encode as integer payload");

    assert!(
        !String::from_utf8_lossy(&encoded).contains("Duration"),
        "payload Duration should not carry a nominal type envelope"
    );

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("Duration should decode as integer payload");

    assert_eq!(decoded, RuntimeValue::Number(2_000.0));
}

#[test]
fn payload_codec_nullable_union_branch_zero_does_not_decode_as_null() {
    let descriptor = json!({
        "kind": "nullable",
        "inner": {
            "kind": "union",
            "items": [
                { "kind": "builtin", "name": "string", "args": [] },
                { "kind": "builtin", "name": "number", "args": [] }
            ]
        }
    });
    let heap = RequestHeap::default();
    let encoded = encode_payload(
        &RuntimeValue::String("branch-zero".to_string()),
        &descriptor,
        &heap,
    )
    .expect("nullable union should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("nullable union should decode");

    assert_eq!(decoded, RuntimeValue::String("branch-zero".to_string()));
}

#[test]
fn payload_codec_encodes_map_literal_as_static_record_payload() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "tag": {
                "kind": "literal",
                "value": { "kind": "string", "value": "accept" }
            },
            "context": {
                "kind": "record",
                "fields": {
                    "userId": { "kind": "builtin", "name": "string", "args": [] }
                }
            }
        }
    });
    let mut heap = RequestHeap::default();
    let mut context = RuntimeMap::new();
    context.insert(
        RuntimeValueKey::string("userId"),
        RuntimeValue::String("user-1".to_string()),
    );
    let context_handle = heap
        .alloc_map(context)
        .expect("context map should allocate");
    let mut record = RuntimeMap::new();
    record.insert(
        RuntimeValueKey::string("tag"),
        RuntimeValue::String("accept".to_string()),
    );
    record.insert(
        RuntimeValueKey::string("context"),
        RuntimeValue::Heap(context_handle),
    );
    let record_handle = heap.alloc_map(record).expect("record map should allocate");

    let encoded = encode_payload(&RuntimeValue::Heap(record_handle), &descriptor, &heap)
        .expect("map literal should encode as static record payload");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("static record payload should decode");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("decoded record should be heap value");
    };
    let HeapNode::Object(object) = decoded_heap.get(handle).expect("record should exist") else {
        panic!("decoded record should be object");
    };
    assert_eq!(
        object.fields().get("tag"),
        Some(&RuntimeValue::String("accept".to_string()))
    );
}

#[test]
fn payload_codec_encodes_map_literal_against_union_record_payload() {
    let descriptor = json!({
        "kind": "union",
        "items": [
            {
                "kind": "record",
                "fields": {
                    "tag": {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "accept" }
                    },
                    "identity": { "kind": "builtin", "name": "string", "args": [] }
                }
            },
            {
                "kind": "record",
                "fields": {
                    "tag": {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "reject" }
                    },
                    "reason": { "kind": "builtin", "name": "string", "args": [] }
                }
            }
        ]
    });
    let mut heap = RequestHeap::default();
    let mut record = RuntimeMap::new();
    record.insert(
        RuntimeValueKey::string("identity"),
        RuntimeValue::String("user-1".to_string()),
    );
    record.insert(
        RuntimeValueKey::string("tag"),
        RuntimeValue::String("accept".to_string()),
    );
    let record_handle = heap.alloc_map(record).expect("record map should allocate");

    let encoded = encode_payload(&RuntimeValue::Heap(record_handle), &descriptor, &heap)
        .expect("map literal should encode against union record payload");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("union record payload should decode");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("decoded union branch should be heap value");
    };
    let HeapNode::Object(object) = decoded_heap
        .get(handle)
        .expect("decoded branch should exist")
    else {
        panic!("decoded union branch should be object");
    };
    assert_eq!(
        object.fields().get("identity"),
        Some(&RuntimeValue::String("user-1".to_string()))
    );
}

#[test]
fn payload_codec_round_trips_map_with_representation_keys() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "Map",
        "args": [
            {
                "kind": "representation",
                "name": "UserId",
                "representation": { "kind": "builtin", "name": "string", "args": [] }
            },
            { "kind": "builtin", "name": "number", "args": [] }
        ]
    });
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("user-1"),
        RuntimeValue::Number(42.0),
    );
    let map_handle = heap.alloc_map(map).expect("map should allocate");

    let encoded = encode_payload(&RuntimeValue::Heap(map_handle), &descriptor, &heap)
        .expect("map should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded =
        decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("map should decode");
    let RuntimeValue::Heap(decoded_handle) = decoded else {
        panic!("decoded map should be heap value");
    };
    let HeapNode::Map(decoded_map) = decoded_heap
        .get(decoded_handle)
        .expect("decoded map should exist")
    else {
        panic!("decoded payload should be map");
    };
    assert_eq!(
        decoded_map.get(&RuntimeValueKey::string("user-1")),
        Some(&RuntimeValue::Number(42.0))
    );
}

#[test]
fn payload_codec_round_trips_map_with_named_representation_keys() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "Map",
        "args": [
            { "kind": "builtin", "name": "UserId", "args": [] },
            { "kind": "builtin", "name": "number", "args": [] }
        ]
    });
    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("user-1"),
        RuntimeValue::Number(42.0),
    );
    let map_handle = heap.alloc_map(map).expect("map should allocate");

    let encoded = encode_payload(&RuntimeValue::Heap(map_handle), &descriptor, &heap)
        .expect("map should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded =
        decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("map should decode");
    let RuntimeValue::Heap(decoded_handle) = decoded else {
        panic!("decoded map should be heap value");
    };
    let HeapNode::Map(decoded_map) = decoded_heap
        .get(decoded_handle)
        .expect("decoded map should exist")
    else {
        panic!("decoded payload should be map");
    };
    assert_eq!(
        decoded_map.get(&RuntimeValueKey::string("user-1")),
        Some(&RuntimeValue::Number(42.0))
    );
}

#[test]
fn json_and_binary_boundaries_share_erased_plan_behavior() {
    let duration_descriptor = json!({
        "kind": "representation",
        "name": "std.time.Duration",
        "representation": { "kind": "builtin", "name": "integer", "args": [] }
    });
    let duration_plan =
        RuntimeTypePlan::from_descriptor(&duration_descriptor).expect("duration plan should build");
    let mut json_heap = RequestHeap::default();
    let duration =
        RuntimeBoundaryCodec::new(&duration_plan, BoundaryUse::TypedJson, "json duration")
            .from_wire_json(&json!(250), &mut json_heap)
            .expect("JSON boundary should erase Duration representation");
    assert_eq!(duration, RuntimeValue::Number(250.0));

    let encoded = encode_payload_plan(&duration, &duration_plan, &test_boundary(), &json_heap)
        .expect("binary boundary should encode erased Duration payload");
    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload_plan(
        &encoded,
        &duration_plan,
        &test_boundary(),
        &mut decoded_heap,
    )
    .expect("binary boundary should decode erased Duration payload");
    assert_eq!(decoded, RuntimeValue::Number(250.0));

    let date_plan = RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": "Date" }))
        .expect("Date plan should build");
    let mut date_json_heap = RequestHeap::default();
    let date = RuntimeBoundaryCodec::new(&date_plan, BoundaryUse::TypedJson, "json date")
        .from_wire_json(&json!("1970-01-01T00:00:00.000Z"), &mut date_json_heap)
        .expect("JSON boundary should decode RFC3339 Date");
    assert_eq!(date, RuntimeValue::Date(0));
    let date_encoded = encode_payload_plan(&date, &date_plan, &test_boundary(), &date_json_heap)
        .expect("binary boundary should encode Date as epoch millis");
    let mut date_decoded_heap = RequestHeap::default();
    assert_eq!(
        decode_payload_plan(
            &date_encoded,
            &date_plan,
            &test_boundary(),
            &mut date_decoded_heap
        )
        .expect("binary boundary should decode Date as epoch millis"),
        RuntimeValue::Date(0)
    );
}

#[test]
fn json_and_binary_boundaries_share_representation_map_key_behavior() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "Map",
        "args": [
            {
                "kind": "representation",
                "name": "UserId",
                "representation": { "kind": "builtin", "name": "string", "args": [] }
            },
            { "kind": "builtin", "name": "integer", "args": [] }
        ]
    });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("map plan should build");
    let mut json_heap = RequestHeap::default();
    let value = RuntimeBoundaryCodec::new(&plan, BoundaryUse::TypedJson, "json map")
        .from_wire_json(&json!({ "user-1": 7 }), &mut json_heap)
        .expect("JSON boundary should erase representation map keys");

    let encoded = encode_payload_plan(&value, &plan, &test_boundary(), &json_heap)
        .expect("binary boundary should encode representation map key as string");
    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload_plan(&encoded, &plan, &test_boundary(), &mut decoded_heap)
        .expect("binary boundary should decode representation map key as string");
    let RuntimeValue::Heap(decoded_handle) = decoded else {
        panic!("decoded map should be heap value");
    };
    let HeapNode::Map(decoded_map) = decoded_heap
        .get(decoded_handle)
        .expect("decoded map should exist")
    else {
        panic!("decoded payload should be map");
    };
    assert_eq!(
        decoded_map.get(&RuntimeValueKey::string("user-1")),
        Some(&RuntimeValue::Number(7.0))
    );
}

#[test]
fn json_and_binary_boundaries_reject_legacy_skiff_type_metadata() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "Map",
        "args": [
            { "kind": "builtin", "name": "string", "args": [] },
            { "kind": "builtin", "name": "string", "args": [] }
        ]
    });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("map plan should build");
    let mut json_heap = RequestHeap::default();
    let json_error = RuntimeBoundaryCodec::new(&plan, BoundaryUse::TypedJson, "json map")
        .from_wire_json(&json!({ "__skiffType": "Legacy" }), &mut json_heap)
        .expect_err("JSON boundary should reject reserved legacy metadata");
    assert!(json_error
        .to_string()
        .contains("reserved Skiff metadata field __skiffType"));

    let mut heap = RequestHeap::default();
    let mut map = RuntimeMap::new();
    map.insert(
        RuntimeValueKey::string("__skiffType"),
        RuntimeValue::String("Legacy".to_string()),
    );
    let handle = heap.alloc_map(map).expect("map should allocate");
    let binary_error =
        encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &test_boundary(), &heap)
            .expect_err("binary boundary should reject reserved legacy metadata");
    assert!(binary_error
        .to_string()
        .contains("reserved Skiff metadata field __skiffType"));
}

#[test]
fn runtime_payload_codec_rejects_interface_wrapper() {
    let descriptor = json!({ "kind": "builtin", "name": "Json", "args": [] });
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "pkg.FileReader".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:pkg.Reader:pkg.FileReader".to_string(),
                    "pkg.Reader".to_string(),
                    Vec::new(),
                ),
                payload: RuntimeValue::Null,
            },
        ))
        .expect("interface should allocate");

    let error = encode_payload(&RuntimeValue::Heap(handle), &descriptor, &heap)
        .expect_err("runtime binary payload should reject interface wrapper");

    assert_interface_recoverable_envelope_error(
        error,
        RecoverableBoundaryErrorCode::UnsupportedEncode,
    );
}

#[test]
fn runtime_payload_codec_fails_closed_typed_any_interface_local_carrier() {
    let plan = any_interface_plan();
    let mut heap = RequestHeap::default();
    let payload = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::String("Ada".to_string()),
        )])))
        .expect("payload object should allocate");
    let handle = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "pkg.FileReader".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:pkg.Reader:pkg.FileReader".to_string(),
                    "pkg.Reader".to_string(),
                    vec![InterfaceMethodSlot::new(
                        0,
                        "method:pkg.Reader.read".to_string(),
                        InterfaceMethodTarget::LocalExecutable {
                            executable: ExecutableAddr::service(2, 7),
                            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                        },
                    )],
                ),
                payload: RuntimeValue::Heap(payload),
            },
        ))
        .expect("interface should allocate");

    let error = encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &test_boundary(), &heap)
        .expect_err("typed any interface payload must fail closed until recover P4");

    assert_interface_recoverable_envelope_error(
        error,
        RecoverableBoundaryErrorCode::UnsupportedEncode,
    );
}

#[test]
fn runtime_binary_callback_capability_is_structurally_non_recoverable() {
    let plan = any_interface_plan();
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_interface(InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                "contract:reader",
                "capability-1",
            )),
        ))
        .expect("callback capability should allocate");

    let error = encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &test_boundary(), &heap)
        .expect_err("runtime binary must reject callback capability");
    let RuntimeError::Recoverable(error) = error else {
        panic!("expected structured recoverable rejection");
    };
    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable
    );
    assert_eq!(
        error
            .detail()
            .and_then(|detail| detail.get("rebuildAttempted")),
        Some(&serde_json::json!(false))
    );
}

#[test]
fn runtime_payload_codec_rejects_reserved_interface_tag_without_reconstructing_interface() {
    let plan = RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": "Json" }))
        .expect("Json plan should build");
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.push(VERSION);
    bytes.push(TAG_INTERFACE);

    let mut decoded_heap = RequestHeap::default();
    let error = decode_payload_plan(&bytes, &plan, &test_boundary(), &mut decoded_heap)
        .expect_err("reserved interface tag must not reconstruct InterfaceValue");

    assert_eq!(decoded_heap.len(), 0);
    assert_interface_recoverable_envelope_error(
        error,
        RecoverableBoundaryErrorCode::UnsupportedDecode,
    );
}

#[test]
fn payload_codec_encodes_record_payload_for_representation_descriptor() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] }
        }
    });
    let mut heap = RequestHeap::default();
    let object_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "name".to_string(),
            RuntimeValue::String("Ada".to_string()),
        )])))
        .expect("record should allocate");
    let encoded = encode_payload(&RuntimeValue::Heap(object_handle), &descriptor, &heap)
        .expect("erased representation payload record should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("erased representation payload record should decode");
    let RuntimeValue::Heap(decoded_handle) = decoded else {
        panic!("decoded record should be heap value");
    };
    let HeapNode::Object(decoded_object) = decoded_heap
        .get(decoded_handle)
        .expect("decoded record should exist")
    else {
        panic!("decoded payload should be object");
    };
    assert_eq!(
        decoded_object.fields().get("name"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );
}

#[test]
fn payload_codec_decodes_representation_descriptor_to_payload_value() {
    let descriptor = json!({
        "kind": "representation",
        "name": "Name",
        "representation": { "kind": "builtin", "name": "string", "args": [] }
    });
    let heap = RequestHeap::default();

    let encoded = encode_payload(&RuntimeValue::String("Ada".to_string()), &descriptor, &heap)
        .expect("erased representation payload should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("erased representation payload should decode");
    assert_eq!(decoded, RuntimeValue::String("Ada".to_string()));
}

#[test]
fn payload_codec_representation_descriptor_does_not_preserve_nominal_identity() {
    let descriptor = json!({
        "kind": "representation",
        "name": "UserId",
        "representation": { "kind": "builtin", "name": "string", "args": [] }
    });
    let heap = RequestHeap::default();

    let encoded = encode_payload(
        &RuntimeValue::String("tenant-1".to_string()),
        &descriptor,
        &heap,
    )
    .expect("erased representation payload should encode");

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
        .expect("erased representation payload should decode");
    assert_eq!(decoded, RuntimeValue::String("tenant-1".to_string()));
}

#[test]
fn payload_codec_rejects_union_with_more_than_256_branches() {
    let descriptor = json!({
        "kind": "union",
        "items": (0..257)
            .map(|_| json!({ "kind": "builtin", "name": "string", "args": [] }))
            .collect::<Vec<_>>()
    });
    let heap = RequestHeap::default();

    let error = encode_payload(
        &RuntimeValue::String("too-many-branches".to_string()),
        &descriptor,
        &heap,
    )
    .expect_err("union with more than 256 branches should fail closed");

    assert!(error.to_string().contains("maximum is 256"));
}
