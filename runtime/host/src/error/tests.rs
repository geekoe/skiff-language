use std::fmt;

use serde_json::json;
use skiff_runtime_boundary::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    RuntimeRecoverableTrustBoundary,
};

use super::{
    add_diagnostic_frame, add_source_frame, Diagnosed, RuntimeError, RuntimeErrorPayload,
    TypeIdentity, WirePayload,
};
use crate::program::{FileAddr, TypeAddr, UnitAddr};

#[derive(Debug)]
struct DummyWirePayload;

impl fmt::Display for DummyWirePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "dummy wire payload")
    }
}

impl std::error::Error for DummyWirePayload {}

impl WirePayload for DummyWirePayload {
    fn payload(&self) -> RuntimeErrorPayload {
        dummy_wire_payload()
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        dummy_catch_projection()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn dummy_wire_payload() -> RuntimeErrorPayload {
    RuntimeErrorPayload {
        code: "test.OpaqueWireError".to_string(),
        message: "dummy wire payload".to_string(),
        status: Some(499),
        details: Some(json!({
            "delegated": true,
        })),
    }
}

fn dummy_catch_projection() -> Option<(TypeIdentity, serde_json::Value)> {
    Some((
        TypeIdentity::builtin("test.OpaqueCatchError"),
        json!({
            "caught": true,
        }),
    ))
}

fn assert_wire_case(
    name: &str,
    error: &dyn WirePayload,
    expected_code: &str,
    expected_catch: Option<&str>,
) {
    let payload = error.payload();
    assert_eq!(payload.code, expected_code, "{name} payload code");
    match (error.catch_projection(), expected_catch) {
        (Some((identity, _)), Some(expected)) => {
            assert_eq!(identity, TypeIdentity::builtin(expected), "{name} catch")
        }
        (None, None) => {}
        (actual, expected) => {
            panic!("{name} catch mismatch: expected {expected:?}, got {actual:?}")
        }
    }
}

fn json_error() -> serde_json::Error {
    serde_json::from_str::<serde_json::Value>("{").expect_err("json should fail")
}

#[test]
fn source_frame_wraps_non_object_details_without_losing_original() {
    let mut payload = RuntimeErrorPayload {
        code: "InternalError".to_string(),
        message: "failed".to_string(),
        status: None,
        details: Some(json!("raw details")),
    };

    add_source_frame(
        &mut payload,
        12,
        json!({ "sourceId": 12, "span": { "kind": "CallExpression" } }),
    );
    add_diagnostic_frame(
        &mut payload,
        json!({ "sourceId": 12, "operation": "Api.fail" }),
    );

    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["originalDetails"], "raw details");
    assert_eq!(details["sourceId"].as_u64(), Some(12));
    assert_eq!(details["sourceFrame"]["sourceId"].as_u64(), Some(12));
    assert_eq!(details["frames"][0]["operation"], "Api.fail");
    assert_eq!(details["frames"][1]["sourceId"].as_u64(), Some(12));
}

#[test]
fn source_frame_uses_outermost_frame_as_primary_location() {
    let mut payload = RuntimeErrorPayload {
        code: "InternalError".to_string(),
        message: "failed".to_string(),
        status: None,
        details: None,
    };

    add_source_frame(
        &mut payload,
        12,
        json!({ "sourceId": 12, "span": { "kind": "MemberExpression" } }),
    );
    add_source_frame(
        &mut payload,
        34,
        json!({ "sourceId": 34, "span": { "kind": "CallExpression" } }),
    );

    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["sourceId"].as_u64(), Some(34));
    assert_eq!(details["sourceFrame"]["sourceId"].as_u64(), Some(34));
    assert_eq!(details["sourceFrames"][0]["sourceId"].as_u64(), Some(34));
    assert_eq!(details["sourceFrames"][1]["sourceId"].as_u64(), Some(12));
}

#[test]
fn internal_decode_payload_uses_internal_error_code() {
    let payload = RuntimeError::Decode("expected runtime string".to_string()).payload();

    assert_eq!(payload.code, "InternalError");
    assert_eq!(payload.message, "expected runtime string");
    assert_eq!(payload.details, None);
}

#[test]
fn wire_payload_delegates_to_inherent_payload_with_default_catch_projection() {
    let error = RuntimeError::Decode("expected runtime string".to_string()).with_source(
        12,
        json!({ "sourceId": 12, "span": { "kind": "CallExpression" } }),
    );

    assert_eq!(WirePayload::payload(&error), error.payload());
    assert_eq!(WirePayload::catch_projection(&error), None);
    assert!(WirePayload::as_any(&error).is::<RuntimeError>());
}

#[test]
fn opaque_payload_delegates_to_boxed_wire_payload() {
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

    assert_eq!(error.payload(), dummy_wire_payload());
}

#[test]
fn opaque_catch_projection_delegates_to_boxed_wire_payload() {
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

    assert_eq!(
        WirePayload::catch_projection(&error),
        dummy_catch_projection()
    );
}

