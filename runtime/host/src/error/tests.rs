use std::fmt;

use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_boundary::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity, NominalTypeIdentity,
        RequestException,
    },
};

use super::{
    add_diagnostic_frame, add_source_frame, CatchIdentity, OrdinaryRuntimeError,
    PlatformBuiltinErrorIdentity, RuntimeError, RuntimeErrorPayload, WirePayload,
};
use skiff_runtime_linked_program::{FileAddr, TypeAddr, UnitAddr};

#[derive(Debug)]
struct DummyWirePayload;

impl fmt::Display for DummyWirePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "dummy wire payload")
    }
}

impl std::error::Error for DummyWirePayload {}

fn mongo_command_error(code: i32, code_name: &str) -> mongodb::error::Error {
    let command_error: mongodb::error::CommandError = serde_json::from_value(json!({
        "code": code,
        "codeName": code_name,
        "errmsg": format!("Mongo command error {code_name}"),
    }))
    .expect("mongodb CommandError should deserialize");
    mongodb::error::ErrorKind::Command(command_error).into()
}

impl WirePayload for DummyWirePayload {
    fn payload(&self) -> RuntimeErrorPayload {
        dummy_wire_payload()
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
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

fn dummy_catch_projection() -> Option<(CatchIdentity, serde_json::Value)> {
    Some((
        PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
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
            assert_eq!(
                identity,
                PlatformBuiltinErrorIdentity::from_symbol(expected)
                    .unwrap_or_else(|| panic!("{name} expected a registered platform catch"))
                    .catch_identity(),
                "{name} catch"
            )
        }
        (None, None) => {}
        (actual, expected) => {
            panic!("{name} catch mismatch: expected {expected:?}, got {actual:?}")
        }
    }
}

fn ordinary_host_payload(error: &RuntimeError) -> RuntimeErrorPayload {
    error
        .ordinary_payload()
        .expect("test case must be an ordinary Host failure")
}

fn assert_host_case(
    name: &str,
    error: &RuntimeError,
    expected_code: &str,
    expected_catch: Option<&str>,
) {
    let payload = ordinary_host_payload(error);
    assert_eq!(payload.code, expected_code, "{name} payload code");
    match (error.ordinary_catch_projection(), expected_catch) {
        (Some((identity, _)), Some(expected)) => {
            assert_eq!(
                identity,
                PlatformBuiltinErrorIdentity::from_symbol(expected)
                    .unwrap_or_else(|| panic!("{name} expected a registered platform catch"))
                    .catch_identity(),
                "{name} catch"
            )
        }
        (None, None) => {}
        (actual, expected) => {
            panic!("{name} catch mismatch: expected {expected:?}, got {actual:?}")
        }
    }
}

fn boxed_host_ordinary(error: RuntimeError) -> Box<dyn WirePayload> {
    Box::new(OrdinaryRuntimeError::try_new(error).expect("matrix case must be ordinary"))
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
    let payload =
        ordinary_host_payload(&RuntimeError::Decode("expected runtime string".to_string()));

    assert_eq!(payload.code, "InternalError");
    assert_eq!(payload.message, "expected runtime string");
    assert_eq!(payload.details, None);
}

#[test]
fn ordinary_wrapper_delegates_payload_with_default_catch_projection() {
    let error = RuntimeError::Decode("expected runtime string".to_string()).with_source(
        12,
        json!({ "sourceId": 12, "span": { "kind": "CallExpression" } }),
    );
    let expected_payload = ordinary_host_payload(&error);
    let ordinary = OrdinaryRuntimeError::try_new(error).expect("decode failure is ordinary");

    assert_eq!(ordinary.payload(), expected_payload);
    assert_eq!(ordinary.catch_projection(), None);
}

#[test]
fn opaque_payload_delegates_to_boxed_wire_payload() {
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

    assert_eq!(ordinary_host_payload(&error), dummy_wire_payload());
}

#[test]
fn opaque_catch_projection_delegates_to_boxed_wire_payload() {
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

    assert_eq!(error.ordinary_catch_projection(), dummy_catch_projection());
}

#[test]
fn service_db_conflict_survives_host_opaque_boundary_with_sanitized_projection() {
    let error = RuntimeError::Opaque(Box::new(skiff_runtime_service_db::ServiceDbError::Mongo(
        mongo_command_error(112, "WriteConflict-secret-server-detail"),
    )));

    let payload = ordinary_host_payload(&error);
    assert_eq!(payload.code, "std.db.ConflictError");
    assert_eq!(
        payload.message,
        "database conflict; retry only at an explicit side-effect-safe boundary"
    );
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "std.db",
            "message": "database conflict; retry only at an explicit side-effect-safe boundary",
            "retryable": true,
        }))
    );
    assert!(!payload.message.contains("secret-server-detail"));
    assert_eq!(
        error.ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::DbConflict.catch_identity(),
            json!({
                "target": "std.db",
                "message": "database conflict; retry only at an explicit side-effect-safe boundary",
                "retryable": true,
            }),
        ))
    );
}

