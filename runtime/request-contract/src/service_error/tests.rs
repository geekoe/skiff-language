use serde_json::{json, Value};
use skiff_artifact_model::{
    platform_error_projection::PlatformErrorProjectionKey, PackageSchemaTypeId,
};

use crate::{
    encode_platform_error_projection_payload, PlatformErrorProjectionCodecError,
    PlatformErrorProjectionPayload, StdFileFileErrorPayload,
};

use super::{
    CatchIdentity, NominalTypeIdentity, OpaqueServiceError, PlatformBuiltinErrorIdentity,
    ServiceErrorDecodeError, ServiceErrorEncodeError, ServiceErrorEnvelope,
    ServiceErrorOuterValidationError, ServiceErrorTextField, ServiceErrorTextViolation,
    MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES, MAX_PLATFORM_ERROR_PROJECTION_KEY_BYTES,
};

fn file_payload(message: &str) -> PlatformErrorProjectionPayload {
    PlatformErrorProjectionPayload::StdFileFileError(StdFileFileErrorPayload {
        message: message.to_owned(),
    })
}

fn current_file_parts(message: &str) -> (String, String, Vec<u8>) {
    let encoded = encode_platform_error_projection_payload(&file_payload(message))
        .expect("generated file payload encodes");
    (
        encoded.projection_key().as_str().to_owned(),
        encoded.entry_fingerprint().to_owned(),
        encoded.into_canonical_payload(),
    )
}

fn valid_unknown_fingerprint() -> String {
    format!("sha256:{}", "0".repeat(64))
}

fn platform_envelope(
    projection_key: &str,
    entry_fingerprint: &str,
    encoded_payload: &[u8],
    trace_id: &str,
    error_id: &str,
) -> ServiceErrorEnvelope {
    ServiceErrorEnvelope::PlatformError {
        projection_key: projection_key.to_owned(),
        entry_fingerprint: entry_fingerprint.to_owned(),
        encoded_payload: encoded_payload.to_vec(),
        trace_id: trace_id.to_owned(),
        error_id: error_id.to_owned(),
    }
}

fn canonical_bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    skiff_canonical_json::canonical_json_bytes(value).expect("fixture canonicalizes")
}

fn canonical_platform_bytes(
    projection_key: &str,
    entry_fingerprint: &str,
    encoded_payload: &[u8],
    trace_id: &str,
    error_id: &str,
) -> Vec<u8> {
    canonical_bytes(&platform_envelope(
        projection_key,
        entry_fingerprint,
        encoded_payload,
        trace_id,
        error_id,
    ))
}

fn expect_invalid_outer(encoded: Vec<u8>) -> ServiceErrorOuterValidationError {
    match OpaqueServiceError::decode(encoded).expect_err("fixture must fail outer validation") {
        ServiceErrorDecodeError::InvalidOuter(error) => error,
        other => panic!("expected invalid outer error, got {other:?}"),
    }
}

