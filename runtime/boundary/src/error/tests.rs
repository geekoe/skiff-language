use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
    RuntimeRecoverableTrustBoundary,
};

use super::*;

const RECOVERABLE_ERROR_CODE_FIXTURE: [(RecoverableBoundaryErrorCode, &str); 13] = [
    (
        RecoverableBoundaryErrorCode::UnsupportedEncode,
        "recoverableUnsupportedEncode",
    ),
    (
        RecoverableBoundaryErrorCode::UnsupportedDecode,
        "recoverableUnsupportedDecode",
    ),
    (
        RecoverableBoundaryErrorCode::CodeIdentityMissing,
        "recoverable_code_identity_missing",
    ),
    (
        RecoverableBoundaryErrorCode::ArtifactUnavailable,
        "recoverable_artifact_unavailable",
    ),
    (
        RecoverableBoundaryErrorCode::NativeMissingAdapter,
        "recoverable_native_missing_adapter",
    ),
    (
        RecoverableBoundaryErrorCode::ExpectedTypeMismatch,
        "recoverable_expected_type_mismatch",
    ),
    (
        RecoverableBoundaryErrorCode::InterfaceConformanceMissing,
        "recoverable_interface_conformance_missing",
    ),
    (
        RecoverableBoundaryErrorCode::StateInvalid,
        "recoverable_state_invalid",
    ),
    (
        RecoverableBoundaryErrorCode::CrossServiceInterfaceCallbackUnavailable,
        "cross_service_interface_callback_unavailable",
    ),
    (
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable,
        "callback_capability_not_recoverable",
    ),
    (
        RecoverableBoundaryErrorCode::CrossServiceRecoverableBehaviorUnavailable,
        "cross_service_recoverable_behavior_unavailable",
    ),
    (
        RecoverableBoundaryErrorCode::UntrustedBehaviorPayload,
        "recoverable_untrusted_behavior_payload",
    ),
    (
        RecoverableBoundaryErrorCode::SealedPayloadInvalid,
        "recoverable_sealed_payload_invalid",
    ),
];

fn recoverable_error_code_fixture_index(code: RecoverableBoundaryErrorCode) -> usize {
    match code {
        RecoverableBoundaryErrorCode::UnsupportedEncode => 0,
        RecoverableBoundaryErrorCode::UnsupportedDecode => 1,
        RecoverableBoundaryErrorCode::CodeIdentityMissing => 2,
        RecoverableBoundaryErrorCode::ArtifactUnavailable => 3,
        RecoverableBoundaryErrorCode::NativeMissingAdapter => 4,
        RecoverableBoundaryErrorCode::ExpectedTypeMismatch => 5,
        RecoverableBoundaryErrorCode::InterfaceConformanceMissing => 6,
        RecoverableBoundaryErrorCode::StateInvalid => 7,
        RecoverableBoundaryErrorCode::CrossServiceInterfaceCallbackUnavailable => 8,
        RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable => 9,
        RecoverableBoundaryErrorCode::CrossServiceRecoverableBehaviorUnavailable => 10,
        RecoverableBoundaryErrorCode::UntrustedBehaviorPayload => 11,
        RecoverableBoundaryErrorCode::SealedPayloadInvalid => 12,
    }
}

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
    for (fixture_index, (code, expected)) in
        RECOVERABLE_ERROR_CODE_FIXTURE.into_iter().enumerate()
    {
        assert_eq!(recoverable_error_code_fixture_index(code), fixture_index);
        assert_eq!(code.as_str(), expected);
    }
}

#[test]
fn recoverable_constructor_accessors_clone_eq_display_and_source_are_stable() {
    let context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableTrustBoundary::OwnerInternal,
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    let expected = RuntimeRecoverableExpectedTypePlan::unresolved("string");
    let error = RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::UnsupportedEncode,
        "recoverable boundary is unsupported",
        &context,
        &expected,
    );

    assert_eq!(
        error.code(),
        RecoverableBoundaryErrorCode::UnsupportedEncode
    );
    assert_eq!(error.message(), "recoverable boundary is unsupported");
    assert_eq!(error.context(), &context);
    assert_eq!(error.expected(), &expected);
    assert_eq!(error.detail(), None);
    assert_eq!(error.clone(), error);
    assert_eq!(
        error.to_string(),
        "recoverable boundary error recoverableUnsupportedEncode: recoverable boundary is unsupported"
    );
    assert!(
        <RecoverableBoundaryError as std::error::Error>::source(&error).is_none(),
        "the leaf error currently has no Rust source"
    );

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
fn recoverable_with_detail_updates_accessor_and_diagnostic_details() {
    let detail = serde_json::json!({ "artifactIdentity": "pkg/service" });
    let error = recoverable_error(RecoverableBoundaryErrorCode::ArtifactUnavailable)
        .with_detail(detail.clone());

    assert_eq!(error.detail(), Some(&detail));

    assert_eq!(
        error
            .details_json()
            .get("detail")
            .and_then(|detail| detail.get("artifactIdentity")),
        Some(&serde_json::json!("pkg/service"))
    );
}

#[test]
fn recoverable_runtime_error_is_diagnostic_only() {
    let recoverable = recoverable_error(RecoverableBoundaryErrorCode::ArtifactUnavailable)
        .with_detail(serde_json::json!({ "artifactIdentity": "pkg/service" }));
    let expected_details = recoverable.details_json();
    let error = RuntimeError::from(recoverable);

    let payload = error.payload();
    assert_eq!(payload.code, "recoverable_artifact_unavailable");
    assert_eq!(payload.message, "recoverable boundary is unsupported");
    assert_eq!(payload.details, Some(expected_details));
    assert_eq!(error.catch_projection(), None);
}

#[test]
fn generic_json_source_error_is_diagnostic_only() {
    let source = serde_json::from_str::<serde_json::Value>("{")
        .expect_err("fixture should be malformed JSON");
    let error = RuntimeError::from(source);

    assert!(matches!(&error, RuntimeError::Json(_)));
    assert_eq!(error.payload().code, "JsonError");
    assert_eq!(error.catch_projection(), None);
}

#[test]
fn diagnostic_only_runtime_error_variants_have_no_catch_projection() {
    let errors = [
        RuntimeError::InvalidArtifact("invalid package artifact".to_string()),
        RuntimeError::Decode("internal value decode failed".to_string()),
        RuntimeError::decode_target("runtime.config", "invalid config"),
        RuntimeError::Unsupported("runtime feature is unsupported".to_string()),
        RuntimeError::ResourceLimitExceeded {
            resource: "response.body".to_string(),
            reason: "too large".to_string(),
            limit: 10,
            current: 8,
            requested_delta: 4,
        },
    ];

    for error in errors {
        assert_eq!(error.catch_projection(), None, "unexpected catch for {error}");
    }
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
}
