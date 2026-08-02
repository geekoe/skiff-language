use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};
use skiff_runtime_boundary::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity, NominalTypeIdentity,
    },
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

fn source_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 41,
            start: SourcePosition::new(2, 3),
            end: SourcePosition::new(2, 8),
        },
    }
}

fn local_user_exception(identity: CatchIdentity, payload: &str) -> UserException {
    UserException::new(
        RequestException::local(
            RuntimeValueCarrier::identified(RuntimeValue::from(payload), identity),
            source_site(),
            vec![ExceptionStackFrame::Local {
                site: source_site(),
            }],
            ErrorCorrelation {
                trace_id: "trace-test".to_string(),
                error_id: "trace-test:local-error:1".to_string(),
            },
        )
        .expect("local test exception"),
    )
}

#[test]
fn replace_user_exception_preserves_nested_diagnostic_and_source_wrappers() {
    let original = local_user_exception(local_identity(1), "provider");
    let replacement_identity = local_identity(2);
    let replacement = local_user_exception(replacement_identity.clone(), "caller");
    let diagnostic_frame = serde_json::json!({ "operation": "service.call" });
    let source_frame = serde_json::json!({ "sourceId": 41, "module": "provider" });
    let error = RuntimeError::WithDiagnosticFrame {
        frame: Box::new(diagnostic_frame.clone()),
        error: Box::new(RuntimeError::WithSource {
            source_id: 41,
            frame: Box::new(source_frame.clone()),
            error: Box::new(RuntimeError::UserException(original)),
        }),
    };

    let replaced = replace_user_exception_preserving_diagnostics(error, replacement);

    let RuntimeError::WithDiagnosticFrame { frame, error } = replaced else {
        panic!("diagnostic wrapper should remain outermost")
    };
    assert_eq!(*frame, diagnostic_frame);
    let RuntimeError::WithSource {
        source_id,
        frame,
        error,
    } = *error
    else {
        panic!("source wrapper should remain nested inside diagnostic wrapper")
    };
    assert_eq!(source_id, 41);
    assert_eq!(*frame, source_frame);
    let RuntimeError::UserException(exception) = *error else {
        panic!("user exception should remain the wrapped leaf")
    };
    assert_eq!(exception.actual_payload_type(), Some(&replacement_identity));
    assert_eq!(
        exception.request().local_value().map(|value| value.value()),
        Some(&RuntimeValue::from("caller"))
    );
}

#[test]
fn boundary_helpers_preserve_wrapper_order_fields_and_terminal_classes() {
    let outer_source = serde_json::json!({
        "span": {
            "span": {
                "sourceId": 41,
                "start": { "line": 2, "column": 3 },
                "end": { "line": 2, "column": 8 }
            }
        }
    });
    let diagnostic = serde_json::json!({ "operation": "service.call" });
    let inner_source = serde_json::json!({ "sourceId": 42, "module": "provider" });
    let wrapped = RuntimeError::WithSource {
        source_id: 41,
        frame: Box::new(outer_source.clone()),
        error: Box::new(RuntimeError::WithDiagnosticFrame {
            frame: Box::new(diagnostic.clone()),
            error: Box::new(RuntimeError::WithSource {
                source_id: 42,
                frame: Box::new(inner_source.clone()),
                error: Box::new(RuntimeError::FileError {
                    message: "denied".to_string(),
                }),
            }),
        }),
    };

    let frames = diagnostic_source_frames(&wrapped);
    assert_eq!(frames, vec![&outer_source, &inner_source]);

    let preserved =
        extract_actor_instance_store_error(wrapped).expect_err("non-actor leaf remains eval");
    let RuntimeError::WithSource {
        source_id,
        frame,
        error,
    } = preserved
    else {
        panic!("outer source wrapper should remain first")
    };
    assert_eq!(source_id, 41);
    assert_eq!(*frame, outer_source);
    let RuntimeError::WithDiagnosticFrame { frame, error } = *error else {
        panic!("diagnostic wrapper should remain second")
    };
    assert_eq!(*frame, diagnostic);
    let RuntimeError::WithSource {
        source_id,
        frame,
        error,
    } = *error
    else {
        panic!("inner source wrapper should remain third")
    };
    assert_eq!(source_id, 42);
    assert_eq!(*frame, inner_source);
    assert!(matches!(*error, RuntimeError::FileError { ref message } if message == "denied"));

    let actor = RuntimeError::ActorInstance(
        crate::actor_instance::ActorInstanceStoreError::InstanceNotFound,
    )
    .with_diagnostic_frame(serde_json::json!({ "operation": "actor.call" }));
    assert!(matches!(
        extract_actor_instance_store_error(actor),
        Ok(crate::actor_instance::ActorInstanceStoreError::InstanceNotFound)
    ));

    let deadline = RuntimeError::ExecutionBudgetExceeded {
        reason: BudgetReason::DeadlineExceeded,
        instruction_count: 9,
        limit: Some(10),
        elapsed_ms: 1.0,
    }
    .with_diagnostic_frame(serde_json::json!({ "operation": "service.stream" }));
    assert!(is_deadline_budget_terminal(&deadline));
    assert!(is_deadline_or_scope_terminal(&deadline));
}

