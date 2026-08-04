//! W-model codec unit tests for the target handshake and bootstrap wire.
//!
//! These exercise the frame-level codecs added by W-model-registration /
//! W-model-bootstrap-wire. Golden bytes themselves are frozen by the shared
//! corpus tests (`runtime/transport/tests/w_model_*_corpus.rs` and the
//! contracts-session/contracts-bootstrap tests).

use skiff_artifact_model::{AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotRef};

use crate::protocol::{
    decode_router_bootstrap_frame, decode_runtime_capabilities_frame, decode_runtime_health_frame,
    decode_runtime_registered_frame, encode_router_bootstrap_frame,
    encode_runtime_capabilities_frame, encode_runtime_health_frame,
    encode_runtime_registered_frame, CapturedBootstrapEpoch, RouterBootstrapFrameHeader,
    RouterBootstrapHttpFrameHeader, RouterBootstrapServiceDbFrameHeader, RouterBootstrapSource,
    RuntimeBootstrapProvider, RuntimeCapabilitiesFrameHeader,
    RuntimeCapabilitiesFrameHeaderMetadata, RuntimeDispatchModeCapability,
    RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader, RuntimeRegisteredFrameHeader,
    StatelessRuntimeBootstrapProvider, RUNTIME_FRAME_SCHEMA_VERSION,
};

fn assembly_ref(byte: char) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            byte.to_string().repeat(64)
        )),
    }
}

fn config_snapshot_ref(byte: char) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            byte.to_string().repeat(32)
        ))
        .unwrap(),
    }
}

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
            generation: 42,
            assembly: assembly_ref('a'),
            config_snapshot: config_snapshot_ref('b'),
        },
    }
}

fn capabilities_header() -> RuntimeCapabilitiesFrameHeader {
    RuntimeCapabilitiesFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.capabilities".to_string(),
        runtime_id: "runtime-a".to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            dispatch_modes: vec![
                RuntimeDispatchModeCapability::Unary,
                RuntimeDispatchModeCapability::ServerStream,
            ],
            package_test_dispatch: true,
            request_cancel: true,
            ..RuntimeCapabilitiesFrameHeaderMetadata::default()
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
    let decoded = decode_runtime_capabilities_frame(&frame).expect("capabilities must decode");
    assert_eq!(decoded, header);

    let with_payload =
        crate::protocol::encode_binary_frame(&header, b"x").expect("raw frame must encode");
    assert!(decode_runtime_capabilities_frame(&with_payload).is_err());
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
fn captured_bootstrap_epoch_strictly_validates_wire_fields() {
    let assembly = format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64));
    let snapshot = format!("skiff-runtime-config-snapshot-v1:{}", "b".repeat(32));
    let epoch = CapturedBootstrapEpoch::new("prod", 42, assembly.clone(), snapshot.clone())
        .expect("valid captured epoch must construct");
    assert_eq!(epoch.profile, "prod");
    assert_eq!(epoch.generation, 42);
    assert_eq!(epoch.assembly, assembly_ref('a'));
    assert_eq!(epoch.config_snapshot, config_snapshot_ref('b'));

    assert!(
        CapturedBootstrapEpoch::new("prod env", 42, assembly.clone(), snapshot.clone()).is_err()
    );
    assert!(CapturedBootstrapEpoch::new(
        "prod",
        9_007_199_254_740_992,
        assembly.clone(),
        snapshot.clone()
    )
    .is_err());
    assert!(
        CapturedBootstrapEpoch::new("prod", 42, "broken".to_string(), snapshot.clone()).is_err()
    );
    assert!(CapturedBootstrapEpoch::new("prod", 42, assembly, "broken".to_string()).is_err());
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
        activation: CapturedBootstrapEpoch::new(
            "prod",
            42,
            format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
            format!("skiff-runtime-config-snapshot-v1:{}", "b".repeat(32)),
        )
        .expect("valid captured epoch"),
    };

    let provider = StatelessRuntimeBootstrapProvider;
    let header = provider
        .bootstrap_frame(&source)
        .expect("stateless provider must construct a valid bootstrap header");
    let frame = encode_router_bootstrap_frame(&header).expect("bootstrap frame must encode");
    let decoded = decode_router_bootstrap_frame(&frame).expect("bootstrap frame must decode");
    assert_eq!(decoded, header);
    assert_eq!(decoded.activation.generation, 42);
    assert_eq!(decoded.activation.profile, "prod");
    assert_eq!(decoded.activation.assembly, assembly_ref('a'));
    assert_eq!(decoded.activation.config_snapshot, config_snapshot_ref('b'));
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
        activation: CapturedBootstrapEpoch::new(
            "prod",
            42,
            format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
            format!("skiff-runtime-config-snapshot-v1:{}", "b".repeat(32)),
        )
        .expect("valid captured epoch"),
    };
    let error = StatelessRuntimeBootstrapProvider
        .bootstrap_frame(&source)
        .expect_err("relative artifactsPath must be rejected");
    assert!(
        error.to_string().contains("absolute normalized path"),
        "unexpected error: {error}"
    );
}