#[test]
fn service_db_fold_boxes_lease_lost_and_delegates_payload() {
    let service_error =
        skiff_runtime_service_db::ServiceDbError::LeaseLost("db lease was lost".to_string());
    let expected_payload = service_error.payload();

    let error = RuntimeError::Opaque(Box::new(service_error));

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
}

#[test]
fn service_db_fold_boxes_bson_decode_and_delegates_platform_payload() {
    let service_error = skiff_runtime_service_db::ServiceDbError::BsonDe(serde::de::Error::custom(
        "invalid bson document",
    ));
    let expected_payload = service_error.payload();

    let error = RuntimeError::Opaque(Box::new(service_error));

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(expected_payload.code, "PlatformBsonDecodeError");
    assert_eq!(error.payload(), expected_payload);
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("service-db fold should box the local error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_service_db::ServiceDbError>());
}

#[test]
fn model_fold_boxes_and_delegates_payload() {
    let model_error = skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded {
        resource: "request.heap".to_string(),
        reason: "too large".to_string(),
        limit: 10,
        current: 8,
        requested_delta: 4,
    };
    let expected_payload = model_error.payload();

    let error = RuntimeError::from(model_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(WirePayload::catch_projection(&error), None);
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("model fold should box the domain error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_model::error::RuntimeModelError>());
}

#[test]
fn boundary_fold_boxes_and_delegates_payload_and_catch_projection() {
    let boundary_error =
        skiff_runtime_boundary::error::RuntimeError::db_decode("std.db", "missing id");
    let expected_payload = boundary_error.payload();
    let expected_catch_projection = boundary_error.catch_projection();

    let error = RuntimeError::from(boundary_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("boundary fold should box the domain error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_boundary::error::RuntimeError>());
}

#[test]
fn linked_type_plan_fold_boxes_and_delegates_protocol_projection() {
    let linked_error = skiff_runtime_linked_type_plan::Error::Protocol {
        target: "svc.account".to_string(),
        message: "bad payload".to_string(),
    };
    let expected_payload = linked_error.payload();
    let expected_catch_projection = linked_error.catch_projection();

    let error = RuntimeError::from(linked_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("linked-type-plan fold should box the domain error");
    };
    assert!(boxed.as_any().is::<skiff_runtime_linked_type_plan::Error>());
}

#[test]
fn native_fold_boxes_and_delegates_timeout_projection() {
    let native_error = skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
        reason: skiff_runtime_native::error::BudgetReason::DeadlineExceeded,
        instruction_count: 42,
        limit: Some(100),
        elapsed_ms: 12.5,
    };
    let expected_payload = native_error.payload();
    let expected_catch_projection = native_error.catch_projection();

    let error = RuntimeError::from(native_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(error.payload().code, "TimeoutError");
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("native fold should box the domain error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_native::error::RuntimeError>());
}

#[test]
fn capability_context_file_fold_boxes_and_delegates_payload_and_catch_projection() {
    let capability_error =
        skiff_runtime_capability_context::FileCapabilityError::file("std.file not found: test");
    let expected_payload = capability_error.payload();
    let expected_catch_projection = capability_error.catch_projection();

    let error = RuntimeError::from(capability_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("capability-context fold should box the local error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_capability_context::FileCapabilityError>());
}

#[test]
fn capability_context_budget_fold_boxes_and_delegates_timeout_projection() {
    let capability_error = skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        },
    );
    let expected_payload = capability_error.payload();
    let expected_catch_projection = capability_error.catch_projection();

    let error = RuntimeError::from(capability_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(error.payload().code, "TimeoutError");
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
}

#[test]
fn capability_context_stream_producer_fold_boxes_and_preserves_eval_payload() {
    let stream_error = skiff_runtime_capability_context::StreamRuntimeError::producer(
        skiff_runtime_eval::error::RuntimeError::Cancelled,
    );
    let expected_payload = stream_error.payload();

    let error = RuntimeError::from(stream_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(expected_payload.code, "CancelError");
    assert_eq!(error.payload(), expected_payload);
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("stream producer fold should box the local stream error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_capability_context::StreamRuntimeError>());
}

#[test]
fn capability_context_stream_producer_fold_preserves_host_wire_catch_projection() {
    let stream_error = skiff_runtime_capability_context::StreamRuntimeError::producer(
        RuntimeError::Opaque(Box::new(DummyWirePayload)),
    );
    let expected_payload = stream_error.payload();
    let expected_catch_projection = stream_error.catch_projection();

    let error = RuntimeError::from(stream_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
    assert_eq!(
        WirePayload::catch_projection(&error),
        dummy_catch_projection()
    );
}

#[test]
fn capability_context_request_payload_fold_boxes_and_delegates_protocol_projection() {
    let capability_error =
        skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
            target: "svc.account".to_string(),
        };
    let expected_payload = capability_error.payload();
    let expected_catch_projection = capability_error.catch_projection();

    let error = RuntimeError::from(capability_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(error.payload().code, "std.service.ProtocolError");
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
}

#[test]
fn eval_leaf_fold_boxes_and_delegates_payload_and_catch_projection() {
    let eval_error = skiff_runtime_eval::error::RuntimeError::ProviderUnavailable {
        target: "svc.account".to_string(),
        reason: "no runtime".to_string(),
    };
    let expected_payload = eval_error.payload();
    let expected_catch_projection = WirePayload::catch_projection(&eval_error);

    let error = RuntimeError::from(eval_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(
        WirePayload::catch_projection(&error),
        expected_catch_projection
    );
}

#[test]
fn eval_root_runtime_payload_fold_becomes_external_error_payload() {
    let eval_error =
        skiff_runtime_eval::error::RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code: "DownstreamError".to_string(),
            message: "downstream failed".to_string(),
            status: Some(503),
            details: Some(json!({ "service": "account" })),
        });

    let error = RuntimeError::from(eval_error);

    assert!(matches!(
        error,
        RuntimeError::ExternalErrorPayload {
            ref code,
            ref message,
            status: Some(503),
            ref details,
        } if code == "DownstreamError"
            && message == "downstream failed"
            && details == &Some(json!({ "service": "account" }))
    ));
    assert_eq!(WirePayload::catch_projection(&error), None);
}

#[test]
fn eval_diagnostic_fold_keeps_host_wrappers_and_delegates_catch_projection() {
    let source_frame = json!({ "sourceId": 12, "span": { "kind": "CallExpression" } });
    let diagnostic_frame = json!({ "operation": "std.test.run" });
    let eval_error = skiff_runtime_eval::error::RuntimeError::FileError {
        message: "std.file not found".to_string(),
    }
    .with_source(12, source_frame.clone())
    .with_diagnostic_frame(diagnostic_frame.clone());

    let error = RuntimeError::from(eval_error);

    assert!(matches!(error, RuntimeError::Diagnosed(_)));
    assert_eq!(
        WirePayload::catch_projection(&error),
        Some((
            TypeIdentity::builtin("std.file.FileError"),
            json!({ "message": "std.file not found" }),
        ))
    );
    let payload = error.payload();
    assert_eq!(payload.code, "std.file.FileError");
    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["sourceId"].as_u64(), Some(12));
    assert_eq!(details["frames"][0], diagnostic_frame);
    assert_eq!(details["frames"][1], source_frame);
}

#[test]
fn request_cancel_detection_preserves_carried_capability_cancellation() {
    assert!(RuntimeError::cancelled().is_request_cancelled());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::ExecutionControlError::Cancelled,
    );
    let error = RuntimeError::from(eval_error);
    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert!(error.is_request_cancelled());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::StreamRuntimeError::producer(
            skiff_runtime_capability_context::ExecutionControlError::Cancelled,
        ),
    );
    let error = RuntimeError::from(eval_error);
    assert!(error.is_request_cancelled());

    let non_cancel_timeout = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(!non_cancel_timeout.is_request_cancelled());

    let cancel_budget = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(cancel_budget.is_request_cancelled());
    assert!(!RuntimeError::Decode("request was cancelled".to_string()).is_request_cancelled());
}

#[test]
fn request_cancel_detection_preserves_carried_request_and_native_cancellation() {
    let request_error =
        RuntimeError::Opaque(Box::new(skiff_runtime_request::RequestError::Cancelled));
    assert!(request_error.is_request_cancelled());

    let request_timeout = RuntimeError::Opaque(Box::new(
        skiff_runtime_request::RequestError::ExecutionBudgetExceeded {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    ));
    assert!(request_timeout.is_request_cancelled());

    let native_error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::Cancelled);
    assert!(matches!(native_error, RuntimeError::Opaque(_)));
    assert!(native_error.is_request_cancelled());

    let native_timeout = RuntimeError::from(
        skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
            reason: skiff_runtime_native::error::BudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(native_timeout.is_request_cancelled());

    let native_opaque_control =
        RuntimeError::from(skiff_runtime_native::error::RuntimeError::Opaque(Box::new(
            skiff_runtime_capability_context::ExecutionControlError::Cancelled,
        )));
    assert!(native_opaque_control.is_request_cancelled());
}

#[test]
fn request_cancel_detection_recurses_through_diagnosed_carriers() {
    let request_error =
        RuntimeError::Opaque(Box::new(skiff_runtime_request::RequestError::Cancelled))
            .with_diagnostic_frame(json!({ "operation": "request.cancel" }));
    assert!(matches!(request_error, RuntimeError::Diagnosed(_)));
    assert!(request_error.is_request_cancelled());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::ExecutionControlError::Cancelled,
    );
    assert!(matches!(
        eval_error,
        skiff_runtime_eval::error::RuntimeError::Opaque(_)
    ));
    let request_eval_error = RuntimeError::Opaque(Box::new(
        skiff_runtime_request::RequestError::Eval(eval_error),
    ))
    .with_diagnostic_frame(json!({ "operation": "request.eval" }));
    assert!(matches!(request_eval_error, RuntimeError::Diagnosed(_)));
    assert!(request_eval_error.is_request_cancelled());

    let native_error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::Cancelled)
        .with_diagnostic_frame(json!({ "operation": "native.cancel" }));
    assert!(matches!(native_error, RuntimeError::Diagnosed(_)));
    assert!(native_error.is_request_cancelled());
}

#[test]
fn diagnosed_payload_merges_frames_and_delegates_catch_projection() {
    let source_frame = json!({ "sourceId": 12, "span": { "kind": "CallExpression" } });
    let diagnostic_frame = json!({ "operation": "std.test.run" });
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload))
        .with_source(12, source_frame.clone())
        .with_diagnostic_frame(diagnostic_frame.clone());

    assert!(matches!(error, RuntimeError::Diagnosed(_)));
    assert!(WirePayload::as_any(match &error {
        RuntimeError::Diagnosed(diagnosed) => diagnosed,
        _ => unreachable!("diagnosed wrapper should be present"),
    })
    .is::<Diagnosed>());
    assert_eq!(
        WirePayload::catch_projection(&error),
        dummy_catch_projection()
    );

    let payload = error.payload();
    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["delegated"], true);
    assert_eq!(details["sourceId"].as_u64(), Some(12));
    assert_eq!(details["sourceFrame"], source_frame);
    assert_eq!(details["sourceFrames"][0], source_frame);
    assert_eq!(details["frames"][0], diagnostic_frame);
    assert_eq!(details["frames"][1], source_frame);
}

#[test]
fn source_frame_is_threaded_under_existing_diagnostic_frame() {
    let source_frame = json!({ "sourceId": 12, "span": { "kind": "CallExpression" } });
    let diagnostic_frame = json!({ "operation": "std.test.run" });
    let error = RuntimeError::Decode("failed".to_string())
        .with_diagnostic_frame(diagnostic_frame.clone())
        .with_source(12, source_frame.clone());

    let payload = error.payload();
    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["frames"][0], diagnostic_frame);
    assert_eq!(details["frames"][1], source_frame);
}

#[test]
fn non_opaque_runtime_errors_keep_default_catch_projection() {
    let errors = [
        RuntimeError::Decode("expected runtime string".to_string()),
        RuntimeError::ExternalErrorPayload {
            code: "ExternalCode".to_string(),
            message: "external payload".to_string(),
            status: None,
            details: None,
        },
    ];

    for error in errors {
        assert_eq!(WirePayload::catch_projection(&error), None);
    }
}

#[test]
fn std_json_decode_target_payload_uses_fully_qualified_code() {
    let payload =
        RuntimeError::decode_target("std.json.decode", "std.json.decode decode failed").payload();

    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(payload.message, "std.json.decode decode failed");
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "std.json.decode",
            "message": "std.json.decode decode failed",
        }))
    );
}

