use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InternalErrorPayload,
        LocalExecutionTypeIdentity, NominalTypeIdentity, OpaqueServiceError, RequestException,
        ServiceErrorEnvelope,
    },
    type_plan::{RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan},
};

use super::*;

fn local_identity(type_index: usize) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index,
            },
            type_arguments: Vec::new(),
        },
    ))
}

fn identified_string_plan(identity: CatchIdentity) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "identified string".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan {
            catch_identity: Some(identity),
            ..RuntimeTypeIdentityPlan::default()
        },
        node: RuntimeTypeNode::String,
    }
}

fn exception_with_payload(payload: RuntimeValueCarrier) -> RequestException {
    RequestException::local(
        payload,
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 1,
                start: SourcePosition::new(2, 3),
                end: SourcePosition::new(2, 8),
            },
        },
        vec![ExceptionStackFrame::Local {
            site: InstructionSourceSite::Source {
                span: SourceSpanRef {
                    source_id: 1,
                    start: SourcePosition::new(2, 3),
                    end: SourcePosition::new(2, 8),
                },
            },
        }],
        ErrorCorrelation {
            trace_id: "trace:exception-member".to_string(),
            error_id: "error:exception-member".to_string(),
        },
    )
    .expect("request-local exception")
}

fn exception_without_local_payload() -> RequestException {
    let envelope = ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "Internal service error".to_string(),
            trace_id: "trace:opaque-exception-member".to_string(),
            error_id: "error:opaque-exception-member".to_string(),
        },
    };
    let opaque = OpaqueServiceError::decode(
        serde_json::to_vec(&envelope).expect("fixed service error bytes"),
    )
    .expect("fixed service error");
    RequestException::imported(
        opaque,
        None,
        InstructionSourceSite::Source {
            span: SourceSpanRef {
                source_id: 2,
                start: SourcePosition::new(4, 1),
                end: SourcePosition::new(4, 6),
            },
        },
        vec![ExceptionStackFrame::RemoteBoundary {
            service_id: "example.service".to_string(),
            operation_id: "operation:call".to_string(),
            error_id: "error:opaque-exception-member".to_string(),
        }],
    )
    .expect("opaque request-local exception")
}

#[test]
fn exception_error_member_returns_the_exact_local_payload_carrier() {
    let identity = local_identity(7);
    let mut heap = RequestHeap::default();
    let payload_handle = heap
        .alloc_object_carriers(BTreeMap::from([(
            "reason".to_string(),
            RuntimeValueCarrier::unidentified(RuntimeValue::from("denied")),
        )]))
        .expect("nominal record payload");
    let payload =
        RuntimeValueCarrier::identified(RuntimeValue::Heap(payload_handle), identity.clone());
    let exception = heap
        .alloc_exception(exception_with_payload(payload.clone()))
        .expect("exception node");

    let actual = runtime_member_access_carrier(
        &RuntimeValueCarrier::unidentified(RuntimeValue::Heap(exception)),
        "error",
        &heap,
    )
    .expect("Exception.error");

    assert_eq!(actual, payload);
    assert_eq!(actual.catch_identity(), Some(&identity));
    assert_eq!(
        runtime_member_access_carrier(&actual, "reason", &heap)
            .expect("nominal payload field")
            .value(),
        &RuntimeValue::from("denied")
    );
    assert_eq!(
        crate::exceptions::request_exception_for_rethrow(
            &RuntimeValueCarrier::unidentified(RuntimeValue::Heap(exception)),
            &heap,
        )
        .expect("rethrow reads the unchanged exception"),
        exception_with_payload(payload)
    );
}

#[test]
fn exception_unknown_member_fails_closed() {
    let payload = RuntimeValueCarrier::identified(RuntimeValue::from("denied"), local_identity(8));
    let mut heap = RequestHeap::default();
    let exception = heap
        .alloc_exception(exception_with_payload(payload))
        .expect("exception node");

    let error = runtime_member_access_carrier(
        &RuntimeValueCarrier::unidentified(RuntimeValue::Heap(exception)),
        "stack",
        &heap,
    )
    .expect_err("unknown Exception member must fail closed");

    assert!(matches!(error, RuntimeError::Decode(message) if
            message.contains("unknown request-local Exception member")));
}

#[test]
fn exception_error_member_without_a_local_payload_fails_closed() {
    let mut heap = RequestHeap::default();
    let exception = heap
        .alloc_exception(exception_without_local_payload())
        .expect("exception node");

    let error = runtime_member_access_carrier(
        &RuntimeValueCarrier::unidentified(RuntimeValue::Heap(exception)),
        "error",
        &heap,
    )
    .expect_err("opaque Exception.error must not expose encoded or redacted payload");

    assert!(matches!(error, RuntimeError::Decode(message) if
            message.contains("has no caller-local payload")));
}

#[test]
fn plan_materialization_uses_existing_nested_identity_before_shape() {
    let first = local_identity(1);
    let second = local_identity(2);
    let union = RuntimeTypePlan::new(
        "same-shape union",
        None,
        RuntimeTypeNode::Union(vec![
            identified_string_plan(first),
            identified_string_plan(second.clone()),
        ]),
    );
    let record = RuntimeTypePlan::synthetic_request_record(vec![RuntimeRecordFieldPlan::new(
        "value", union, true,
    )]);
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_object_carriers(BTreeMap::from([(
            "value".to_string(),
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), second.clone()),
        )]))
        .expect("record");

    runtime_carrier_for_plan(
        RuntimeValue::Heap(handle),
        &record,
        "existing carrier",
        &mut heap,
    )
    .expect("existing identity selects the exact branch");

    assert_eq!(
        heap.object_field_carrier(handle, "value")
            .expect("record")
            .expect("field")
            .catch_identity(),
        Some(&second)
    );
}

#[test]
fn plan_materialization_rejects_a_conflicting_existing_identity() {
    let first = local_identity(1);
    let second = local_identity(2);
    let plan = identified_string_plan(second);

    let error = runtime_carrier_for_plan(
        RuntimeValueCarrier::identified(RuntimeValue::from("payload"), first),
        &plan,
        "conflicting carrier",
        &mut RequestHeap::default(),
    )
    .expect_err("identity mismatch must fail closed");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
}