#[test]
fn replace_user_exception_leaves_other_error_classes_unchanged() {
    let replacement = local_user_exception(local_identity(2), "caller");

    let error = replace_user_exception_preserving_diagnostics(
        RuntimeError::Protocol {
            target: "operation:provider".to_string(),
            message: "provider protocol failure".to_string(),
        },
        replacement,
    );

    assert!(matches!(
        error,
        RuntimeError::Protocol { ref target, ref message }
            if target == "operation:provider" && message == "provider protocol failure"
    ));
}

#[test]
fn cross_heap_error_rematerialization_preserves_wrappers_and_local_payload() {
    let identity = local_identity(6);
    let mut source_heap = RequestHeap::default();
    source_heap.alloc_bytes(vec![0]).expect("source padding");
    let payload = source_heap
        .alloc_array(vec![RuntimeValue::from("provider-payload")])
        .expect("provider payload should allocate");
    let request = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::Heap(payload), identity.clone()),
        source_site(),
        vec![ExceptionStackFrame::Local {
            site: source_site(),
        }],
        ErrorCorrelation {
            trace_id: "cross-heap-wrapper-trace".to_string(),
            error_id: "cross-heap-wrapper-error".to_string(),
        },
    )
    .expect("wrapped user exception should be valid");
    let source_frame = serde_json::json!({ "sourceId": 41 });
    let diagnostic_frame = serde_json::json!({ "operation": "callback" });
    let error = RuntimeError::UserException(UserException::new(request))
        .with_source(41, source_frame.clone())
        .with_diagnostic_frame(diagnostic_frame.clone());
    let mut destination = RequestHeap::default();
    destination
        .alloc_bytes(vec![1])
        .expect("destination padding");
    let collision = destination
        .alloc_array(vec![RuntimeValue::from("caller-collision")])
        .expect("destination collision should allocate");
    assert_eq!(
        collision, payload,
        "test requires an exact handle collision"
    );

    let materialized =
        rematerialize_runtime_error_between_heaps(error, &source_heap, &mut destination)
            .expect("wrapped user exception should rematerialize");
    let RuntimeError::WithDiagnosticFrame { frame, error } = materialized else {
        panic!("diagnostic wrapper should remain outermost")
    };
    assert_eq!(*frame, diagnostic_frame);
    let RuntimeError::WithSource {
        source_id,
        frame,
        error,
    } = *error
    else {
        panic!("source wrapper should remain nested")
    };
    assert_eq!(source_id, 41);
    assert_eq!(*frame, source_frame);
    let RuntimeError::UserException(exception) = *error else {
        panic!("typed user exception should remain the wrapped leaf")
    };
    assert_eq!(exception.actual_payload_type(), Some(&identity));
    let RuntimeValue::Heap(materialized_payload) = exception
        .request()
        .local_value()
        .expect("materialized local payload")
        .value()
    else {
        panic!("materialized local payload should remain heap-backed")
    };
    assert_ne!(*materialized_payload, collision);
    assert!(matches!(
        destination.get(*materialized_payload),
        Ok(skiff_runtime_model::runtime_value::HeapNode::Array(items))
            if items == &[RuntimeValue::from("provider-payload")]
    ));
    assert!(matches!(
        destination.get(collision),
        Ok(skiff_runtime_model::runtime_value::HeapNode::Array(items))
            if items == &[RuntimeValue::from("caller-collision")]
    ));
}

