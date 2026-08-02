use serde_json::json;

use super::{OpaqueServiceError, PlatformBuiltinErrorIdentity, ServiceErrorEnvelope};

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
fn platform_builtin_identity_maps_symbols() {
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("TimeoutError"),
        Some(PlatformBuiltinErrorIdentity::Timeout)
    );
    assert_eq!(
        PlatformBuiltinErrorIdentity::DbConflict.symbol(),
        "std.db.ConflictError"
    );
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError"),
        None
    );
}
