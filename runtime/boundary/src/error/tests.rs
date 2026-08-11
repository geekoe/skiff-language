use std::borrow::Cow;

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

fn diagnostic_attributes(error: &dyn RuntimeDiagnostic) -> DiagnosticAttributes {
    let mut attributes = DiagnosticAttributes::new();
    error.record_diagnostic_attributes(&mut attributes);
    attributes
}

#[test]
fn recoverable_error_representation_stays_below_large_result_threshold() {
    const LARGE_RESULT_ERROR_LIMIT_BYTES: usize = 128;

    let recoverable_size = std::mem::size_of::<RecoverableBoundaryError>();
    assert!(
        recoverable_size <= LARGE_RESULT_ERROR_LIMIT_BYTES,
        "RecoverableBoundaryError is {recoverable_size} bytes"
    );

    let runtime_error_size = std::mem::size_of::<RuntimeError>();
    assert!(
        runtime_error_size <= LARGE_RESULT_ERROR_LIMIT_BYTES,
        "boundary RuntimeError is {runtime_error_size} bytes"
    );
}

#[test]
fn recoverable_error_codes_are_stable() {
    for (fixture_index, (code, expected)) in RECOVERABLE_ERROR_CODE_FIXTURE.into_iter().enumerate()
    {
        assert_eq!(recoverable_error_code_fixture_index(code), fixture_index);
        assert_eq!(code.as_str(), expected);
    }
}

#[test]
fn recoverable_diagnostics_cover_all_codes_without_exposing_details() {
    for (code, expected_code) in RECOVERABLE_ERROR_CODE_FIXTURE {
        let error = recoverable_error(code).with_detail(serde_json::json!({
            "secret": "recoverable-detail-secret",
        }));
        let diagnostic: &dyn RuntimeDiagnostic = &error;

        assert_eq!(diagnostic.diagnostic_code().as_str(), expected_code);
        assert_eq!(
            diagnostic.diagnostic_message(),
            Cow::Borrowed("recoverable boundary is unsupported")
        );
        assert!(diagnostic_attributes(diagnostic).is_empty());

        let outer = RuntimeError::from(error);
        let outer_diagnostic: &dyn RuntimeDiagnostic = &outer;
        assert_eq!(outer_diagnostic.diagnostic_code().as_str(), expected_code);
        assert_eq!(
            outer_diagnostic.diagnostic_message(),
            Cow::Borrowed("recoverable boundary is unsupported")
        );
        assert!(diagnostic_attributes(outer_diagnostic).is_empty());

        let payload = outer.payload();
        assert_eq!(payload.code, expected_code);
        assert_eq!(payload.message, "recoverable boundary is unsupported");
        assert_eq!(outer.catch_projection(), None);
    }
}