#[test]
fn request_heap_owned_stream_error_materializes_the_exact_local_exception() {
    let identity = local_identity(7);
    let item_identity = local_identity(8);
    let mut source_heap = RequestHeap::default();
    source_heap.alloc_bytes(vec![0]).expect("dummy source node");
    let payload_handle = source_heap
        .alloc_array_carriers(vec![RuntimeValueCarrier::identified(
            RuntimeValue::from("nested"),
            item_identity.clone(),
        )])
        .expect("stream exception payload");
    let source = source_site();
    let correlation = ErrorCorrelation {
        trace_id: "trace-stream".to_string(),
        error_id: "trace-stream:local-error:1".to_string(),
    };
    let request = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::Heap(payload_handle), identity.clone()),
        source.clone(),
        vec![ExceptionStackFrame::Local {
            site: source.clone(),
        }],
        correlation.clone(),
    )
    .expect("stream exception");
    let stream_error = skiff_runtime_capability_context::StreamRuntimeError::producer(
        RequestHeapOwnedStreamError::try_new(
            RuntimeError::UserException(UserException::new(request)),
            source_heap,
        )
        .expect("user exception is an ordinary stream failure"),
    );
    let mut destination = RequestHeap::default();

    let materialized = materialize_stream_runtime_error(stream_error, &mut destination)
        .expect("stream exception materialization");
    let RuntimeError::UserException(exception) = unwrap_diagnostic_source_context(&materialized)
    else {
        panic!("stream error must remain a user exception");
    };
    assert_eq!(exception.actual_payload_type(), Some(&identity));
    assert_eq!(exception.request().source(), &source);
    assert_eq!(exception.request().correlation(), &correlation);
    let RuntimeValue::Heap(cloned_handle) = exception
        .request()
        .local_value()
        .expect("local cause")
        .value()
    else {
        panic!("stream cause must remain an array");
    };
    assert_ne!(*cloned_handle, payload_handle);
    assert_eq!(
        destination
            .array_item_carrier(*cloned_handle, 0)
            .expect("cloned array")
            .expect("cloned nested value")
            .catch_identity(),
        Some(&item_identity)
    );
}