#[test]
fn generated_current_pair_uses_canonical_symbol_and_payload() {
    let encoded = encode_platform_error_projection_payload(&file_payload("safe"))
        .expect("generated payload encodes");

    assert_eq!(
        encoded.projection_key(),
        PlatformErrorProjectionKey::StdFileFileError
    );
    assert_eq!(encoded.projection_key().as_str(), "std.file.FileError");
    assert_eq!(encoded.canonical_payload(), br#"{"message":"safe"}"#);
    assert!(encoded.entry_fingerprint().starts_with("sha256:"));
    assert_eq!(encoded.entry_fingerprint().len(), 71);
}

#[test]
fn exact_known_decode_builds_validated_typed_evidence_and_preserves_bytes() {
    let (projection_key, entry_fingerprint, payload) = current_file_parts("unavailable");
    let encoded = canonical_platform_bytes(
        &projection_key,
        &entry_fingerprint,
        &payload,
        "trace-known",
        "error-known",
    );

    let error = OpaqueServiceError::decode(encoded.clone()).expect("exact pair decodes");
    let validated = error
        .known_platform_projection()
        .expect("exact pair creates evidence");
    assert_eq!(
        validated.projection_key(),
        PlatformErrorProjectionKey::StdFileFileError
    );
    assert!(matches!(
        validated.payload(),
        PlatformErrorProjectionPayload::StdFileFileError(StdFileFileErrorPayload { message })
            if message == "unavailable"
    ));
    assert!(matches!(
        error.envelope(),
        ServiceErrorEnvelope::PlatformError {
            projection_key: key,
            entry_fingerprint: fingerprint,
            encoded_payload,
            trace_id,
            error_id,
        } if key == "std.file.FileError"
            && fingerprint == &entry_fingerprint
            && encoded_payload == &payload
            && trace_id == "trace-known"
            && error_id == "error-known"
    ));
    assert_eq!(error.encoded_bytes(), encoded);

    let cacheless = OpaqueServiceError {
        envelope: error.envelope.clone(),
        encoded_bytes: error.encoded_bytes.clone(),
        validated_known_platform_projection: None,
    };
    assert_eq!(error, cacheless, "derived typed cache must not affect Eq");
    assert_eq!(error.into_encoded_bytes(), encoded);
}

#[test]
fn unknown_key_with_non_json_payload_is_opaque_and_bit_exact() {
    let fingerprint = valid_unknown_fingerprint();
    let payload = [0xff, 0x00, b'{'];
    let encoded = canonical_platform_bytes(
        "future.UnknownError",
        &fingerprint,
        &payload,
        "trace-unknown",
        "error-unknown",
    );

    let error = OpaqueServiceError::decode(encoded.clone()).expect("unknown pair stays opaque");
    assert!(error.known_platform_projection().is_none());
    assert_eq!(error.encoded_bytes(), encoded);
    assert_eq!(error.into_encoded_bytes(), encoded);
}

#[test]
fn same_key_different_valid_fingerprint_never_calls_current_codec() {
    let (projection_key, current_fingerprint, _) = current_file_parts("ignored");
    let other_fingerprint = valid_unknown_fingerprint();
    assert_ne!(other_fingerprint, current_fingerprint);
    let non_json = [0xff, 0xfe];
    let encoded = canonical_platform_bytes(
        &projection_key,
        &other_fingerprint,
        &non_json,
        "trace-old",
        "error-old",
    );

    let error = OpaqueServiceError::decode(encoded.clone())
        .expect("same key with another valid fingerprint is opaque");
    assert!(error.known_platform_projection().is_none());
    assert_eq!(error.into_encoded_bytes(), encoded);
}

#[test]
fn exact_known_malformed_and_noncanonical_payloads_are_protocol_errors() {
    let (projection_key, entry_fingerprint, _) = current_file_parts("ignored");
    let malformed = canonical_platform_bytes(
        &projection_key,
        &entry_fingerprint,
        b"not-json",
        "trace-malformed",
        "error-malformed",
    );
    assert!(matches!(
        OpaqueServiceError::decode(malformed),
        Err(ServiceErrorDecodeError::ExactKnownPlatformPayload(
            PlatformErrorProjectionCodecError::MalformedKnownPayload { .. }
        ))
    ));

    let noncanonical_payload = br#"{ "message": "safe" }"#;
    let noncanonical = canonical_platform_bytes(
        &projection_key,
        &entry_fingerprint,
        noncanonical_payload,
        "trace-noncanonical",
        "error-noncanonical",
    );
    assert!(matches!(
        OpaqueServiceError::decode(noncanonical),
        Err(ServiceErrorDecodeError::ExactKnownPlatformPayload(
            PlatformErrorProjectionCodecError::NonCanonicalKnownPayload { .. }
        ))
    ));
}

#[test]
fn platform_wire_is_the_exact_new_hard_cut_shape_and_old_shape_is_rejected() {
    let fingerprint = valid_unknown_fingerprint();
    let envelope = platform_envelope("future.Error", &fingerprint, b"opaque", "trace", "error");
    let value = serde_json::to_value(&envelope).expect("envelope serializes");
    assert_eq!(
        value,
        json!({
            "kind": "platformError",
            "projectionKey": "future.Error",
            "entryFingerprint": fingerprint,
            "encodedPayload": [111, 112, 97, 113, 117, 101],
            "traceId": "trace",
            "errorId": "error",
        })
    );
    assert!(OpaqueServiceError::decode(canonical_bytes(&envelope)).is_ok());

    let old = json!({
        "kind": "platformError",
        "builtinErrorIdentity": "std.file.FileError",
        "encodedPayload": [123, 125],
        "traceId": "trace",
        "errorId": "error",
    });
    assert!(matches!(
        OpaqueServiceError::decode(canonical_bytes(&old)),
        Err(ServiceErrorDecodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidWireShape
        ))
    ));
}