#[test]
fn service_db_fold_boxes_lease_lost_and_delegates_payload() {
    let service_error =
        skiff_runtime_service_db::ServiceDbError::LeaseLost("db lease was lost".to_string());
    let expected_payload = service_error.payload();

    let error = RuntimeError::Opaque(Box::new(service_error));

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
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
    assert_eq!(ordinary_host_payload(&error), expected_payload);
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
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), None);
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
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
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
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
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
    let expected_payload = native_error
        .ordinary_payload()
        .expect("deadline is ordinary");
    let expected_catch_projection = native_error.ordinary_catch_projection();

    let error = RuntimeError::from(native_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(ordinary_host_payload(&error).code, "TimeoutError");
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
    let RuntimeError::Opaque(boxed) = &error else {
        unreachable!("native fold should box the domain error");
    };
    assert!(boxed
        .as_any()
        .is::<skiff_runtime_native::error::OrdinaryRuntimeError>());
}

#[test]
fn capability_context_file_fold_boxes_and_delegates_payload_and_catch_projection() {
    let capability_error =
        skiff_runtime_capability_context::FileCapabilityError::file("std.file not found: test");
    let expected_payload = capability_error
        .ordinary_payload()
        .expect("file failure is ordinary");
    let expected_catch_projection = capability_error.ordinary_catch_projection();

    let error = RuntimeError::from(capability_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
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
    let expected_payload = capability_error
        .ordinary_payload()
        .expect("deadline is ordinary");
    let expected_catch_projection = capability_error.ordinary_catch_projection();

    let error = RuntimeError::from(capability_error);

    assert!(matches!(
        error,
        RuntimeError::ExecutionBudgetExceeded { .. }
    ));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(ordinary_host_payload(&error).code, "TimeoutError");
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
}

#[test]
fn capability_context_stream_cancel_fold_preserves_terminal_without_projection() {
    let error = RuntimeError::from(skiff_runtime_capability_context::StreamRuntimeError::Cancelled);

    assert!(matches!(error, RuntimeError::Cancelled));
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
}

#[test]
fn capability_context_stream_producer_fold_preserves_host_wire_catch_projection() {
    let stream_error =
        skiff_runtime_capability_context::StreamRuntimeError::producer(DummyWirePayload);
    let expected_payload = stream_error
        .ordinary_payload()
        .expect("dummy producer failure is ordinary");
    let expected_catch_projection = stream_error.ordinary_catch_projection();

    let error = RuntimeError::from(stream_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
    assert_eq!(error.ordinary_catch_projection(), dummy_catch_projection());
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
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(
        ordinary_host_payload(&error).code,
        "std.service.ProtocolError"
    );
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
}

#[test]
fn eval_leaf_fold_boxes_and_delegates_payload_and_catch_projection() {
    let eval_error = skiff_runtime_eval::error::RuntimeError::ProviderUnavailable {
        target: "svc.account".to_string(),
        reason: "no runtime".to_string(),
    };
    let expected_payload = eval_error
        .ordinary_payload()
        .expect("provider loss is ordinary");
    let expected_catch_projection = eval_error.ordinary_catch_projection();

    let error = RuntimeError::from(eval_error);

    assert!(matches!(error, RuntimeError::Opaque(_)));
    assert_eq!(ordinary_host_payload(&error), expected_payload);
    assert_eq!(error.ordinary_catch_projection(), expected_catch_projection);
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
    assert_eq!(error.ordinary_catch_projection(), None);
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
        error.ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::File.catch_identity(),
            json!({ "message": "std.file not found" }),
        ))
    );
    let payload = ordinary_host_payload(&error);
    assert_eq!(payload.code, "std.file.FileError");
    let details = payload.details.expect("diagnostic details should exist");
    assert_eq!(details["sourceId"].as_u64(), Some(12));
    assert_eq!(details["frames"][0], diagnostic_frame);
    assert_eq!(details["frames"][1], source_frame);
}