#[test]
fn std_json_encode_target_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::decode_target(
        "std.json.encode",
        "std.json.encode input: actor ref is not a JSON value",
    )
    .payload();

    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(
        payload.message,
        "std.json.encode input: actor ref is not a JSON value"
    );
}

#[test]
fn config_decode_target_payload_uses_config_code() {
    let payload =
        RuntimeError::decode_target("config.require", "path apiKey must be a string").payload();

    assert_eq!(payload.code, "config.DecodeError");
    assert_eq!(payload.message, "path apiKey must be a string");
}

#[test]
fn number_decode_target_payload_uses_std_number_code() {
    let payload = RuntimeError::decode_target(
        "number.assertSafeInteger",
        "number.assertSafeInteger requires a safe integer",
    )
    .payload();

    assert_eq!(payload.code, "std.number.DecodeError");
    assert_eq!(
        payload.message,
        "number.assertSafeInteger requires a safe integer"
    );
}

#[test]
fn time_decode_target_payload_uses_std_time_code() {
    let payload = RuntimeError::decode_target(
        "Date.requireParse",
        "Date.requireParse requires RFC3339 Date",
    )
    .payload();

    assert_eq!(payload.code, "std.time.DecodeError");
    assert_eq!(payload.message, "Date.requireParse requires RFC3339 Date");
}