#[test]
fn request_heap_root_rebind_preserves_nested_local_exception_payload_and_diagnostics() {
    let identity = local_identity(9);
    let nested_identity = local_identity(10);
    let source = source_site();
    let correlation = ErrorCorrelation {
        trace_id: "trace-rollback".to_string(),
        error_id: "trace-rollback:local-error:1".to_string(),
    };
    let diagnostic_frame = serde_json::json!({ "operation": "db.transaction" });
    let source_frame = serde_json::json!({ "sourceId": 41, "module": "transaction-body" });
    let mut heap = RequestHeap::default();
    heap.alloc_bytes(vec![0]).expect("pre-checkpoint heap node");
    let checkpoint = heap.checkpoint();
    heap.alloc_bytes(vec![1])
        .expect("dead transaction-local node");
    let nested_handle = heap
        .alloc_array_carriers(vec![RuntimeValueCarrier::identified(
            RuntimeValue::from("nested"),
            nested_identity.clone(),
        )])
        .expect("nested exception payload");
    let payload_handle = heap
        .alloc_array_carriers(vec![RuntimeValueCarrier::from(RuntimeValue::Heap(
            nested_handle,
        ))])
        .expect("outer exception payload");
    let request = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::Heap(payload_handle), identity.clone()),
        source.clone(),
        vec![ExceptionStackFrame::Local {
            site: source.clone(),
        }],
        correlation.clone(),
    )
    .expect("local transaction exception");
    let error = RuntimeError::UserException(UserException::new(request))
        .with_source(41, source_frame.clone())
        .with_diagnostic_frame(diagnostic_frame.clone());

    let prepared = heap
        .prepare_rollback_rebase(checkpoint, &[RuntimeValue::Heap(payload_handle)])
        .expect("rollback graph must be valid");
    let preserved_root = prepared.rebased_roots()[0].clone();
    let preserved = rebind_runtime_error_request_heap_root(error, Some(preserved_root))
        .expect("error root mapping must match");
    heap.commit_prepared_rollback_rebase(prepared);

    assert_eq!(
        heap.len(),
        3,
        "only the checkpoint and reachable payload remain"
    );
    let RuntimeError::WithDiagnosticFrame { frame, error } = preserved else {
        panic!("diagnostic wrapper should remain outermost")
    };
    assert_eq!(*frame, diagnostic_frame);
    let RuntimeError::WithSource {
        source_id,
        frame,
        error,
    } = *error
    else {
        panic!("source wrapper should remain nested")
    };
    assert_eq!(source_id, 41);
    assert_eq!(*frame, source_frame);
    let RuntimeError::UserException(exception) = *error else {
        panic!("user exception should remain the leaf")
    };
    assert_eq!(exception.actual_payload_type(), Some(&identity));
    assert_eq!(exception.request().source(), &source);
    assert_eq!(exception.request().correlation(), &correlation);
    let RuntimeValue::Heap(preserved_payload) = exception
        .request()
        .local_value()
        .expect("local exception cause")
        .value()
    else {
        panic!("exception cause must remain a heap payload")
    };
    assert_ne!(*preserved_payload, payload_handle);
    let RuntimeValue::Heap(preserved_nested) = heap
        .array_item_carrier(*preserved_payload, 0)
        .expect("outer payload remains readable")
        .expect("outer payload item")
        .into_value()
    else {
        panic!("outer payload must retain its nested heap value")
    };
    let nested = heap
        .array_item_carrier(preserved_nested, 0)
        .expect("nested payload remains readable")
        .expect("nested payload item");
    assert_eq!(nested.value(), &RuntimeValue::from("nested"));
    assert_eq!(nested.catch_identity(), Some(&nested_identity));
}

fn recoverable_boundary_error() -> RecoverableBoundaryError {
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");

    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::UnsupportedDecode,
        "recoverable decode is unsupported",
        &context,
        &expected,
    )
}

#[test]
fn recoverable_payload_uses_boundary_details_contract() {
    let error = recoverable_boundary_error();
    let expected_details = error.details_json();

    let payload = RuntimeError::Recoverable(error)
        .ordinary_payload()
        .expect("recoverable error remains ordinary");

    assert_eq!(payload.code, "recoverableUnsupportedDecode");
    assert_eq!(payload.status, None);
    assert_eq!(payload.details, Some(expected_details));
}

#[test]
fn ordinary_projection_preserves_diagnostic_payload_and_excludes_cancellation() {
    let error = RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
        code: "DownstreamError".to_string(),
        message: "downstream failed".to_string(),
        status: Some(503),
        details: Some(serde_json::json!({ "service": "account" })),
    })
    .with_diagnostic_frame(serde_json::json!({ "sourceId": 7 }));

    assert_eq!(
        error.ordinary_payload().expect("ordinary payload").code,
        "DownstreamError"
    );
    assert_eq!(error.ordinary_catch_projection(), None);
    assert!(RuntimeError::Cancelled.is_cancellation_terminal());
    assert_eq!(RuntimeError::Cancelled.ordinary_payload(), None);
    assert_eq!(RuntimeError::Cancelled.ordinary_catch_projection(), None);
    assert!(matches!(
        OrdinaryRuntimeError::try_new(RuntimeError::Cancelled),
        Err(RuntimeError::Cancelled)
    ));
    let stream_terminal =
        RequestHeapOwnedStreamError::try_new(RuntimeError::Cancelled, RequestHeap::default())
            .expect_err("request-heap stream wrapper must reject cancellation");
    assert!(stream_terminal.is_cancellation_terminal());
}

fn assert_catch_projection(
    error: RuntimeError,
    expected_identity: &'static str,
    expected_payload: Value,
) {
    let identity = PlatformBuiltinErrorIdentity::from_symbol(expected_identity)
        .expect("test must use the finite platform-error registry")
        .catch_identity();
    assert_eq!(
        error.ordinary_catch_projection(),
        Some((identity, expected_payload))
    );
}