#[test]
fn request_cancel_detection_preserves_carried_capability_cancellation() {
    assert!(RuntimeError::cancelled().is_cancellation_terminal());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::ExecutionControlError::Cancelled,
    );
    let error = RuntimeError::from(eval_error);
    assert!(matches!(error, RuntimeError::Cancelled));
    assert!(error.is_cancellation_terminal());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::StreamRuntimeError::Cancelled,
    );
    let error = RuntimeError::from(eval_error);
    assert!(error.is_cancellation_terminal());

    let non_cancel_timeout = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(!non_cancel_timeout.is_cancellation_terminal());

    let cancel_budget = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(cancel_budget.is_cancellation_terminal());
    assert!(!RuntimeError::Decode("request was cancelled".to_string()).is_cancellation_terminal());
}

#[test]
fn request_cancel_detection_preserves_carried_request_and_native_cancellation() {
    let request_error = skiff_runtime_request::RequestError::Cancelled;
    assert!(request_error.is_cancellation_terminal());
    assert_eq!(request_error.ordinary_payload(), None);

    let native_error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::Cancelled);
    assert!(matches!(native_error, RuntimeError::Cancelled));
    assert!(native_error.is_cancellation_terminal());

    let native_timeout = RuntimeError::from(
        skiff_runtime_native::error::RuntimeError::ExecutionBudgetExceeded {
            reason: skiff_runtime_native::error::BudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        },
    );
    assert!(native_timeout.is_cancellation_terminal());

    let native_opaque_ordinary = RuntimeError::from(
        skiff_runtime_native::error::RuntimeError::Opaque(Box::new(DummyWirePayload)),
    );
    assert!(!native_opaque_ordinary.is_cancellation_terminal());
}

#[test]
fn request_cancel_detection_recurses_through_diagnosed_carriers() {
    let request_error =
        RuntimeError::Cancelled.with_diagnostic_frame(json!({ "operation": "request.cancel" }));
    assert!(matches!(request_error, RuntimeError::Diagnosed(_)));
    assert!(request_error.is_cancellation_terminal());

    let eval_error = skiff_runtime_eval::error::RuntimeError::from(
        skiff_runtime_capability_context::ExecutionControlError::Cancelled,
    );
    let request_eval_error = RuntimeError::from(eval_error)
        .with_diagnostic_frame(json!({ "operation": "request.eval" }));
    assert!(matches!(request_eval_error, RuntimeError::Diagnosed(_)));
    assert!(request_eval_error.is_cancellation_terminal());

    let native_error = RuntimeError::from(skiff_runtime_native::error::RuntimeError::Cancelled)
        .with_diagnostic_frame(json!({ "operation": "native.cancel" }));
    assert!(matches!(native_error, RuntimeError::Diagnosed(_)));
    assert!(native_error.is_cancellation_terminal());
}

#[test]
fn diagnosed_payload_merges_frames_and_delegates_catch_projection() {
    let source_frame = json!({ "sourceId": 12, "span": { "kind": "CallExpression" } });
    let diagnostic_frame = json!({ "operation": "std.test.run" });
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload))
        .with_source(12, source_frame.clone())
        .with_diagnostic_frame(diagnostic_frame.clone());

    assert!(matches!(error, RuntimeError::Diagnosed(_)));
    assert_eq!(error.ordinary_catch_projection(), dummy_catch_projection());

    let payload = ordinary_host_payload(&error);
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

    let payload = ordinary_host_payload(&error);
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
        assert_eq!(error.ordinary_catch_projection(), None);
    }
}

