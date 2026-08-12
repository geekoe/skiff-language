//! W-model codec unit tests for the target handshake and bootstrap wire.
//!
//! These exercise the frame-level codecs added by W-model-registration /
//! W-model-bootstrap-wire. Golden bytes themselves are frozen by the shared
//! corpus tests (`runtime/transport/tests/w_model_*_corpus.rs` and the
//! contracts-session/contracts-bootstrap tests).

use serde_json::{json, Value};
use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
    PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
};

use crate::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame, decode_runtime_capabilities_frame,
    decode_runtime_health_frame, decode_runtime_registered_frame, encode_router_bootstrap_frame,
    encode_runtime_capabilities_frame, encode_runtime_health_frame,
    encode_runtime_registered_frame, RouterBootstrapFrameHeader, RouterBootstrapHttpFrameHeader,
    RouterBootstrapServiceDbFrameHeader, RouterBootstrapSource, RuntimeBootstrapProvider,
    RuntimeCapabilitiesFrameHeader, RuntimeCapabilitiesFrameHeaderMetadata,
    RuntimeDispatchModeCapability, RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
    RuntimeRegisteredFrameHeader, StatelessRuntimeBootstrapProvider, BINARY_FRAME_VERSION,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

fn bootstrap_header() -> RouterBootstrapFrameHeader {
    RouterBootstrapFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "router.bootstrap".to_string(),
        artifacts_path: "/opt/skiff/artifacts".to_string(),
        service_db: RouterBootstrapServiceDbFrameHeader {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        http: RouterBootstrapHttpFrameHeader {
            max_response_bytes: 8_388_608,
        },
        activation: crate::protocol::RouterBootstrapActivationFrameHeader {
            profile: "prod".to_string(),
        },
    }
}

fn capabilities_header() -> RuntimeCapabilitiesFrameHeader {
    RuntimeCapabilitiesFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.capabilities".to_string(),
        runtime_id: "runtime-a".to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            platform_error_projection_registry: current_platform_error_projection_registry_ref()
                .clone(),
            dispatch_modes: vec![
                RuntimeDispatchModeCapability::Unary,
                RuntimeDispatchModeCapability::ServerStream,
            ],
            package_test_dispatch: true,
            request_cancel: true,
            runtime_program: false,
            artifact_root: None,
            lazy_load: false,
            loaded_build_ids: Vec::new(),
        },
    }
}

fn registered_header() -> RuntimeRegisteredFrameHeader {
    RuntimeRegisteredFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.registered".to_string(),
        runtime_id: "runtime-a".to_string(),
    }
}

fn health_header() -> RuntimeHealthFrameHeader {
    RuntimeHealthFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.health".to_string(),
        runtime_id: "runtime-a".to_string(),
        observed_at: "2026-08-02T00:00:00Z".to_string(),
        counters: RuntimeHealthCountersFrameHeader {
            outbound_requests_pending: 0,
            outbound_stream_leases_active: 0,
            stream_runtime_streams_active: 0,
            flag_backed_cancel_waiters_active: 0,
            task_requests_active: 0,
        },
    }
}

#[test]
fn router_bootstrap_frame_roundtrips_and_enforces_empty_payload() {
    let header = bootstrap_header();
    let frame = encode_router_bootstrap_frame(&header).expect("bootstrap frame must encode");
    let decoded = decode_router_bootstrap_frame(&frame).expect("bootstrap frame must decode");
    assert_eq!(decoded, header);

    let with_payload =
        crate::protocol::encode_binary_frame(&header, b"non-empty").expect("raw frame must encode");
    let error = decode_router_bootstrap_frame(&with_payload)
        .expect_err("non-empty bootstrap payload must be rejected");
    assert!(
        error.to_string().contains("payload must be empty"),
        "unexpected error: {error}"
    );
}