#[test]
fn platform_wire_rejects_every_missing_field_and_any_extra_field() {
    let fingerprint = valid_unknown_fingerprint();
    let base = serde_json::to_value(platform_envelope(
        "future.Error",
        &fingerprint,
        b"opaque",
        "trace",
        "error",
    ))
    .expect("fixture serializes");

    for missing in [
        "projectionKey",
        "entryFingerprint",
        "encodedPayload",
        "traceId",
        "errorId",
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .expect("object fixture")
            .remove(missing);
        assert!(matches!(
            OpaqueServiceError::decode(canonical_bytes(&value)),
            Err(ServiceErrorDecodeError::InvalidOuter(
                ServiceErrorOuterValidationError::InvalidWireShape
            ))
        ));
    }

    let mut extra = base;
    extra["builtinErrorIdentity"] = json!("std.file.FileError");
    assert!(matches!(
        OpaqueServiceError::decode(canonical_bytes(&extra)),
        Err(ServiceErrorDecodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidWireShape
        ))
    ));
}

#[test]
fn projection_key_accepts_only_the_ascii_token_bounds() {
    let fingerprint = valid_unknown_fingerprint();
    for key in [
        "x".to_owned(),
        "a".repeat(MAX_PLATFORM_ERROR_PROJECTION_KEY_BYTES),
    ] {
        let encoded = canonical_platform_bytes(&key, &fingerprint, b"x", "trace", "error");
        assert!(
            OpaqueServiceError::decode(encoded).is_ok(),
            "valid key: {key}"
        );
    }
    let allowed = "A_z-9.Future.Error";
    assert!(OpaqueServiceError::decode(canonical_platform_bytes(
        allowed,
        &fingerprint,
        b"x",
        "trace",
        "error",
    ))
    .is_ok());

    for key in [
        "".to_owned(),
        "a".repeat(MAX_PLATFORM_ERROR_PROJECTION_KEY_BYTES + 1),
    ] {
        assert!(matches!(
            expect_invalid_outer(canonical_platform_bytes(
                &key,
                &fingerprint,
                b"x",
                "trace",
                "error",
            )),
            ServiceErrorOuterValidationError::InvalidProjectionKeyLength { .. }
        ));
    }

    for key in ["bad/key", "bad key", "错误"] {
        assert!(matches!(
            expect_invalid_outer(canonical_platform_bytes(
                key,
                &fingerprint,
                b"x",
                "trace",
                "error",
            )),
            ServiceErrorOuterValidationError::InvalidProjectionKeyCharacter { .. }
        ));
    }
}

#[test]
fn projection_key_rejects_final_numeric_version_suffixes() {
    let fingerprint = valid_unknown_fingerprint();
    for key in [
        "future.Error.v0",
        "future.Error.v000",
        "future.Error.v1",
        "future.Error.v01",
    ] {
        assert_eq!(
            expect_invalid_outer(canonical_platform_bytes(
                key,
                &fingerprint,
                b"x",
                "trace",
                "error",
            )),
            ServiceErrorOuterValidationError::VersionedProjectionKeySuffix
        );
    }

    for key in ["future.Error.v", "future.Error.v1x", "future.v1.Error"] {
        assert!(OpaqueServiceError::decode(canonical_platform_bytes(
            key,
            &fingerprint,
            b"x",
            "trace",
            "error",
        ))
        .is_ok());
    }
}

#[test]
fn entry_fingerprint_requires_exact_lowercase_sha256_shape() {
    let valid = valid_unknown_fingerprint();
    assert!(OpaqueServiceError::decode(canonical_platform_bytes(
        "future.Error",
        &valid,
        b"x",
        "trace",
        "error",
    ))
    .is_ok());

    let invalid = [
        String::new(),
        format!("sha256:{}", "0".repeat(63)),
        format!("sha256:{}", "0".repeat(65)),
        format!("sha256:{}A", "0".repeat(63)),
        format!("sha256:{}g", "0".repeat(63)),
        format!("sha-256:{}", "0".repeat(64)),
        format!("SHA256:{}", "0".repeat(64)),
        format!(" sha256:{}", "0".repeat(64)),
        format!("sha256:{} ", "0".repeat(64)),
    ];
    for fingerprint in invalid {
        assert_eq!(
            expect_invalid_outer(canonical_platform_bytes(
                "future.Error",
                &fingerprint,
                b"x",
                "trace",
                "error",
            )),
            ServiceErrorOuterValidationError::InvalidEntryFingerprint
        );
    }
}

