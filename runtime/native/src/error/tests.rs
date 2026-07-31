use super::*;
use std::fmt;

#[derive(Debug)]
struct DummyWirePayload;

impl fmt::Display for DummyWirePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("dummy wire payload")
    }
}

impl std::error::Error for DummyWirePayload {}

impl WirePayload for DummyWirePayload {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: "test.NativeOpaque".to_string(),
            message: "dummy wire payload".to_string(),
            status: Some(499),
            details: Some(serde_json::json!({ "nativeOpaque": true })),
        }
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            serde_json::json!({ "caught": true }),
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn budget_reason_strings_distinguish_internal_cancel_from_timeouts() {
    assert_eq!(BudgetReason::Cancelled.as_str(), "cancelled");
    assert_eq!(BudgetReason::DeadlineExceeded.as_str(), "deadlineExceeded");
    assert_eq!(
        BudgetReason::InstructionLimitExceeded.as_str(),
        "instructionLimitExceeded"
    );
}

#[test]
fn native_ordinary_projection_excludes_cancellation_and_keeps_timeouts() {
    let decode_target = RuntimeError::decode_target("number.parse", "not a number")
        .ordinary_payload()
        .expect("decode error remains ordinary");
    assert_eq!(decode_target.code, "std.number.DecodeError");
    assert_eq!(
        decode_target.details,
        Some(serde_json::json!({
            "target": "number.parse",
            "message": "not a number",
        }))
    );

    assert!(RuntimeError::Cancelled.is_cancellation_terminal());
    assert_eq!(RuntimeError::Cancelled.ordinary_payload(), None);
    assert_eq!(RuntimeError::Cancelled.ordinary_catch_projection(), None);
    assert!(matches!(
        OrdinaryRuntimeError::try_new(RuntimeError::Cancelled),
        Err(RuntimeError::Cancelled)
    ));

    let timeout = RuntimeError::ExecutionBudgetExceeded {
        reason: BudgetReason::DeadlineExceeded,
        instruction_count: 42,
        limit: Some(100),
        elapsed_ms: 12.5,
    }
    .ordinary_payload()
    .expect("deadline remains ordinary");
    assert_eq!(timeout.code, "TimeoutError");
    assert_eq!(timeout.message, "execution deadline exceeded");
    assert_eq!(
        timeout.details,
        Some(serde_json::json!({
            "reason": "deadlineExceeded",
            "instructionCount": 42,
            "limit": 100,
            "elapsedMs": 12.5,
        }))
    );
}

#[test]
fn native_ordinary_catch_projection_covers_public_catchable_variants() {
    assert_eq!(
        RuntimeError::decode_target("Date.requireParse", "bad date").ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::TimeDecode.catch_identity(),
            serde_json::json!({
                "target": "Date.requireParse",
                "message": "bad date",
            })
        ))
    );
    assert_eq!(
        RuntimeError::bytes_decode("request.body", "invalid utf-8").ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::BytesDecode.catch_identity(),
            serde_json::json!({
                "target": "request.body",
                "message": "invalid utf-8",
            })
        ))
    );
    assert_eq!(
        RuntimeError::db_decode("users.createdAt", "invalid date").ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::DbDecode.catch_identity(),
            serde_json::json!({
                "target": "users.createdAt",
                "message": "invalid date",
            })
        ))
    );
    assert_eq!(
        RuntimeError::file_error("std.file denied").ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::File.catch_identity(),
            serde_json::json!({
                "message": "std.file denied",
            })
        ))
    );
    assert_eq!(
        RuntimeError::http_error(
            "upstream failed",
            Some(serde_json::json!({ "status": 503 })),
        )
        .ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            serde_json::json!({
                "message": "upstream failed",
                "detail": { "status": 503 },
            })
        ))
    );
    assert_eq!(
        RuntimeError::resource_error("prompts/system.md", "missing").ordinary_catch_projection(),
        None
    );
    assert_eq!(RuntimeError::Cancelled.ordinary_catch_projection(), None);
    assert_eq!(
        RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::InstructionLimitExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        }
        .ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
            serde_json::json!({
                "reason": "instructionLimitExceeded",
                "instructionCount": 42,
                "limit": 100,
                "elapsedMs": 12.5,
            })
        ))
    );
}

#[test]
fn native_ordinary_diagnostics_have_no_catch_projection() {
    assert_eq!(
        RuntimeError::decode_target("unknown.target", "bad value").ordinary_catch_projection(),
        None
    );
    assert_eq!(
        RuntimeError::InvalidArtifact("bad artifact".to_string()).ordinary_catch_projection(),
        None
    );
    assert_eq!(
        RuntimeError::Decode("bad value".to_string()).ordinary_catch_projection(),
        None
    );
    assert_eq!(
        RuntimeError::Unsupported("not implemented".to_string()).ordinary_catch_projection(),
        None
    );
    assert_eq!(
        RuntimeError::ResourceLimitExceeded {
            resource: "memory".to_string(),
            reason: "limit reached".to_string(),
            limit: 10,
            current: 10,
            requested_delta: 1,
        }
        .ordinary_catch_projection(),
        None
    );
}

#[test]
fn native_opaque_delegates_payload_and_catch_projection() {
    let error = RuntimeError::Opaque(Box::new(DummyWirePayload));

    assert_eq!(
        error
            .ordinary_payload()
            .expect("opaque ordinary payload")
            .code,
        "test.NativeOpaque"
    );
    assert_eq!(
        error
            .ordinary_payload()
            .expect("opaque ordinary payload")
            .status,
        Some(499)
    );
    assert_eq!(
        error.ordinary_catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            serde_json::json!({ "caught": true }),
        ))
    );
}