#[test]
fn catch_projection_covers_standard_eval_errors() {
    assert_catch_projection(
        RuntimeError::DecodeTarget {
            target: "std.json.decode".to_string(),
            message: "invalid json".to_string(),
        },
        "std.json.DecodeError",
        serde_json::json!({
            "target": "std.json.decode",
            "message": "invalid json",
        }),
    );
    assert_catch_projection(
        RuntimeError::BytesDecode {
            target: "bytes.toUtf8String".to_string(),
            message: "invalid utf8".to_string(),
        },
        "std.bytes.DecodeError",
        serde_json::json!({
            "target": "bytes.toUtf8String",
            "message": "invalid utf8",
        }),
    );
    assert_catch_projection(
        RuntimeError::DbDecode {
            target: "std.db".to_string(),
            message: "missing id".to_string(),
        },
        "std.db.DecodeError",
        serde_json::json!({
            "target": "std.db",
            "message": "missing id",
        }),
    );
    assert_catch_projection(
        RuntimeError::FileError {
            message: "std.file not found".to_string(),
        },
        "std.file.FileError",
        serde_json::json!({
            "message": "std.file not found",
        }),
    );
    assert_eq!(
        RuntimeError::resource_error("prompts/system.md", "missing",).ordinary_catch_projection(),
        None,
        "ResourceError is package-owned and must not project a platform catch identity",
    );
    assert_catch_projection(
        RuntimeError::HttpError {
            message: "std.http.request failed".to_string(),
            detail: Some(serde_json::json!({ "status": 500 })),
        },
        "std.http.HttpError",
        serde_json::json!({
            "message": "std.http.request failed",
            "detail": { "status": 500 },
        }),
    );
    assert_eq!(RuntimeError::Cancelled.ordinary_catch_projection(), None);
    assert_catch_projection(
        RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::InstructionLimitExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        },
        "TimeoutError",
        serde_json::json!({
            "reason": "instructionLimitExceeded",
            "instructionCount": 42,
            "limit": 100,
            "elapsedMs": 12.5,
        }),
    );
    assert_catch_projection(
        RuntimeError::ProviderUnavailable {
            target: "svc.account".to_string(),
            reason: "no runtime".to_string(),
        },
        "std.service.ProviderUnavailableError",
        serde_json::json!({
            "target": "svc.account",
            "reason": "no runtime",
        }),
    );
    assert_catch_projection(
        RuntimeError::Protocol {
            target: "svc.account".to_string(),
            message: "bad frame".to_string(),
        },
        "std.service.ProtocolError",
        serde_json::json!({
            "target": "svc.account",
            "message": "bad frame",
        }),
    );
}

#[test]
fn unknown_decode_target_is_not_catchable() {
    let error = RuntimeError::DecodeTarget {
        target: "runtime.config".to_string(),
        message: "path apiKey must be a string".to_string(),
    };

    assert_eq!(error.ordinary_catch_projection(), None);
}

#[test]
fn root_runtime_payload_is_wire_only() {
    let stored = RuntimeErrorPayload {
        code: "DownstreamError".to_string(),
        message: "downstream failed".to_string(),
        status: Some(503),
        details: Some(serde_json::json!({ "service": "account" })),
    };
    let error = RuntimeError::RootRuntimePayload(stored.clone());

    assert_eq!(error.ordinary_payload(), Some(stored));
    assert_eq!(error.ordinary_catch_projection(), None);
}