#[test]
fn unknown_decode_target_payload_uses_internal_error_code() {
    let payload =
        RuntimeError::decode_target("runtime.config", "path apiKey must be a string").payload();

    assert_eq!(payload.code, "InternalError");
    assert_eq!(payload.message, "path apiKey must be a string");
}

#[test]
fn std_db_decode_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::Opaque(Box::new(
        skiff_runtime_service_db::ServiceDbError::db_decode(
            "std.db",
            "db value missing key field id",
        ),
    ))
    .payload();

    assert_eq!(payload.code, "std.db.DecodeError");
    assert_eq!(payload.message, "db value missing key field id");
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "std.db",
            "message": "db value missing key field id",
        }))
    );
}

#[test]
fn std_bytes_decode_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::Opaque(Box::new(
        skiff_runtime_boundary::error::RuntimeError::bytes_decode(
            "bytes.toUtf8String",
            "bytes.toUtf8String decode failed",
        ),
    ))
    .payload();

    assert_eq!(payload.code, "std.bytes.DecodeError");
    assert_eq!(payload.message, "bytes.toUtf8String decode failed");
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "bytes.toUtf8String",
            "message": "bytes.toUtf8String decode failed",
        }))
    );
}

#[test]
fn file_error_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::file_error("std.file not found: test").payload();

    assert_eq!(payload.code, "std.file.FileError");
    assert_eq!(payload.message, "std.file not found: test");
    assert_eq!(payload.details, None);
}