#[test]
fn std_json_decode_target_payload_uses_fully_qualified_code() {
    let payload = RuntimeError::decode_target("std.json.decode", "std.json.decode decode failed")
        .ordinary_payload()
        .expect("decode failure is ordinary");

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
    .ordinary_payload()
    .expect("decode failure is ordinary");

    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(
        payload.message,
        "std.json.encode input: actor ref is not a JSON value"
    );
}

#[test]
fn config_decode_target_payload_uses_config_code() {
    let payload = RuntimeError::decode_target("config.require", "path apiKey must be a string")
        .ordinary_payload()
        .expect("decode failure is ordinary");

    assert_eq!(payload.code, "config.DecodeError");
    assert_eq!(payload.message, "path apiKey must be a string");
}

#[test]
fn number_decode_target_payload_uses_std_number_code() {
    let payload = RuntimeError::decode_target(
        "number.assertSafeInteger",
        "number.assertSafeInteger requires a safe integer",
    )
    .ordinary_payload()
    .expect("decode failure is ordinary");

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
    .ordinary_payload()
    .expect("decode failure is ordinary");

    assert_eq!(payload.code, "std.time.DecodeError");
    assert_eq!(payload.message, "Date.requireParse requires RFC3339 Date");
}

#[test]
fn unknown_decode_target_payload_uses_internal_error_code() {
    let payload = RuntimeError::decode_target("runtime.config", "path apiKey must be a string")
        .ordinary_payload()
        .expect("decode failure is ordinary");

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
    .ordinary_payload()
    .expect("database decode failure is ordinary");

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
    .ordinary_payload()
    .expect("bytes decode failure is ordinary");

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
    let payload = RuntimeError::file_error("std.file not found: test")
        .ordinary_payload()
        .expect("file failure is ordinary");

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
    .ordinary_payload()
    .expect("HTTP failure is ordinary");

    assert_eq!(payload.code, "std.http.HttpError");
    assert_eq!(payload.message, "std.http.request missing url");
    assert_eq!(payload.details, Some(json!({ "field": "url" })));
}

#[test]
fn cancellation_has_no_projection_but_timeout_remains_ordinary() {
    let cancel = RuntimeError::cancelled();
    assert!(cancel.is_cancellation_terminal());
    assert_eq!(cancel.ordinary_payload(), None);
    assert_eq!(cancel.ordinary_catch_projection(), None);

    let timeout = RuntimeError::execution_budget_exceeded(
        skiff_runtime_capability_context::ExecutionBudgetFailure {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        },
    )
    .ordinary_payload()
    .expect("deadline is ordinary");
    assert_eq!(timeout.code, "TimeoutError");
    assert_eq!(timeout.message, "execution deadline exceeded");
}