#[test]
fn platform_payload_enforces_one_through_sixty_four_kibibytes() {
    let fingerprint = valid_unknown_fingerprint();
    for payload in [
        vec![b'x'],
        vec![b'x'; MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES],
    ] {
        assert!(OpaqueServiceError::decode(canonical_platform_bytes(
            "future.Error",
            &fingerprint,
            &payload,
            "trace",
            "error",
        ))
        .is_ok());
    }

    for length in [0, MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES + 1] {
        assert_eq!(
            expect_invalid_outer(canonical_platform_bytes(
                "future.Error",
                &fingerprint,
                &vec![b'x'; length],
                "trace",
                "error",
            )),
            ServiceErrorOuterValidationError::InvalidPlatformPayloadLength { length }
        );
    }
}

#[test]
fn correlation_rejects_empty_and_surrounding_whitespace_but_not_inner_space() {
    let fingerprint = valid_unknown_fingerprint();
    assert!(OpaqueServiceError::decode(canonical_platform_bytes(
        "future.Error",
        &fingerprint,
        b"x",
        "trace id",
        "error id",
    ))
    .is_ok());

    for (trace_id, error_id, field, violation) in [
        (
            "",
            "error",
            ServiceErrorTextField::TraceId,
            ServiceErrorTextViolation::Empty,
        ),
        (
            " ",
            "error",
            ServiceErrorTextField::TraceId,
            ServiceErrorTextViolation::Empty,
        ),
        (
            " trace",
            "error",
            ServiceErrorTextField::TraceId,
            ServiceErrorTextViolation::SurroundingWhitespace,
        ),
        (
            "trace",
            "error ",
            ServiceErrorTextField::ErrorId,
            ServiceErrorTextViolation::SurroundingWhitespace,
        ),
    ] {
        assert_eq!(
            expect_invalid_outer(canonical_platform_bytes(
                "future.Error",
                &fingerprint,
                b"x",
                trace_id,
                error_id,
            )),
            ServiceErrorOuterValidationError::InvalidText { field, violation }
        );
    }
}

#[test]
fn opaque_decode_requires_canonical_outer_whitespace_and_field_order() {
    let fingerprint = valid_unknown_fingerprint();
    let canonical = canonical_platform_bytes("future.Error", &fingerprint, b"x", "trace", "error");
    assert!(OpaqueServiceError::decode(canonical.clone()).is_ok());

    let mut whitespace = canonical;
    whitespace.push(b'\n');
    assert!(matches!(
        OpaqueServiceError::decode(whitespace),
        Err(ServiceErrorDecodeError::NonCanonicalOuterBytes)
    ));

    let reordered = format!(
        r#"{{"kind":"platformError","projectionKey":"future.Error","entryFingerprint":"{fingerprint}","encodedPayload":[120],"traceId":"trace","errorId":"error"}}"#
    )
    .into_bytes();
    assert!(matches!(
        OpaqueServiceError::decode(reordered),
        Err(ServiceErrorDecodeError::NonCanonicalOuterBytes)
    ));
}

#[test]
fn outer_validation_and_outer_canonicality_precede_exact_known_codec() {
    let (projection_key, entry_fingerprint, _) = current_file_parts("ignored");
    let mut invalid_outer =
        canonical_platform_bytes(&projection_key, &entry_fingerprint, b"", "trace", "error");
    invalid_outer.push(b'\n');
    assert_eq!(
        expect_invalid_outer(invalid_outer),
        ServiceErrorOuterValidationError::InvalidPlatformPayloadLength { length: 0 }
    );

    let mut malformed_payload_noncanonical_outer = canonical_platform_bytes(
        &projection_key,
        &entry_fingerprint,
        b"not-json",
        "trace",
        "error",
    );
    malformed_payload_noncanonical_outer.push(b'\n');
    assert!(matches!(
        OpaqueServiceError::decode(malformed_payload_noncanonical_outer),
        Err(ServiceErrorDecodeError::NonCanonicalOuterBytes)
    ));
}