#[test]
fn std_http_error_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::http_error(
        "std.http.request missing url",
        Some(json!({ "field": "url" })),
    )
    .payload();

    assert_eq!(payload.code, "std.http.HttpError");
    assert_eq!(payload.message, "std.http.request missing url");
    assert_eq!(payload.details, Some(json!({ "field": "url" })));
}

#[test]
fn cancel_and_timeout_payload_codes_match_catchable_names() {
    let cancel = RuntimeError::cancelled().payload();
    assert_eq!(cancel.code, "CancelError");
    assert_eq!(cancel.message, "request was cancelled");

    let timeout = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        },
    )
    .payload();
    assert_eq!(timeout.code, "TimeoutError");
    assert_eq!(timeout.message, "execution deadline exceeded");
}

#[test]
fn service_error_payload_codes_are_fully_qualified() {
    let provider = RuntimeError::ProviderUnavailable {
        target: "svc.account".to_string(),
        reason: "no runtime".to_string(),
    }
    .payload();
    assert_eq!(provider.code, "std.service.ProviderUnavailableError");

    let protocol = RuntimeError::Protocol {
        target: "svc.account".to_string(),
        message: "bad frame".to_string(),
    }
    .payload();
    assert_eq!(protocol.code, "std.service.ProtocolError");
}

#[test]
fn phase6_host_small_root_golden_matrix() {
    let external_payload = RuntimeErrorPayload {
        code: "DownstreamError".to_string(),
        message: "downstream failed".to_string(),
        status: Some(503),
        details: Some(json!({ "service": "account" })),
    };
    assert_eq!(
        RuntimeError::ExternalErrorPayload {
            code: external_payload.code.clone(),
            message: external_payload.message.clone(),
            status: external_payload.status,
            details: external_payload.details.clone(),
        }
        .payload(),
        external_payload
    );

    let cases: Vec<(&str, RuntimeError, &str, Option<&str>)> = vec![
        (
            "host Decode",
            RuntimeError::Decode("decode failed".to_string()),
            "InternalError",
            None,
        ),
        (
            "host Unsupported",
            RuntimeError::Unsupported("unsupported feature".to_string()),
            "UnsupportedRuntimeFeature",
            None,
        ),
        (
            "host ProviderUnavailable",
            RuntimeError::ProviderUnavailable {
                target: "svc.account".to_string(),
                reason: "no runtime".to_string(),
            },
            "std.service.ProviderUnavailableError",
            Some("std.service.ProviderUnavailableError"),
        ),
        (
            "host Protocol",
            RuntimeError::Protocol {
                target: "svc.account".to_string(),
                message: "bad frame".to_string(),
            },
            "std.service.ProtocolError",
            Some("std.service.ProtocolError"),
        ),
        (
            "host ExternalErrorPayload",
            RuntimeError::ExternalErrorPayload {
                code: "DownstreamError".to_string(),
                message: "downstream failed".to_string(),
                status: Some(503),
                details: Some(json!({ "service": "account" })),
            },
            "DownstreamError",
            None,
        ),
        (
            "host Json",
            RuntimeError::Json(json_error()),
            "JsonError",
            None,
        ),
        (
            "host Opaque",
            RuntimeError::Opaque(Box::new(DummyWirePayload)),
            "test.OpaqueWireError",
            Some("test.OpaqueCatchError"),
        ),
        (
            "host Diagnosed Opaque",
            RuntimeError::Opaque(Box::new(DummyWirePayload))
                .with_diagnostic_frame(json!({ "operation": "phase6.matrix" })),
            "test.OpaqueWireError",
            Some("test.OpaqueCatchError"),
        ),
    ];

    for (name, error, expected_code, expected_catch) in cases {
        assert_wire_case(name, &error, expected_code, expected_catch);
    }
}

