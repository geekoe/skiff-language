use serde_json::json;

use super::{
    CatchIdentity, NominalTypeIdentity, OpaqueServiceError, PlatformBuiltinErrorIdentity,
    ServiceErrorEnvelope,
};

fn platform_error_bytes(
    identity: &str,
    encoded_payload: &[u8],
    trace_id: &str,
    error_id: &str,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "kind": "platformError",
        "builtinErrorIdentity": identity,
        "encodedPayload": encoded_payload,
        "traceId": trace_id,
        "errorId": error_id,
    }))
    .expect("platform error fixture encodes")
}

#[test]
fn opaque_service_error_preserves_exact_wire_bytes() {
    let payload = json!({
        "kind": "internalError",
        "payload": {
            "message": "boom",
            "traceId": "trace-1",
            "errorId": "error-1"
        }
    });
    let encoded = serde_json::to_vec(&payload).expect("fixture encodes");
    let error = OpaqueServiceError::decode(encoded.clone()).expect("opaque error decodes");
    assert_eq!(error.encoded_bytes(), encoded.as_slice());
    assert!(matches!(
        error.envelope(),
        ServiceErrorEnvelope::InternalError { payload }
            if payload.message == "boom"
    ));
    assert_eq!(error.into_encoded_bytes(), encoded);
}

#[test]
fn platform_builtin_identity_registry_is_frozen_exhaustively() {
    let fixtures = [
        (PlatformBuiltinErrorIdentity::Timeout, "TimeoutError"),
        (
            PlatformBuiltinErrorIdentity::ConfigDecode,
            "config.DecodeError",
        ),
        (
            PlatformBuiltinErrorIdentity::BytesDecode,
            "std.bytes.DecodeError",
        ),
        (
            PlatformBuiltinErrorIdentity::NumberDecode,
            "std.number.DecodeError",
        ),
        (
            PlatformBuiltinErrorIdentity::JsonDecode,
            "std.json.DecodeError",
        ),
        (
            PlatformBuiltinErrorIdentity::DbConflict,
            "std.db.ConflictError",
        ),
        (
            PlatformBuiltinErrorIdentity::DbConstraint,
            "std.db.ConstraintError",
        ),
        (
            PlatformBuiltinErrorIdentity::DbDecode,
            "std.db.DecodeError",
        ),
        (PlatformBuiltinErrorIdentity::File, "std.file.FileError"),
        (
            PlatformBuiltinErrorIdentity::TimeDecode,
            "std.time.DecodeError",
        ),
        (
            PlatformBuiltinErrorIdentity::ServiceProviderUnavailable,
            "std.service.ProviderUnavailableError",
        ),
        (
            PlatformBuiltinErrorIdentity::ServiceProtocol,
            "std.service.ProtocolError",
        ),
        (PlatformBuiltinErrorIdentity::Http, "std.http.HttpError"),
    ];

    for (identity, symbol) in fixtures {
        assert_eq!(identity.symbol(), symbol);
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol(symbol),
            Some(identity)
        );
        assert_eq!(
            serde_json::to_value(identity).expect("identity serializes"),
            json!(symbol)
        );
        assert_eq!(
            serde_json::from_value::<PlatformBuiltinErrorIdentity>(json!(symbol))
                .expect("known identity deserializes"),
            identity
        );
        assert_eq!(
            identity.catch_identity(),
            CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(identity))
        );
    }

    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError"),
        None
    );
    assert!(serde_json::from_value::<PlatformBuiltinErrorIdentity>(json!(
        "std.resource.ResourceError"
    ))
    .is_err());
}

#[test]
fn known_platform_identity_with_valid_nonempty_payload_preserves_exact_bytes() {
    let encoded = platform_error_bytes(
        "std.file.FileError",
        br#"{"message":"file unavailable"}"#,
        "trace-known",
        "error-known",
    );

    let error = OpaqueServiceError::decode(encoded.clone()).expect("known envelope decodes");

    assert_eq!(error.encoded_bytes(), encoded.as_slice());
    assert!(matches!(
        error.envelope(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::File,
            encoded_payload,
            trace_id,
            error_id,
        } if encoded_payload == br#"{"message":"file unavailable"}"#
            && trace_id == "trace-known"
            && error_id == "error-known"
    ));
    assert_eq!(error.into_encoded_bytes(), encoded);
}

#[test]
fn known_identity_codec_malformed_nonempty_payload_is_outer_valid_baseline() {
    // The current outer decoder validates only that encodedPayload is nonempty.
    // Identity-specific codecs reject these bytes later in the service channel.
    let encoded = platform_error_bytes(
        "std.file.FileError",
        b"not-json",
        "trace-malformed",
        "error-malformed",
    );

    let error = OpaqueServiceError::decode(encoded.clone())
        .expect("nonempty payload remains outer-valid in the M0 baseline");

    assert_eq!(error.encoded_bytes(), encoded.as_slice());
    assert!(matches!(
        error.envelope(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::File,
            encoded_payload,
            ..
        } if encoded_payload == b"not-json"
    ));
}

#[test]
fn unknown_platform_identity_fails_closed_in_the_outer_decoder() {
    let encoded = platform_error_bytes(
        "std.future.FutureError",
        br#"{"message":"future"}"#,
        "trace-unknown",
        "error-unknown",
    );

    assert!(OpaqueServiceError::decode(encoded).is_err());
}

#[test]
fn platform_error_correlation_rejects_empty_or_surrounding_whitespace() {
    for (trace_id, error_id) in [
        ("", "error"),
        ("trace", " "),
        (" trace", "error"),
        ("trace", "error "),
    ] {
        let encoded =
            platform_error_bytes("TimeoutError", b"payload", trace_id, error_id);
        assert!(OpaqueServiceError::decode(encoded).is_err());
    }
}
