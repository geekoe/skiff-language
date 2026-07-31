use serde_json::json;
use skiff_artifact_model::{SourcePosition, SourceSpanRef};

use super::*;
use crate::{
    addr::{FileAddr, UnitAddr},
    value::{RuntimeValue, RuntimeValueCarrier},
};

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 7,
            start: SourcePosition::new(3, 4),
            end: SourcePosition::new(3, 9),
        },
    }
}

fn public_envelope() -> ServiceErrorEnvelope {
    ServiceErrorEnvelope::PublicTypedError {
        package_id: "example.errors".to_string(),
        stable_schema_key: "NotFound".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("schema:not-found"),
        encoded_payload: br#"{"id":"42"}"#.to_vec(),
        trace_id: "trace-1".to_string(),
        error_id: "error-1".to_string(),
    }
}

fn internal_envelope() -> ServiceErrorEnvelope {
    ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "The service could not complete the request.".to_string(),
            trace_id: "trace-1".to_string(),
            error_id: "error-2".to_string(),
        },
    }
}

fn platform_envelope() -> ServiceErrorEnvelope {
    ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
        encoded_payload: br#"{"retryable":true}"#.to_vec(),
        trace_id: "trace-1".to_string(),
        error_id: "error-3".to_string(),
    }
}

#[test]
fn legacy_cancel_platform_error_envelope_is_rejected_by_the_finite_registry() {
    let legacy = r#"{
          "kind": "platformError",
          "builtinErrorIdentity": "CancelError",
          "encodedPayload": [],
          "traceId": "trace-cancel",
          "errorId": "error-cancel"
        }"#;

    let error = serde_json::from_str::<ServiceErrorEnvelope>(legacy).unwrap_err();
    assert!(
        error.to_string().contains("unknown variant `CancelError`"),
        "legacy identity must be rejected before payload validation: {error}"
    );
}

#[test]
fn legacy_cancel_symbol_is_not_a_platform_builtin_identity() {
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("CancelError"),
        None
    );
}

#[test]
fn database_constraint_platform_identity_round_trips_exactly() {
    let identity = PlatformBuiltinErrorIdentity::DbConstraint;

    assert_eq!(identity.symbol(), "std.db.ConstraintError");
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.db.ConstraintError"),
        Some(identity)
    );
    assert_eq!(
        serde_json::to_string(&identity).unwrap(),
        r#""std.db.ConstraintError""#
    );
}

#[test]
fn legacy_cancel_json_string_is_rejected_by_the_finite_registry() {
    assert!(serde_json::from_str::<PlatformBuiltinErrorIdentity>(r#""CancelError""#).is_err());
}

#[test]
fn websocket_request_errors_keep_all_five_exact_named_union_branch_identities() {
    let owner = NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::loaded_file(0),
            type_index: 42,
        },
        type_arguments: Vec::new(),
    });
    for kind in WebSocketRequestErrorKind::ALL {
        let remote = kind == WebSocketRequestErrorKind::Remote;
        let error = WebSocketRequestError::new(
            owner.clone(),
            kind,
            "sanitized",
            remote.then_some(-32603),
            remote.then(|| json!({"peer": true})),
        )
        .expect("exact WebSocket request branch");
        assert_eq!(
            error.exact_catch_identity(),
            CatchIdentity::NamedUnionBranch {
                union: owner.clone(),
                branch: NamedUnionBranchIdentity::SyntheticDiscriminator {
                    discriminator_field: "kind".to_string(),
                    discriminator_value: kind.discriminator().to_string(),
                },
            }
        );
        assert_eq!(
            error.catch_projection().unwrap().1["kind"],
            kind.discriminator()
        );
    }
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("std.websocket.WebSocketRequestError"),
        None
    );
    assert_eq!(
        PlatformBuiltinErrorIdentity::JsonDecode.catch_identity(),
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(
            PlatformBuiltinErrorIdentity::JsonDecode
        ))
    );
    assert_eq!(
        PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(
            PlatformBuiltinErrorIdentity::Timeout
        ))
    );
}