#[test]
fn local_platform_constructor_emits_canonical_exact_known_carrier() {
    let payload = file_payload("safe");
    let local = OpaqueServiceError::platform_error(&payload, "trace-local", "error-local")
        .expect("local platform error encodes");
    assert_eq!(local.encoded_bytes(), canonical_bytes(local.envelope()));
    assert_eq!(
        local
            .known_platform_projection()
            .expect("local exact pair is known")
            .payload(),
        &payload
    );
    assert!(matches!(
        local.envelope(),
        ServiceErrorEnvelope::PlatformError {
            projection_key,
            entry_fingerprint,
            trace_id,
            error_id,
            ..
        } if projection_key == "std.file.FileError"
            && entry_fingerprint.starts_with("sha256:")
            && trace_id == "trace-local"
            && error_id == "error-local"
    ));

    let decoded = OpaqueServiceError::decode(local.encoded_bytes().to_vec())
        .expect("local bytes decode through inbound validation");
    assert_eq!(local, decoded);
}

#[test]
fn oversized_local_platform_encode_is_typed_and_internal_constructor_can_reuse_correlation() {
    let trace_id = "trace-same";
    let error_id = "error-same";
    let oversized = file_payload(&"x".repeat(MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES));
    let error = OpaqueServiceError::platform_error(&oversized, trace_id, error_id)
        .expect_err("generated payload exceeds the outer carrier bound");
    assert!(matches!(
        error,
        ServiceErrorEncodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidPlatformPayloadLength { length }
        ) if length > MAX_PLATFORM_ERROR_ENCODED_PAYLOAD_BYTES
    ));

    // This proves the variant-specific capability only. Production exporter
    // fallback ownership is intentionally outside this crate/M3 task.
    let internal = OpaqueServiceError::internal_error(
        "The service could not complete the request.",
        trace_id,
        error_id,
    )
    .expect("internal carrier accepts the same canonical correlation");
    assert_eq!(internal.envelope().trace_id(), trace_id);
    assert_eq!(internal.envelope().error_id(), error_id);
}

#[test]
fn internal_and_public_typed_constructors_round_trip_canonical_bytes() {
    let internal = OpaqueServiceError::internal_error(
        "The service could not complete the request.",
        "trace-internal",
        "error-internal",
    )
    .expect("internal carrier encodes");
    let public = OpaqueServiceError::public_typed_error(
        "example.errors",
        "NotFound",
        PackageSchemaTypeId::new("schema:not-found"),
        br#"{"id":"42"}"#,
        "trace-public",
        "error-public",
    )
    .expect("public typed carrier encodes");

    for carrier in [internal, public] {
        assert_eq!(carrier.encoded_bytes(), canonical_bytes(carrier.envelope()));
        assert!(carrier.known_platform_projection().is_none());
        let decoded = OpaqueServiceError::decode(carrier.encoded_bytes().to_vec())
            .expect("canonical local carrier decodes");
        assert_eq!(carrier, decoded);
    }
}

#[test]
fn malformed_json_and_invalid_outer_are_disjoint_typed_failures() {
    assert!(matches!(
        OpaqueServiceError::decode(br#"{"kind":"platformError"#.to_vec()),
        Err(ServiceErrorDecodeError::MalformedJson { .. })
    ));
    assert!(matches!(
        OpaqueServiceError::decode(canonical_bytes(&json!({"kind": "futureError"}))),
        Err(ServiceErrorDecodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidWireShape
        ))
    ));
}

#[test]
fn error_display_never_includes_payload_correlation_or_envelope_bytes() {
    let fingerprint = valid_unknown_fingerprint();
    let invalid = canonical_platform_bytes(
        "future.Error",
        &fingerprint,
        b"raw-secret-payload",
        " secret-trace",
        "secret-error",
    );
    let outer = OpaqueServiceError::decode(invalid).expect_err("correlation is invalid");
    let display = outer.to_string();
    for secret in ["raw-secret-payload", "secret-trace", "secret-error"] {
        assert!(!display.contains(secret));
    }

    let (projection_key, entry_fingerprint, _) = current_file_parts("ignored");
    let codec = OpaqueServiceError::decode(canonical_platform_bytes(
        &projection_key,
        &entry_fingerprint,
        b"raw-codec-secret",
        "trace-codec-secret",
        "error-codec-secret",
    ))
    .expect_err("exact malformed payload is rejected");
    let display = codec.to_string();
    for secret in [
        "raw-codec-secret",
        "trace-codec-secret",
        "error-codec-secret",
    ] {
        assert!(!display.contains(secret));
    }
}