#[test]
fn phase6_cross_crate_error_code_and_catch_golden_matrix() {
    let capability_deadline =
        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
            skiff_runtime_capability_context::ExecutionBudgetFailure {
                reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
                instruction_count: 42,
                limit: Some(100),
                elapsed_ms: 12.5,
            },
        );
    let capability_cancelled_budget =
        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
            skiff_runtime_capability_context::ExecutionBudgetFailure {
                reason: skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled,
                instruction_count: 0,
                limit: None,
                elapsed_ms: 0.0,
            },
        );
    let request_timeout = skiff_runtime_request::RequestError::ExecutionBudgetExceeded {
        reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 42,
        limit: Some(100),
        elapsed_ms: 12.5,
    };
    let request_external = skiff_runtime_request::RequestError::ExternalErrorPayload {
        code: "DownstreamError".to_string(),
        message: "downstream failed".to_string(),
        status: Some(503),
        details: Some(json!({ "service": "account" })),
    };
    let eval_user_exception = skiff_runtime_eval::error::RuntimeError::UserException(
        skiff_runtime_eval::error::UserException::from_typed_payload(
            json!({ "message": "assertion failed" }),
            TypeIdentity::builtin("std.json.DecodeError"),
            Some(TypeIdentity::builtin("std.json.DecodeError")),
        )
        .expect("user exception should build"),
    );
    let eval_root_payload =
        skiff_runtime_eval::error::RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code: "DownstreamError".to_string(),
            message: "downstream failed".to_string(),
            status: Some(503),
            details: Some(json!({ "service": "account" })),
        });
    let linked_boundary = skiff_runtime_linked_type_plan::Error::Boundary(
        skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied"),
    );
    let request_eval = skiff_runtime_request::RequestError::Eval(
        skiff_runtime_eval::error::RuntimeError::file_error("std.file denied"),
    );
    let request_boundary = skiff_runtime_request::RequestError::Boundary(
        skiff_runtime_boundary::error::RuntimeError::http_error(
            "std.http failed",
            Some(json!({ "status": 500 })),
        ),
    );

    let cases: Vec<(&str, Box<dyn WirePayload>, &str, Option<&str>)> =
        vec![
        (
            "capability FileCapabilityError::File",
            Box::new(skiff_runtime_capability_context::FileCapabilityError::file(
                "std.file denied",
            )),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
        (
            "capability FileCapabilityError::ProviderUnavailable",
            Box::new(
                skiff_runtime_capability_context::FileCapabilityError::provider_unavailable(
                    "svc.account",
                    "no runtime",
                ),
            ),
            "std.service.ProviderUnavailableError",
            Some("std.service.ProviderUnavailableError"),
        ),
        (
            "capability FileCapabilityError::ResourceLimitExceeded",
            Box::new(
                skiff_runtime_capability_context::FileCapabilityError::resource_limit_exceeded(
                    "std.file",
                    "too large",
                    10,
                    8,
                    4,
                ),
            ),
            "ResourceLimitExceeded",
            None,
        ),
        (
            "capability FileCapabilityError::Decode",
            Box::new(skiff_runtime_capability_context::FileCapabilityError::decode(
                "invalid file payload",
            )),
            "InternalError",
            None,
        ),
        (
            "capability ExecutionControlError::Cancelled",
            Box::new(skiff_runtime_capability_context::ExecutionControlError::Cancelled),
            "CancelError",
            Some("CancelError"),
        ),
        (
            "capability ExecutionControlError::BudgetExceeded(deadline)",
            Box::new(capability_deadline),
            "TimeoutError",
            Some("TimeoutError"),
        ),
        (
            "capability ExecutionControlError::BudgetExceeded(cancelled)",
            Box::new(capability_cancelled_budget),
            "CancelError",
            Some("CancelError"),
        ),
        (
            "capability StreamRuntimeError::Cancelled",
            Box::new(skiff_runtime_capability_context::StreamRuntimeError::cancelled()),
            "CancelError",
            Some("CancelError"),
        ),
        (
            "capability StreamRuntimeError::Producer",
            Box::new(skiff_runtime_capability_context::StreamRuntimeError::producer(
                DummyWirePayload,
            )),
            "test.OpaqueWireError",
            Some("test.OpaqueCatchError"),
        ),
        (
            "capability RequestPayloadContextError::MissingBinaryHttp",
            Box::new(
                skiff_runtime_capability_context::RequestPayloadContextError::MissingBinaryHttp {
                    target: "svc.account".to_string(),
                },
            ),
            "std.service.ProtocolError",
            Some("std.service.ProtocolError"),
        ),
        (
            "capability OutboundRequestRegistryError",
            Box::new(
                skiff_runtime_capability_context::OutboundRequestRegistryError::DuplicateRequestId(
                    "req-1".to_string(),
                ),
            ),
            "InternalError",
            None,
        ),
        (
            "service-db LeaseLost",
            Box::new(skiff_runtime_service_db::ServiceDbError::LeaseLost(
                "lease lost".to_string(),
            )),
            "LeaseLost",
            None,
        ),
        (
            "service-db Mongo",
            Box::new(skiff_runtime_service_db::ServiceDbError::Mongo(
                mongodb::error::Error::custom("mongo failed"),
            )),
            "PlatformMongoError",
            None,
        ),
        (
            "service-db BsonSer",
            Box::new(skiff_runtime_service_db::ServiceDbError::BsonSer(
                serde::ser::Error::custom("bson encode failed"),
            )),
            "PlatformBsonEncodeError",
            None,
        ),
        (
            "service-db BsonDe",
            Box::new(skiff_runtime_service_db::ServiceDbError::BsonDe(
                serde::de::Error::custom("bson decode failed"),
            )),
            "PlatformBsonDecodeError",
            None,
        ),
        (
            "service-db DbDecode",
            Box::new(skiff_runtime_service_db::ServiceDbError::db_decode(
                "std.db",
                "missing id",
            )),
            "std.db.DecodeError",
            None,
        ),
        (
            "boundary DecodeTarget config",
            Box::new(skiff_runtime_boundary::error::RuntimeError::decode_target(
                "config.require",
                "missing config",
            )),
            "config.DecodeError",
            Some("config.DecodeError"),
        ),
        (
            "boundary DecodeTarget std.json",
            Box::new(skiff_runtime_boundary::error::RuntimeError::decode_target(
                "std.json.decode",
                "bad json",
            )),
            "std.json.DecodeError",
            Some("std.json.DecodeError"),
        ),
        (
            "boundary DecodeTarget std.number",
            Box::new(skiff_runtime_boundary::error::RuntimeError::decode_target(
                "number.parse",
                "bad number",
            )),
            "std.number.DecodeError",
            Some("std.number.DecodeError"),
        ),
        (
            "boundary DecodeTarget std.time",
            Box::new(skiff_runtime_boundary::error::RuntimeError::decode_target(
                "Date.requireParse",
                "bad time",
            )),
            "std.time.DecodeError",
            Some("std.time.DecodeError"),
        ),
        (
            "boundary DecodeTarget unknown",
            Box::new(skiff_runtime_boundary::error::RuntimeError::decode_target(
                "runtime.config",
                "bad config",
            )),
            "InternalError",
            None,
        ),
        (
            "boundary BytesDecode",
            Box::new(skiff_runtime_boundary::error::RuntimeError::bytes_decode(
                "bytes.toUtf8String",
                "bad bytes",
            )),
            "std.bytes.DecodeError",
            Some("std.bytes.DecodeError"),
        ),
        (
            "boundary DbDecode",
            Box::new(skiff_runtime_boundary::error::RuntimeError::db_decode(
                "std.db",
                "missing id",
            )),
            "std.db.DecodeError",
            Some("std.db.DecodeError"),
        ),
        (
            "boundary FileError",
            Box::new(skiff_runtime_boundary::error::RuntimeError::file_error(
                "std.file denied",
            )),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
        (
            "boundary HttpError",
            Box::new(skiff_runtime_boundary::error::RuntimeError::http_error(
                "std.http failed",
                Some(json!({ "status": 500 })),
            )),
            "std.http.HttpError",
            Some("std.http.HttpError"),
        ),
        (
            "native DecodeTarget",
            Box::new(skiff_runtime_native::error::RuntimeError::decode_target(
                "std.json.decode",
                "bad json",
            )),
            "std.json.DecodeError",
            Some("std.json.DecodeError"),
        ),
        (
            "native Opaque",
            Box::new(skiff_runtime_native::error::RuntimeError::Opaque(Box::new(
                DummyWirePayload,
            ))),
            "test.OpaqueWireError",
            Some("test.OpaqueCatchError"),
        ),
        (
            "eval UserException",
            Box::new(eval_user_exception),
            "UnhandledServiceError",
            None,
        ),
        (
            "eval RootRuntimePayload",
            Box::new(eval_root_payload),
            "DownstreamError",
            None,
        ),
        (
            "request Protocol",
            Box::new(skiff_runtime_request::RequestError::protocol(
                "svc.account",
                "bad frame",
            )),
            "std.service.ProtocolError",
            Some("std.service.ProtocolError"),
        ),
        (
            "request Cancelled",
            Box::new(skiff_runtime_request::RequestError::Cancelled),
            "CancelError",
            Some("CancelError"),
        ),
        (
            "request ExecutionBudgetExceeded",
            Box::new(request_timeout),
            "TimeoutError",
            Some("TimeoutError"),
        ),
        (
            "request ExternalErrorPayload",
            Box::new(request_external),
            "DownstreamError",
            None,
        ),
        (
            "request Eval delegation",
            Box::new(request_eval),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
        (
            "request Boundary delegation",
            Box::new(request_boundary),
            "std.http.HttpError",
            Some("std.http.HttpError"),
        ),
        (
            "linked-type-plan Protocol",
            Box::new(skiff_runtime_linked_type_plan::Error::Protocol {
                target: "svc.account".to_string(),
                message: "bad payload".to_string(),
            }),
            "std.service.ProtocolError",
            Some("std.service.ProtocolError"),
        ),
        (
            "linked-type-plan Boundary delegation",
            Box::new(linked_boundary),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
    ];

    for (name, error, expected_code, expected_catch) in cases {
        assert_wire_case(name, error.as_ref(), expected_code, expected_catch);
    }
}

#[test]
fn recoverable_payload_uses_boundary_details_contract() {
    let error = recoverable_boundary_error();
    let expected_details = error.details_json();

    let payload = RuntimeError::from(skiff_runtime_boundary::error::RuntimeError::Recoverable(
        error,
    ))
    .payload();

    assert_eq!(payload.code, "recoverableUnsupportedDecode");
    assert_eq!(payload.status, None);
    assert_eq!(payload.details, Some(expected_details));
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

fn service_type_identity(type_index: usize) -> TypeIdentity {
    TypeIdentity::address(TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    })
}

fn legacy_metadata_spoofed_eval_user_exception_payload(
    legacy_metadata_type_name: &str,
) -> RuntimeErrorPayload {
    let identity = service_type_identity(0);
    let eval_error = skiff_runtime_eval::error::RuntimeError::UserException(
        skiff_runtime_eval::error::UserException::from_envelope(json!({
            "__skiffException": true,
            "__skiffActualPayloadType": identity,
            "__skiffActualPayloadTypeDebug": legacy_metadata_type_name,
            "error": {
                "__skiffType": legacy_metadata_type_name,
                "message": "spoofed message",
                "detail": { "title": "ship" },
                "target": { "path": "body.title" }
            },
            "source": null,
            "stack": []
        }))
        .expect("spoofed user exception envelope should build"),
    );
    let error = RuntimeError::from(eval_error);

    error.payload()
}

#[test]
fn user_exception_payload_ignores_legacy_metadata_spoofed_http_error_type() {
    let payload = legacy_metadata_spoofed_eval_user_exception_payload("std.http.HttpError");

    assert_eq!(payload.status, None);
    assert_eq!(payload.code, "UnhandledServiceError");
    assert_eq!(
        payload.message,
        "unhandled user exception service:file[0]:type[0]: spoofed message"
    );
    assert_eq!(
        payload.details,
        Some(json!({ "actualPayloadType": "service:file[0]:type[0]" }))
    );
}

#[test]
fn user_exception_payload_ignores_legacy_metadata_spoofed_decode_error_type() {
    let payload = legacy_metadata_spoofed_eval_user_exception_payload("std.json.DecodeError");

    assert_eq!(payload.status, None);
    assert_eq!(payload.code, "UnhandledServiceError");
    assert_eq!(
        payload.message,
        "unhandled user exception service:file[0]:type[0]: spoofed message"
    );
    assert_eq!(
        payload.details,
        Some(json!({ "actualPayloadType": "service:file[0]:type[0]" }))
    );
}

#[test]
fn user_exception_payload_includes_erased_payload_message() {
    let eval_error = skiff_runtime_eval::error::RuntimeError::UserException(
        skiff_runtime_eval::error::UserException::from_typed_payload(
            json!({
                "target": "skiff.test.assert",
                "message": "assertion detail survived boundary"
            }),
            TypeIdentity::builtin("std.json.DecodeError"),
            Some(TypeIdentity::builtin("std.json.DecodeError")),
        )
        .expect("typed user exception should build"),
    );
    let error = RuntimeError::from(eval_error);

    let payload = error.payload();

    assert_eq!(payload.code, "UnhandledServiceError");
    assert_eq!(
        payload.message,
        "unhandled user exception std.json.DecodeError: assertion detail survived boundary"
    );
    assert_eq!(
        payload.details,
        Some(json!({ "actualPayloadType": "std.json.DecodeError" }))
    );
    assert_eq!(WirePayload::catch_projection(&error), None);
}

#[test]
fn user_exception_debug_name_remains_display_only_envelope_data() {
    let identity = TypeIdentity::address(TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    });
    let exception = skiff_runtime_eval::error::UserException::from_envelope(json!({
        "__skiffException": true,
        "__skiffActualPayloadType": identity,
        "__skiffActualPayloadTypeDebug": "std.http.HttpError",
        "error": { "message": "spoofed message" },
        "source": null,
        "stack": []
    }))
    .expect("spoofed user exception envelope should build");

    assert_eq!(
        exception.envelope()["__skiffActualPayloadTypeDebug"],
        "std.http.HttpError"
    );
    let payload = RuntimeError::from(skiff_runtime_eval::error::RuntimeError::UserException(
        exception,
    ))
    .payload();

    assert_eq!(
        payload.message,
        "unhandled user exception service:file[0]:type[0]: spoofed message"
    );
    assert_eq!(
        payload.details,
        Some(json!({ "actualPayloadType": "service:file[0]:type[0]" }))
    );
}

#[test]
fn diagnostic_source_preserves_source_frame_assembly_id() {
    let error = RuntimeError::Decode("failed".to_string()).with_source(
        7,
        json!({
            "assemblyId": 1,
            "sourceId": 7,
            "source": { "path": "package/main.skiff" }
        }),
    );

    let source = error
        .diagnostic_source()
        .expect("source frame should provide diagnostic source");

    assert_eq!(source.assembly_id, Some(1));
    assert_eq!(source.source_id, 7);
    assert_eq!(error.diagnostic_source_id(), Some(7));
}

#[test]
fn diagnostic_source_preserves_outer_diagnostic_frame_assembly_id() {
    let error = RuntimeError::Decode("failed".to_string())
        .with_source(
            7,
            json!({
                "assemblyId": 1,
                "sourceId": 7,
                "source": { "path": "package/main.skiff" }
            }),
        )
        .with_diagnostic_frame(json!({
            "assemblyId": 1,
            "sourceId": 7,
            "sourceFrame": {
                "assemblyId": 1,
                "sourceId": 7,
                "source": { "path": "package/main.skiff" }
            }
        }));

    let source = error
        .diagnostic_source()
        .expect("diagnostic frame should provide diagnostic source");

    assert_eq!(source.assembly_id, Some(1));
    assert_eq!(source.source_id, 7);
}