#[test]
fn capabilities_frame_roundtrips_and_enforces_empty_payload() {
    let header = capabilities_header();
    let frame = encode_runtime_capabilities_frame(&header).expect("capabilities must encode");
    assert_eq!(BINARY_FRAME_VERSION, 1);
    assert_eq!(frame[4], BINARY_FRAME_VERSION);
    let decoded = decode_runtime_capabilities_frame(&frame).expect("capabilities must decode");
    assert_eq!(decoded, header);

    let raw = decode_binary_frame(&frame).expect("capabilities binary frame");
    let capabilities = raw.header["capabilities"]
        .as_object()
        .expect("capabilities metadata must be an object");
    assert_eq!(
        capabilities
            .get("platformErrorProjectionRegistry")
            .expect("exact registry field name"),
        &serde_json::to_value(current_platform_error_projection_registry_ref())
            .expect("current registry serializes")
    );
    assert!(!capabilities.contains_key("platform_error_projection_registry"));

    let with_payload =
        crate::protocol::encode_binary_frame(&header, b"x").expect("raw frame must encode");
    assert!(decode_runtime_capabilities_frame(&with_payload).is_err());
}

#[test]
fn capabilities_registry_accepts_valid_historical_fingerprint_without_current_pin() {
    let historical_fingerprint = format!("sha256:{}", "0".repeat(64));
    let mut value = serde_json::to_value(capabilities_header()).expect("header serializes");
    value["capabilities"]["platformErrorProjectionRegistry"]["fingerprint"] =
        Value::String(historical_fingerprint.clone());

    let frame = crate::protocol::encode_binary_frame(&value, &[]).expect("raw frame must encode");
    let decoded = decode_runtime_capabilities_frame(&frame)
        .expect("general-shape historical registry must decode");
    assert_eq!(
        decoded
            .capabilities
            .platform_error_projection_registry
            .fingerprint(),
        historical_fingerprint
    );

    let reencoded =
        encode_runtime_capabilities_frame(&decoded).expect("historical descriptor must re-encode");
    let roundtrip = decode_runtime_capabilities_frame(&reencoded)
        .expect("historical descriptor must roundtrip");
    assert_eq!(roundtrip, decoded);
}

#[test]
fn capabilities_registry_is_required_and_strictly_validated() {
    let valid_fingerprint = format!("sha256:{}", "0".repeat(64));
    let invalid_descriptors = [
        (
            "invalid registry id",
            Some(json!({
                "registryId": "other",
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": valid_fingerprint.clone(),
            })),
        ),
        (
            "invalid registry version",
            Some(json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION + 1,
                "fingerprint": valid_fingerprint.clone(),
            })),
        ),
        (
            "uppercase fingerprint",
            Some(json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": format!("sha256:{}", "A".repeat(64)),
            })),
        ),
        (
            "malformed fingerprint",
            Some(json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": "sha256:not-a-digest",
            })),
        ),
        (
            "unknown descriptor field",
            Some(json!({
                "registryId": PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
                "registryVersion": PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
                "fingerprint": valid_fingerprint.clone(),
                "unexpected": true,
            })),
        ),
        ("missing descriptor", None),
    ];

    for (name, descriptor) in invalid_descriptors {
        let mut value = serde_json::to_value(capabilities_header()).expect("header serializes");
        let capabilities = value["capabilities"]
            .as_object_mut()
            .expect("capabilities metadata must be an object");
        match descriptor {
            Some(descriptor) => {
                capabilities.insert("platformErrorProjectionRegistry".to_string(), descriptor);
            }
            None => {
                capabilities.remove("platformErrorProjectionRegistry");
            }
        }
        let frame =
            crate::protocol::encode_binary_frame(&value, &[]).expect("raw frame must encode");
        assert!(
            decode_runtime_capabilities_frame(&frame).is_err(),
            "{name} must be rejected"
        );
    }
}

#[test]
fn capabilities_rejects_runtime_frame_v4_without_a_dual_reader() {
    let mut value = serde_json::to_value(capabilities_header()).expect("header serializes");
    value["schemaVersion"] = Value::String("skiff-runtime-frame-v4".to_string());
    let frame = crate::protocol::encode_binary_frame(&value, &[]).expect("raw frame must encode");
    let error = decode_runtime_capabilities_frame(&frame)
        .expect_err("runtime-frame-v4 capabilities must be rejected");
    assert!(error.to_string().contains("skiff-runtime-frame-v5"));
}