#[test]
fn runtime_error_diagnostics_cover_all_variants_and_match_wire_messages() {
    let json_source = serde_json::from_str::<serde_json::Value>("{")
        .expect_err("fixture should be malformed JSON");
    let json_message = json_source.to_string();
    let fixtures = [
        (
            RuntimeError::InvalidArtifact("invalid package artifact".to_string()),
            "InvalidArtifact",
            "invalid package artifact".to_string(),
            false,
        ),
        (
            RuntimeError::Decode("internal value decode failed".to_string()),
            "InternalError",
            "internal value decode failed".to_string(),
            false,
        ),
        (
            RuntimeError::decode_target("std.json.decode", "invalid json"),
            "std.json.DecodeError",
            "invalid json".to_string(),
            false,
        ),
        (
            RuntimeError::bytes_decode("private-bytes-target", "invalid utf8"),
            "std.bytes.DecodeError",
            "invalid utf8".to_string(),
            false,
        ),
        (
            RuntimeError::db_decode("private-db-target", "invalid row"),
            "std.db.DecodeError",
            "invalid row".to_string(),
            false,
        ),
        (
            RuntimeError::file_error("private path /srv/secret"),
            "std.file.FileError",
            "private path /srv/secret".to_string(),
            false,
        ),
        (
            RuntimeError::http_error(
                "private upstream text",
                Some(serde_json::json!({ "secret": "http-detail-secret" })),
            ),
            "std.http.HttpError",
            "private upstream text".to_string(),
            false,
        ),
        (
            RuntimeError::Unsupported("runtime feature is unsupported".to_string()),
            "UnsupportedRuntimeFeature",
            "runtime feature is unsupported".to_string(),
            false,
        ),
        (
            RuntimeError::from(
                recoverable_error(RecoverableBoundaryErrorCode::ArtifactUnavailable)
                    .with_detail(serde_json::json!({ "secret": "recoverable-detail-secret" })),
            ),
            "recoverable_artifact_unavailable",
            "recoverable boundary is unsupported".to_string(),
            false,
        ),
        (
            RuntimeError::ResourceLimitExceeded {
                resource: "private-resource".to_string(),
                reason: "private reason text".to_string(),
                limit: 1024,
                current: 900,
                requested_delta: 200,
            },
            "ResourceLimitExceeded",
            "resource limit exceeded for private-resource: private reason text".to_string(),
            true,
        ),
        (
            RuntimeError::Json(json_source),
            "JsonError",
            json_message,
            true,
        ),
    ];

    for (error, expected_code, expected_message, expect_owned_message) in fixtures {
        let diagnostic: &dyn RuntimeDiagnostic = &error;
        assert_eq!(diagnostic.diagnostic_code().as_str(), expected_code);

        let diagnostic_message = diagnostic.diagnostic_message();
        assert_eq!(diagnostic_message.as_ref(), expected_message.as_str());
        assert_eq!(
            matches!(diagnostic_message, Cow::Owned(_)),
            expect_owned_message,
            "unexpected message ownership for {expected_code}"
        );

        let attributes = diagnostic_attributes(diagnostic);
        if expected_code == "ResourceLimitExceeded" {
            assert_eq!(
                attributes
                    .iter()
                    .map(|(key, value)| (key.as_str(), *value))
                    .collect::<Vec<_>>(),
                vec![
                    ("limit", DiagnosticFieldValue::U64(1024)),
                    ("current", DiagnosticFieldValue::U64(900)),
                    ("requested_delta", DiagnosticFieldValue::U64(200)),
                ]
            );
            assert!(!attributes.was_truncated());
        } else {
            assert!(
                attributes.is_empty(),
                "{expected_code} exposed unexpected attributes: {attributes:?}"
            );
        }

        let payload = error.payload();
        assert_eq!(payload.code, expected_code);
        assert_eq!(payload.message, expected_message);
        assert!(error.as_any().is::<RuntimeError>());
    }
}

#[test]
fn decode_target_diagnostics_use_only_the_closed_wire_mapping() {
    let fixtures = [
        ("std.json.decode", "std.json.DecodeError"),
        ("std.json.encode", "std.json.DecodeError"),
        ("std.resource.json", "std.json.DecodeError"),
        ("config.require", "config.DecodeError"),
        ("config.optional", "config.DecodeError"),
        ("config.has", "config.DecodeError"),
        ("number.parse", "std.number.DecodeError"),
        ("number.assertSafeInteger", "std.number.DecodeError"),
        ("Date.parse", "std.time.DecodeError"),
        ("Duration.from", "std.time.DecodeError"),
        ("private.unknown.target", "InternalError"),
    ];

    for (target, expected_code) in fixtures {
        let error = RuntimeError::decode_target(target, "private decode text");
        assert_eq!(
            RuntimeDiagnostic::diagnostic_code(&error).as_str(),
            expected_code
        );
        assert_eq!(error.payload().code, expected_code);
        assert!(diagnostic_attributes(&error).is_empty());
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
fn matching_diagnostic_and_wire_codes_do_not_grant_catch_projection() {
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
        assert_eq!(
            RuntimeDiagnostic::diagnostic_code(&error).as_str(),
            error.payload().code
        );
        assert_eq!(
            error.catch_projection(),
            None,
            "unexpected catch for {error}"
        );
        assert!(error.as_any().is::<RuntimeError>());
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