#[test]
fn timeout_platform_identity_and_envelope_round_trip_unchanged() {
    let identity = PlatformBuiltinErrorIdentity::Timeout;

    assert_eq!(identity.symbol(), "TimeoutError");
    assert_eq!(
        PlatformBuiltinErrorIdentity::from_symbol("TimeoutError"),
        Some(identity)
    );
    assert_eq!(
        identity.catch_identity(),
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(identity))
    );

    let identity_json = serde_json::to_string(&identity).unwrap();
    assert_eq!(identity_json, r#""TimeoutError""#);
    assert_eq!(
        serde_json::from_str::<PlatformBuiltinErrorIdentity>(&identity_json).unwrap(),
        identity
    );

    let envelope = ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: identity,
        encoded_payload: br#"{"message":"deadline exceeded"}"#.to_vec(),
        trace_id: "trace-timeout".to_string(),
        error_id: "error-timeout".to_string(),
    };
    let wire = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(
        serde_json::from_slice::<ServiceErrorEnvelope>(&wire).unwrap(),
        envelope
    );
}

fn exact_public_bytes() -> Vec<u8> {
    br#"{
          "kind":"publicTypedError",
          "packageId":"example.errors",
          "stableSchemaKey":"NotFound",
          "packageSchemaTypeId":"schema:not-found",
          "encodedPayload":[123,125],
          "traceId":"trace-1",
          "errorId":"error-1"
        }"#
    .to_vec()
}

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

#[test]
fn service_error_envelopes_round_trip_all_variants() {
    let envelopes = [public_envelope(), internal_envelope(), platform_envelope()];

    for expected in envelopes {
        let wire = serde_json::to_value(&expected).unwrap();
        assert_eq!(
            serde_json::from_value::<ServiceErrorEnvelope>(wire).unwrap(),
            expected
        );
    }
}

#[test]
fn service_error_envelope_strictly_rejects_invalid_wire() {
    let base = serde_json::to_value(public_envelope()).unwrap();
    let mut cases = Vec::new();

    let mut unknown_variant = base.clone();
    unknown_variant["kind"] = json!("futureError");
    cases.push(unknown_variant);

    let mut extra = base.clone();
    extra["details"] = json!({});
    cases.push(extra);

    for missing in [
        "packageId",
        "stableSchemaKey",
        "packageSchemaTypeId",
        "encodedPayload",
        "traceId",
        "errorId",
    ] {
        let mut value = base.clone();
        value.as_object_mut().unwrap().remove(missing);
        cases.push(value);
    }

    let mut empty_owner = base.clone();
    empty_owner["packageId"] = json!(" ");
    cases.push(empty_owner);

    let mut unknown_builtin = serde_json::to_value(ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: PlatformBuiltinErrorIdentity::DbConflict,
        encoded_payload: vec![1],
        trace_id: "trace".to_string(),
        error_id: "error".to_string(),
    })
    .unwrap();
    unknown_builtin["builtinErrorIdentity"] = json!("std.resource.ResourceError");
    cases.push(unknown_builtin);

    let mut internal_extra = serde_json::to_value(ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "sanitized".to_string(),
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        },
    })
    .unwrap();
    internal_extra["payload"]["details"] = json!({ "private": true });
    cases.push(internal_extra);

    let mut internal_missing = serde_json::to_value(ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "sanitized".to_string(),
            trace_id: "trace".to_string(),
            error_id: "error".to_string(),
        },
    })
    .unwrap();
    internal_missing["payload"]
        .as_object_mut()
        .unwrap()
        .remove("message");
    cases.push(internal_missing);

    for case in cases {
        assert!(
            serde_json::from_value::<ServiceErrorEnvelope>(case).is_err(),
            "invalid service envelope must fail closed"
        );
    }
}

#[test]
fn linked_imported_error_catches_exactly_and_preserves_fixed_bytes() {
    let encoded = exact_public_bytes();
    let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
    let identity = local_identity(4);
    let local_value =
        RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
    let exception =
        RequestException::imported(opaque, Some(local_value), site(), Vec::new()).unwrap();

    assert_eq!(exception.local_catch_identity(), Some(&identity));
    assert_eq!(
        exception.fixed_service_error().unwrap().encoded_bytes(),
        encoded
    );
    let RequestExceptionCause::OpaqueService {
        error,
        local_value: Some(local_value),
    } = exception.cause()
    else {
        panic!("expected linked imported cause");
    };
    assert_eq!(error.encoded_bytes(), encoded);
    assert_eq!(local_value.catch_identity(), Some(&identity));

    let mapped = exception.map_local_value(|_| {
        RuntimeValueCarrier::identified(RuntimeValue::from("moved"), identity.clone())
    });
    assert_eq!(mapped.local_catch_identity(), Some(&identity));
    assert_eq!(
        mapped.fixed_service_error().unwrap().encoded_bytes(),
        encoded
    );
}