#[test]
fn registered_frame_roundtrips_and_enforces_empty_payload() {
    let header = registered_header();
    let frame = encode_runtime_registered_frame(&header).expect("registered must encode");
    let decoded = decode_runtime_registered_frame(&frame).expect("registered must decode");
    assert_eq!(decoded, header);

    let with_payload =
        crate::protocol::encode_binary_frame(&header, b"x").expect("raw frame must encode");
    assert!(decode_runtime_registered_frame(&with_payload).is_err());
}

#[test]
fn health_frame_roundtrips_and_enforces_empty_payload() {
    let header = health_header();
    let frame = encode_runtime_health_frame(&header).expect("health must encode");
    let decoded = decode_runtime_health_frame(&frame).expect("health must decode");
    assert_eq!(decoded, header);

    let with_payload =
        crate::protocol::encode_binary_frame(&header, b"x").expect("raw frame must encode");
    assert!(decode_runtime_health_frame(&with_payload).is_err());
}

#[test]
fn wrong_type_and_schema_version_are_rejected() {
    let mut capabilities = capabilities_header();
    capabilities.envelope_type = "runtime.registered".to_string();
    let frame =
        crate::protocol::encode_binary_frame(&capabilities, &[]).expect("raw frame must encode");
    let error =
        decode_runtime_capabilities_frame(&frame).expect_err("wrong frame type must be rejected");
    assert!(error
        .to_string()
        .contains("type must be runtime.capabilities"));

    let mut registered = registered_header();
    registered.schema_version = "skiff-runtime-frame-v2".to_string();
    let frame =
        crate::protocol::encode_binary_frame(&registered, &[]).expect("raw frame must encode");
    let error =
        decode_runtime_registered_frame(&frame).expect_err("wrong schemaVersion must be rejected");
    assert!(error.to_string().contains("schemaVersion must be"));
}

#[test]
fn bootstrap_profile_is_strictly_validated() {
    let source = RouterBootstrapSource {
        artifacts_path: "/opt/skiff/artifacts".to_string(),
        service_db: RouterBootstrapServiceDbFrameHeader {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        http: RouterBootstrapHttpFrameHeader {
            max_response_bytes: 8_388_608,
        },
        profile: "prod".to_string(),
    };
    assert!(StatelessRuntimeBootstrapProvider
        .bootstrap_frame(&source)
        .is_ok());

    let mut invalid = source.clone();
    invalid.profile = "prod env".to_string();
    assert!(StatelessRuntimeBootstrapProvider
        .bootstrap_frame(&invalid)
        .is_err());
}

#[test]
fn stateless_provider_builds_bootstrap_header_from_captured_source() {
    let source = RouterBootstrapSource {
        artifacts_path: "/opt/skiff/artifacts".to_string(),
        service_db: RouterBootstrapServiceDbFrameHeader {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        http: RouterBootstrapHttpFrameHeader {
            max_response_bytes: 8_388_608,
        },
        profile: "prod".to_string(),
    };

    let provider = StatelessRuntimeBootstrapProvider;
    let header = provider
        .bootstrap_frame(&source)
        .expect("stateless provider must construct a valid bootstrap header");
    let frame = encode_router_bootstrap_frame(&header).expect("bootstrap frame must encode");
    let decoded = decode_router_bootstrap_frame(&frame).expect("bootstrap frame must decode");
    assert_eq!(decoded, header);
    assert_eq!(decoded.activation.profile, "prod");
}

#[test]
fn invalid_router_bootstrap_source_is_rejected_by_provider() {
    let source = RouterBootstrapSource {
        artifacts_path: "relative/artifacts".to_string(),
        service_db: RouterBootstrapServiceDbFrameHeader {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        http: RouterBootstrapHttpFrameHeader {
            max_response_bytes: 8_388_608,
        },
        profile: "prod".to_string(),
    };
    let error = StatelessRuntimeBootstrapProvider
        .bootstrap_frame(&source)
        .expect_err("relative artifactsPath must be rejected");
    assert!(
        error.to_string().contains("absolute normalized path"),
        "unexpected error: {error}"
    );
}