#[test]
fn domain_forward_preserves_concrete_error_variants_payload_and_catch_projection() {
    let model_error = skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
        resource: "request.heap".to_string(),
        reason: "too large".to_string(),
        limit: 10,
        current: 8,
        requested_delta: 4,
    };
    let expected_model_payload = model_error.payload();
    let error = RuntimeError::from(model_error);
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded {
            ref resource,
            ..
        } if resource == "request.heap"
    ));
    assert_eq!(error.ordinary_payload(), Some(expected_model_payload));
    assert_eq!(error.ordinary_catch_projection(), None);

    let boundary_error = skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied");
    let expected_boundary_payload = boundary_error.payload();
    let expected_boundary_catch_projection = boundary_error.catch_projection();
    let error = RuntimeError::from(boundary_error);
    assert!(matches!(error, RuntimeError::FileError { .. }));
    assert_eq!(error.ordinary_payload(), Some(expected_boundary_payload));
    assert_eq!(
        error.ordinary_catch_projection(),
        expected_boundary_catch_projection
    );

    let linked_error = skiff_runtime_linked_type_plan::Error::Protocol {
        target: "svc.account".to_string(),
        message: "bad payload".to_string(),
    };
    let expected_linked_payload = linked_error.payload();
    let expected_linked_catch_projection = linked_error.catch_projection();
    let error = RuntimeError::from(linked_error);
    assert!(matches!(error, RuntimeError::Protocol { .. }));
    assert_eq!(error.ordinary_payload(), Some(expected_linked_payload));
    assert_eq!(
        error.ordinary_catch_projection(),
        expected_linked_catch_projection
    );
}

#[test]
fn eval_to_native_back_projection_preserves_concrete_non_control_errors() {
    let error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::DbDecode {
        target: "std.db".to_string(),
        message: "missing id".to_string(),
    });

    assert!(matches!(error, RuntimeError::DbDecode { .. }));
    let native = eval_error_to_native(error);
    assert!(matches!(
        native,
        skiff_runtime_native::error::RuntimeError::DbDecode { ref target, ref message }
            if target == "std.db" && message == "missing id"
    ));
}

#[test]
fn eval_to_native_back_projection_preserves_diagnostic_wrappers_as_opaque() {
    let source_frame = serde_json::json!({ "sourceId": 7 });
    let diagnostic_frame = serde_json::json!({ "operation": "eval.test" });
    let error = RuntimeError::FileError {
        message: "std.file denied".to_string(),
    }
    .with_source(7, source_frame.clone())
    .with_diagnostic_frame(diagnostic_frame.clone());

    let native = eval_error_to_native(error);

    match native {
        skiff_runtime_native::error::RuntimeError::Opaque(error) => {
            let payload = error.payload();
            assert_eq!(payload.code, "std.file.FileError");
            let details = payload.details.expect("diagnostic details should exist");
            assert_eq!(details["sourceId"].as_u64(), Some(7));
            assert_eq!(details["frames"][0], diagnostic_frame);
            assert_eq!(details["frames"][1], source_frame);
            assert_eq!(
                error.catch_projection(),
                Some((
                    PlatformBuiltinErrorIdentity::File.catch_identity(),
                    serde_json::json!({ "message": "std.file denied" }),
                ))
            );
        }
        error => panic!("expected native Opaque, got {error:?}"),
    }
}

#[test]
fn capability_context_errors_preserve_concrete_variants_payload_and_catch_projection() {
    let file_error =
        skiff_runtime_capability_context::FileCapabilityError::file("std.file not found");
    let expected_payload = file_error
        .ordinary_payload()
        .expect("file error remains ordinary");
    let expected_catch_projection = file_error.ordinary_catch_projection();
    let error = RuntimeError::from(file_error);
    assert!(matches!(error, RuntimeError::FileError { .. }));
    assert_eq!(error.ordinary_payload(), Some(expected_payload));
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);

    let protocol_error =
        skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
            target: "svc.account".to_string(),
        };
    let expected_payload = protocol_error.payload();
    let expected_catch_projection = protocol_error.catch_projection();
    let error = RuntimeError::from(protocol_error);
    assert!(matches!(
        error,
        RuntimeError::Protocol { ref target, .. } if target == "svc.account"
    ));
    assert_eq!(error.ordinary_payload(), Some(expected_payload));
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);

    let timeout_error = skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        },
    );
    let expected_payload = timeout_error
        .ordinary_payload()
        .expect("deadline remains ordinary");
    let expected_catch_projection = timeout_error.ordinary_catch_projection();
    let error = RuntimeError::from(timeout_error);
    assert!(matches!(
        error,
        RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::DeadlineExceeded,
            ..
        }
    ));
    assert_eq!(error.ordinary_payload(), Some(expected_payload));
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
}
mod scope_terminal;