#[test]
fn unlinked_imported_error_misses_catch_and_map_keeps_fixed_bytes() {
    let encoded = exact_public_bytes();
    let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
    let exception = RequestException::imported(opaque, None, site(), Vec::new()).unwrap();

    assert_eq!(exception.local_catch_identity(), None);
    assert_eq!(exception.local_value(), None);
    let mapped = exception.map_local_value(|_| panic!("None must not materialize a carrier"));
    assert_eq!(mapped.local_catch_identity(), None);
    assert_eq!(mapped.local_value(), None);
    assert_eq!(
        mapped.fixed_service_error().unwrap().encoded_bytes(),
        encoded
    );
}

#[test]
fn every_fixed_error_kind_can_retain_a_local_carrier() {
    for (type_index, expected) in [
        (7, public_envelope()),
        (8, internal_envelope()),
        (9, platform_envelope()),
    ] {
        let encoded = serde_json::to_vec(&expected).unwrap();
        let opaque = OpaqueServiceError::decode(encoded.clone()).unwrap();
        let identity = local_identity(type_index);
        let local_value =
            RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
        let exception =
            RequestException::imported(opaque, Some(local_value), site(), Vec::new()).unwrap();

        assert_eq!(exception.local_catch_identity(), Some(&identity));
        assert_eq!(
            exception.fixed_service_error().unwrap().envelope(),
            &expected
        );
        assert_eq!(
            exception.fixed_service_error().unwrap().encoded_bytes(),
            encoded
        );
    }
}

#[test]
fn local_exception_rethrow_state_stays_local_and_has_no_fixed_error() {
    let identity = local_identity(4);
    let source = site();
    let stack = vec![
        ExceptionStackFrame::Local {
            site: source.clone(),
        },
        ExceptionStackFrame::RemoteBoundary {
            service_id: "skiff.run/catalog".to_string(),
            operation_id: "lookup".to_string(),
            error_id: "error".to_string(),
        },
    ];
    let correlation = ErrorCorrelation {
        trace_id: "trace".to_string(),
        error_id: "error".to_string(),
    };
    let value = RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone());
    let exception =
        RequestException::local(value, source.clone(), stack.clone(), correlation.clone()).unwrap();
    let rethrown = exception.map_local_value(|_| {
        RuntimeValueCarrier::identified(RuntimeValue::from("moved"), identity.clone())
    });

    assert_eq!(rethrown.local_catch_identity(), Some(&identity));
    assert_eq!(rethrown.fixed_service_error(), None);
    assert_eq!(rethrown.source(), &source);
    assert_eq!(rethrown.stack(), stack);
    assert_eq!(rethrown.correlation(), &correlation);
    assert!(matches!(
        rethrown.cause(),
        RequestExceptionCause::Local { .. }
    ));
}

#[test]
fn local_exception_rejects_missing_identity_stack_and_correlation() {
    let identity = local_identity(5);
    let correlation = ErrorCorrelation {
        trace_id: "trace".to_string(),
        error_id: "error".to_string(),
    };
    assert!(RequestException::local(
        RuntimeValue::from("payload").into(),
        site(),
        vec![ExceptionStackFrame::Local { site: site() }],
        correlation.clone(),
    )
    .is_err());
    assert!(RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity.clone(),),
        site(),
        Vec::new(),
        correlation,
    )
    .is_err());
    assert!(RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::from("payload"), identity,),
        site(),
        vec![ExceptionStackFrame::Local { site: site() }],
        ErrorCorrelation {
            trace_id: " ".to_string(),
            error_id: "error".to_string(),
        },
    )
    .is_err());
}

#[test]
fn imported_error_rejects_an_unidentified_local_value() {
    let opaque = OpaqueServiceError::decode(exact_public_bytes()).unwrap();
    assert!(RequestException::imported(
        opaque,
        Some(RuntimeValue::from("payload").into()),
        site(),
        Vec::new(),
    )
    .is_err());
}

#[test]
fn opaque_service_error_decode_remains_strict() {
    let malformed = br#"{
          "kind":"internalError",
          "payload":{
            "message":"sanitized",
            "traceId":"trace",
            "errorId":"error",
            "private":true
          }
        }"#
    .to_vec();

    assert!(OpaqueServiceError::decode(malformed).is_err());
}
