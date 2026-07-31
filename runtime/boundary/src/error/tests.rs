use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    RuntimeRecoverableTrustBoundary,
};

use super::*;

fn recoverable_error(code: RecoverableBoundaryErrorCode) -> RecoverableBoundaryError {
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");

    RecoverableBoundaryError::new(
        code,
        "recoverable boundary is unsupported",
        &context,
        &expected,
    )
}

#[test]
fn recoverable_error_codes_are_stable() {
    assert_eq!(
        RecoverableBoundaryErrorCode::UnsupportedEncode.as_str(),
        "recoverableUnsupportedEncode"
    );
    assert_eq!(
        RecoverableBoundaryErrorCode::UnsupportedDecode.as_str(),
        "recoverableUnsupportedDecode"
    );
    assert_eq!(
        RecoverableBoundaryErrorCode::ArtifactUnavailable.as_str(),
        "recoverable_artifact_unavailable"
    );
    assert_eq!(
        RecoverableBoundaryErrorCode::StateInvalid.as_str(),
        "recoverable_state_invalid"
    );
    assert_eq!(
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable.as_str(),
        "callback_capability_not_recoverable"
    );
}

#[test]
fn recoverable_details_json_contains_context_and_expected() {
    let error = recoverable_error(RecoverableBoundaryErrorCode::UnsupportedEncode);

    let details = error.details_json();
    let object = details
        .as_object()
        .expect("recoverable details should be an object");

    assert_eq!(object.len(), 2);
    assert_eq!(
        object.get("context"),
        Some(&serde_json::to_value(error.context()).expect("context should serialize"))
    );
    assert_eq!(
        object.get("expected"),
        Some(&serde_json::to_value(error.expected()).expect("expected should serialize"))
    );
}

#[test]
fn recoverable_details_json_includes_optional_detail() {
    let error = recoverable_error(RecoverableBoundaryErrorCode::ArtifactUnavailable)
        .with_detail(serde_json::json!({ "artifactIdentity": "pkg/service" }));

    assert_eq!(
        error
            .details_json()
            .get("detail")
            .and_then(|detail| detail.get("artifactIdentity")),
        Some(&serde_json::json!("pkg/service"))
    );
}

#[test]
fn boundary_payload_uses_domain_wire_codes() {
    let decode_target = RuntimeError::decode_target("std.json.decode", "invalid json");
    let payload = decode_target.payload();
    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(
        payload.details,
        Some(serde_json::json!({
            "target": "std.json.decode",
            "message": "invalid json",
        }))
    );

    let unknown_target = RuntimeError::decode_target("runtime.config", "invalid config");
    assert_eq!(unknown_target.payload().code, "InternalError");

    let resource = RuntimeError::ResourceLimitExceeded {
        resource: "response.body".to_string(),
        reason: "too large".to_string(),
        limit: 10,
        current: 8,
        requested_delta: 4,
    }
    .payload();
    assert_eq!(resource.code, "ResourceLimitExceeded");
    assert_eq!(
        resource.details,
        Some(serde_json::json!({
            "resource": "response.body",
            "reason": "too large",
            "limit": 10,
            "current": 8,
            "requestedDelta": 4,
        }))
    );
}

#[test]
fn boundary_catch_projection_covers_public_catchable_variants() {
    assert_eq!(
        RuntimeError::decode_target("config.require", "missing config").catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::ConfigDecode.catch_identity(),
            serde_json::json!({
                "target": "config.require",
                "message": "missing config",
            })
        ))
    );
    assert_eq!(
        RuntimeError::bytes_decode("bytes.toUtf8String", "invalid utf8").catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::BytesDecode.catch_identity(),
            serde_json::json!({
                "target": "bytes.toUtf8String",
                "message": "invalid utf8",
            })
        ))
    );
    assert_eq!(
        RuntimeError::db_decode("std.db", "missing id").catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::DbDecode.catch_identity(),
            serde_json::json!({
                "target": "std.db",
                "message": "missing id",
            })
        ))
    );
    assert_eq!(
        RuntimeError::file_error("std.file not found").catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::File.catch_identity(),
            serde_json::json!({
                "message": "std.file not found",
            })
        ))
    );
    assert_eq!(
        RuntimeError::http_error(
            "std.http failed",
            Some(serde_json::json!({ "status": 500 }))
        )
        .catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            serde_json::json!({
                "message": "std.http failed",
                "detail": { "status": 500 },
            })
        ))
    );
    assert_eq!(
        RuntimeError::decode_target("runtime.config", "invalid config").catch_projection(),
        None
    );
}