#[test]
fn transitional_platform_builtin_identity_remains_local_only() {
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.db.ConflictError"),
        Some(PlatformBuiltinErrorIdentity::DbConflict)
    );
    assert_eq!(
        PlatformBuiltinErrorIdentity::DbConflict.catch_identity(),
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(
            PlatformBuiltinErrorIdentity::DbConflict
        ))
    );
    assert!(PlatformBuiltinErrorIdentity::from_symbol("future.Error").is_none());
}

#[test]
fn direct_envelope_deserialization_validates_outer_but_fixing_requires_canonical_bytes() {
    let fingerprint = valid_unknown_fingerprint();
    let reordered = format!(
        r#"{{"kind":"platformError","projectionKey":"future.Error","entryFingerprint":"{fingerprint}","encodedPayload":[120],"traceId":"trace","errorId":"error"}}"#
    );
    let envelope = serde_json::from_str::<ServiceErrorEnvelope>(&reordered)
        .expect("direct DTO parse validates shape and outer fields");
    assert!(matches!(
        envelope,
        ServiceErrorEnvelope::PlatformError { .. }
    ));
    assert!(matches!(
        OpaqueServiceError::decode(reordered.into_bytes()),
        Err(ServiceErrorDecodeError::NonCanonicalOuterBytes)
    ));
}

#[test]
fn public_and_internal_outer_validation_remains_strict() {
    let empty_public = ServiceErrorEnvelope::PublicTypedError {
        package_id: "example.errors".to_owned(),
        stable_schema_key: "NotFound".to_owned(),
        package_schema_type_id: PackageSchemaTypeId::new("schema:not-found"),
        encoded_payload: Vec::new(),
        trace_id: "trace".to_owned(),
        error_id: "error".to_owned(),
    };
    assert_eq!(
        empty_public.validate(),
        Err(ServiceErrorOuterValidationError::EmptyPublicTypedPayload)
    );

    assert!(matches!(
        OpaqueServiceError::internal_error(" ", "trace", "error"),
        Err(ServiceErrorEncodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidText {
                field: ServiceErrorTextField::InternalMessage,
                violation: ServiceErrorTextViolation::Empty,
            }
        ))
    ));
    assert!(matches!(
        OpaqueServiceError::public_typed_error(
            " example.errors",
            "NotFound",
            PackageSchemaTypeId::new("schema:not-found"),
            b"x",
            "trace",
            "error",
        ),
        Err(ServiceErrorEncodeError::InvalidOuter(
            ServiceErrorOuterValidationError::InvalidText {
                field: ServiceErrorTextField::PackageId,
                violation: ServiceErrorTextViolation::SurroundingWhitespace,
            }
        ))
    ));
}

#[test]
fn encoded_payload_stays_a_raw_byte_array_in_outer_json() {
    let fingerprint = valid_unknown_fingerprint();
    let value = serde_json::to_value(platform_envelope(
        "future.Error",
        &fingerprint,
        &[0, 127, 128, 255],
        "trace",
        "error",
    ))
    .expect("envelope serializes");
    assert_eq!(value["encodedPayload"], json!([0, 127, 128, 255]));

    let canonical = canonical_bytes(&value);
    let decoded = OpaqueServiceError::decode(canonical.clone()).expect("raw bytes decode");
    assert_eq!(decoded.encoded_bytes(), canonical);
    let ServiceErrorEnvelope::PlatformError {
        encoded_payload, ..
    } = decoded.envelope()
    else {
        panic!("expected platform envelope");
    };
    assert_eq!(encoded_payload, &[0, 127, 128, 255]);
}

#[test]
fn canonical_value_fixture_is_stable_for_outer_order_checks() {
    let fingerprint = valid_unknown_fingerprint();
    let envelope = platform_envelope("future.Error", &fingerprint, b"x", "trace", "error");
    let canonical = canonical_bytes(&envelope);
    let value: Value = serde_json::from_slice(&canonical).expect("canonical fixture parses");
    assert_eq!(value["kind"], "platformError");
    assert_eq!(canonical, canonical_bytes(&value));
}