#[test]
fn service_error_payload_codes_are_fully_qualified() {
    let provider = RuntimeError::ProviderUnavailable {
        target: "svc.account".to_string(),
        reason: "no runtime".to_string(),
    }
    .ordinary_payload()
    .expect("provider loss is ordinary");
    assert_eq!(provider.code, "std.service.ProviderUnavailableError");

    let protocol = RuntimeError::Protocol {
        target: "svc.account".to_string(),
        message: "bad frame".to_string(),
    }
    .ordinary_payload()
    .expect("protocol failure is ordinary");
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
        .ordinary_payload()
        .expect("external payload is ordinary"),
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
            Some("std.service.ProtocolError"),
        ),
        (
            "host Diagnosed Opaque",
            RuntimeError::Opaque(Box::new(DummyWirePayload))
                .with_diagnostic_frame(json!({ "operation": "phase6.matrix" })),
            "test.OpaqueWireError",
            Some("std.service.ProtocolError"),
        ),
    ];

    for (name, error, expected_code, expected_catch) in cases {
        assert_host_case(name, &error, expected_code, expected_catch);
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
        local_user_exception(0, "assertion failed", "phase6"),
    );
    let eval_root_payload =
        skiff_runtime_eval::error::RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
            code: "DownstreamError".to_string(),
            message: "downstream failed".to_string(),
            status: Some(503),
            details: Some(json!({ "service": "account" })),
        });
    let linked_boundary = skiff_runtime_linked_type_plan::Error::Boundary(Box::new(
        skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied"),
    ));
    let request_eval = skiff_runtime_request::RequestError::Eval(
        skiff_runtime_eval::error::RuntimeError::file_error("std.file denied"),
    );
    let request_boundary = skiff_runtime_request::RequestError::Boundary(
        skiff_runtime_boundary::error::RuntimeError::http_error(
            "std.http failed",
            Some(json!({ "status": 500 })),
        ),
    );

    for terminal in [
        RuntimeError::from(skiff_runtime_capability_context::ExecutionControlError::Cancelled),
        RuntimeError::from(
            skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                skiff_runtime_capability_context::ExecutionBudgetFailure {
                    reason: skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled,
                    instruction_count: 0,
                    limit: None,
                    elapsed_ms: 0.0,
                },
            ),
        ),
        RuntimeError::from(skiff_runtime_capability_context::StreamRuntimeError::Cancelled),
    ] {
        assert!(terminal.is_cancellation_terminal());
        assert_eq!(terminal.ordinary_payload(), None);
        assert_eq!(terminal.ordinary_catch_projection(), None);
    }
    let request_cancelled = skiff_runtime_request::RequestError::Cancelled;
    assert!(request_cancelled.is_cancellation_terminal());
    assert_eq!(request_cancelled.ordinary_payload(), None);
    assert_eq!(request_cancelled.ordinary_catch_projection(), None);

    let cases: Vec<(&str, Box<dyn WirePayload>, &str, Option<&str>)> = vec![
        (
            "capability FileCapabilityError::File",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_capability_context::FileCapabilityError::file("std.file denied"),
            )),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
        (
            "capability FileCapabilityError::ProviderUnavailable",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_capability_context::FileCapabilityError::provider_unavailable(
                    "svc.account",
                    "no runtime",
                ),
            )),
            "std.service.ProviderUnavailableError",
            Some("std.service.ProviderUnavailableError"),
        ),
        (
            "capability FileCapabilityError::ResourceLimitExceeded",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_capability_context::FileCapabilityError::resource_limit_exceeded(
                    "std.file",
                    "too large",
                    10,
                    8,
                    4,
                ),
            )),
            "ResourceLimitExceeded",
            None,
        ),
        (
            "capability FileCapabilityError::Decode",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_capability_context::FileCapabilityError::decode(
                    "invalid file payload",
                ),
            )),
            "InternalError",
            None,
        ),
        (
            "capability ExecutionControlError::BudgetExceeded(deadline)",
            boxed_host_ordinary(RuntimeError::from(capability_deadline)),
            "TimeoutError",
            Some("TimeoutError"),
        ),
        (
            "capability StreamRuntimeError::Producer",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_capability_context::StreamRuntimeError::producer(DummyWirePayload),
            )),
            "test.OpaqueWireError",
            Some("std.service.ProtocolError"),
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
            "service-db Mongo WriteConflict",
            Box::new(skiff_runtime_service_db::ServiceDbError::Mongo(
                mongo_command_error(112, "WriteConflict"),
            )),
            "std.db.ConflictError",
            Some("std.db.ConflictError"),
        ),
        (
            "service-db unique constraint",
            Box::new(skiff_runtime_service_db::ServiceDbError::Constraint(
                skiff_runtime_service_db::DbConstraintViolation::unique(
                    skiff_runtime_service_db::DbConstraintTarget::new(
                        "example.com/accounts",
                        "user",
                    )
                    .unwrap(),
                ),
            )),
            "std.db.ConstraintError",
            Some("std.db.ConstraintError"),
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
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_native::error::RuntimeError::decode_target(
                    "std.json.decode",
                    "bad json",
                ),
            )),
            "std.json.DecodeError",
            Some("std.json.DecodeError"),
        ),
        (
            "native Opaque",
            boxed_host_ordinary(RuntimeError::from(
                skiff_runtime_native::error::RuntimeError::Opaque(Box::new(DummyWirePayload)),
            )),
            "test.OpaqueWireError",
            Some("std.service.ProtocolError"),
        ),
        (
            "eval UserException",
            boxed_host_ordinary(RuntimeError::from(eval_user_exception)),
            "UnhandledServiceError",
            None,
        ),
        (
            "eval RootRuntimePayload",
            boxed_host_ordinary(RuntimeError::from(eval_root_payload)),
            "DownstreamError",
            None,
        ),
        (
            "request Protocol",
            Box::new(
                skiff_runtime_request::OrdinaryRequestError::try_new(
                    skiff_runtime_request::RequestError::protocol("svc.account", "bad frame"),
                )
                .expect("protocol failure is ordinary"),
            ),
            "std.service.ProtocolError",
            Some("std.service.ProtocolError"),
        ),
        (
            "request ExecutionBudgetExceeded",
            Box::new(
                skiff_runtime_request::OrdinaryRequestError::try_new(request_timeout)
                    .expect("deadline is ordinary"),
            ),
            "TimeoutError",
            Some("TimeoutError"),
        ),
        (
            "request ExternalErrorPayload",
            Box::new(
                skiff_runtime_request::OrdinaryRequestError::try_new(request_external)
                    .expect("external failure is ordinary"),
            ),
            "DownstreamError",
            None,
        ),
        (
            "request Eval delegation",
            Box::new(
                skiff_runtime_request::OrdinaryRequestError::try_new(request_eval)
                    .expect("file failure is ordinary"),
            ),
            "std.file.FileError",
            Some("std.file.FileError"),
        ),
        (
            "request Boundary delegation",
            Box::new(
                skiff_runtime_request::OrdinaryRequestError::try_new(request_boundary)
                    .expect("HTTP failure is ordinary"),
            ),
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
    .ordinary_payload()
    .expect("recoverable boundary failure is ordinary");

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

fn service_type_identity(type_index: usize) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::LoadedFileIndex(0),
                type_index,
            },
            type_arguments: Vec::new(),
        },
    ))
}

fn test_exception_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn local_user_exception(
    type_index: usize,
    payload: &str,
    correlation_label: &str,
) -> skiff_runtime_eval::error::UserException {
    let site = test_exception_site();
    skiff_runtime_eval::error::UserException::new(
        RequestException::local(
            RuntimeValueCarrier::identified(
                RuntimeValue::from(payload),
                service_type_identity(type_index),
            ),
            site.clone(),
            vec![ExceptionStackFrame::Local { site }],
            ErrorCorrelation {
                trace_id: format!("trace-{correlation_label}"),
                error_id: format!("trace-{correlation_label}:local-error:1"),
            },
        )
        .expect("test user exception should carry identity, source, stack and correlation"),
    )
}

#[test]
fn user_exception_payload_redacts_local_value_and_exposes_only_correlation() {
    let identity = service_type_identity(0);
    let exception = local_user_exception(0, "private assertion detail", "redaction");
    assert_eq!(exception.actual_payload_type(), Some(&identity));
    assert_eq!(
        exception.request().local_value().map(|value| value.value()),
        Some(&RuntimeValue::from("private assertion detail"))
    );
    assert_eq!(
        exception.request().correlation(),
        &ErrorCorrelation {
            trace_id: "trace-redaction".to_string(),
            error_id: "trace-redaction:local-error:1".to_string(),
        }
    );
    let eval_error = skiff_runtime_eval::error::RuntimeError::UserException(exception);
    let error = RuntimeError::from(eval_error);

    let payload = ordinary_host_payload(&error);

    assert_eq!(payload.code, "UnhandledServiceError");
    assert_eq!(payload.message, "unhandled request-local user exception");
    assert_eq!(
        payload.details,
        Some(json!({
            "traceId": "trace-redaction",
            "errorId": "trace-redaction:local-error:1",
        }))
    );
    assert!(!payload.message.contains("private assertion detail"));
    assert!(!payload.message.contains("service:file"));
    assert_eq!(error.ordinary_catch_projection(), None);
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
