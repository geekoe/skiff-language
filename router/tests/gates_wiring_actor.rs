//! E-gates actor inbound sink tests: decoded actor frames drive the W-actor
//! owners and the sink writes canonical responses/forwards through the
//! exact-session writer (fake runtime seams; owner semantics themselves are
//! covered by the W-actor corpus tests).

use std::sync::{Arc, Mutex};

use base64::Engine;
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::actor::ActorLogicalKey;
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::InboundFrameSink;
use skiff_router::supervisor::actor::assemble_actor_components;
use skiff_router::supervisor::actor_sink::ActorFrameSink;
use skiff_router::supervisor::session_ports::SessionHandle;
use skiff_router::supervisor::ws::WsSessionWriter;
use skiff_router::ws::types::SystemClock;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_transport::actor_method::{
    encode_actor_method_frame, ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
    ActorMethodDeadlineFrameHeader, ActorMethodFrame, ActorMethodInvokeFrameHeader,
    ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::actor_owner::{
    encode_actor_owner_control_frame, ActorOwnerControlFenceFrameHeader,
    ActorOwnerControlFrameHeader, ActorOwnerControlOperation, ActorOwnerRouteAuthorityFrameHeader,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, ActorFindRequestFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorRemoveRequestFrameHeader, ActorReplaceRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};

fn fixture_epoch() -> Arc<RoutingEpoch> {
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    let snapshot = RuntimeConfigSnapshot::new(
        "prod",
        skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("snapshot id"),
        },
        Vec::new(),
    )
    .expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = ActorRoutingCatalog::from_projection(Arc::new(projection));
    Arc::new(
        RoutingEpoch::new(
            "prod",
            1,
            Arc::new(assembly),
            Arc::new(snapshot),
            Arc::new(catalog),
        )
        .expect("epoch"),
    )
}

#[derive(Debug, Default)]
struct FakeWsSessionWriter {
    frames: Arc<Mutex<Vec<WrittenFrame>>>,
}

type WrittenFrame = (RuntimeSessionEpoch, Vec<u8>);

impl WsSessionWriter for FakeWsSessionWriter {
    fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((runtime.clone(), bytes));
        Ok(())
    }
}

fn sink() -> (
    ActorFrameSink,
    Arc<FakeWsSessionWriter>,
    Arc<ActiveRoutingEpochStore>,
) {
    let epoch = fixture_epoch();
    let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
    epoch_store.publish(Arc::clone(&epoch));
    let session = SessionHandle::new();
    let components = assemble_actor_components(
        Arc::clone(&epoch),
        Arc::clone(&epoch_store),
        session.clone(),
    )
    .expect("actor components");
    let writer = Arc::new(FakeWsSessionWriter::default());
    let sink = ActorFrameSink::new(
        components,
        session,
        Arc::clone(&epoch_store),
        writer.clone(),
        Arc::new(SystemClock),
        Arc::new(skiff_router::task::NoopActorAttemptTerminalSink),
    );
    (sink, writer, epoch_store)
}

fn runtime() -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    }
}

fn actor_key() -> ActorLogicalKey {
    ActorLogicalKey {
        service_id: "example.com/actors".to_string(),
        actor_type_identity: format!("skiff-actor-type-v1:sha256:{}", digest("type")),
        actor_id_type_identity: format!("skiff-actor-id-type-v1:sha256:{}", digest("id-type")),
        actor_id_encoding_version: "skiff-actor-id-encoding-v1:sha256:abcd".to_string(),
        canonical_actor_id_key_bytes_base64: "YWxpY2U=".to_string(),
        actor_id_hash: format!("skiff-actor-id-hash-v1:sha256:{}", digest("hash")),
    }
}

fn key_metadata() -> skiff_runtime_transport::protocol::ActorKeyFrameMetadata {
    let key = actor_key();
    skiff_runtime_transport::protocol::ActorKeyFrameMetadata {
        service_id: key.service_id,
        actor_type_identity: key.actor_type_identity,
        actor_id_type_identity: key.actor_id_type_identity,
        actor_id_encoding_version: key.actor_id_encoding_version,
        canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64,
        actor_id_hash: Some(key.actor_id_hash),
    }
}

fn activation_identity() -> skiff_runtime_transport::protocol::ActivationIdentityFrameMetadata {
    skiff_runtime_transport::protocol::ActivationIdentityFrameMetadata {
        assembly_identity: assembly_identity_str(),
        generation: 1,
        runtime_replica_id: "runtime-a".to_string(),
        deployment_revision: "1".to_string(),
    }
}

fn digest(seed: &str) -> String {
    let mut digest = String::new();
    for byte in seed.bytes().chain(std::iter::repeat(0)) {
        digest.push_str(&format!("{byte:02x}"));
        if digest.len() >= 64 {
            break;
        }
    }
    while digest.len() < 64 {
        digest.push('0');
    }
    digest
}

fn assembly_identity_str() -> String {
    format!("skiff-runtime-assembly-v3:sha256:{}", digest("assembly"))
}

fn abi_identity() -> skiff_artifact_model::ActorAbiIdentity {
    skiff_artifact_model::ActorAbiIdentity::new(format!(
        "skiff-actor-abi-v1:sha256:{}",
        digest("abi")
    ))
}

fn implementation_identity() -> skiff_artifact_model::ActorImplementationIdentity {
    skiff_artifact_model::ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        digest("impl")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_or_create_without_owner_fails_closed_with_error_frame() {
        let (sink, writer, _) = sink();
        let header = ActorGetOrCreateRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.getOrCreate.request".to_string(),
            rpc_id: "rpc-1".to_string(),
            runtime_id: "runtime-a".to_string(),
            activation_identity: activation_identity(),
            actor_key: key_metadata(),
            actor_abi_identity: format!("skiff-actor-abi-v1:sha256:{}", digest("abi")),
            actor_implementation_identity: format!(
                "skiff-actor-implementation-v1:sha256:{}",
                digest("impl")
            ),
            bootstrap_encoding_version: "skiff-actor-bootstrap-v1".to_string(),
            declaration_owner: skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
                unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
                file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                actor_symbol: "main".to_string(),
            },
            deadline: None,
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let bytes = encode_binary_frame(&header, b"bootstrap").expect("encode getOrCreate");
        let result = sink.handle(&runtime(), &bytes);
        // The production activation control port is not wired to a live session
        // in this unit context, so the broker fails closed and the sink writes an
        // error frame (fail closed rather than a session terminal).
        assert!(result.is_ok());
        let frames = writer.frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 1);
        let decoded = skiff_runtime_transport::protocol::decode_binary_frame(&frames[0].1)
            .expect("error frame decodes");
        let frame_type = decoded
            .header
            .get("type")
            .and_then(|value| value.as_str())
            .expect("frame type");
        assert_eq!(frame_type, "actor.getOrCreate.error");
    }

    #[tokio::test]
    async fn find_remove_and_replace_respond_with_canonical_frames() {
        let (sink, writer, _) = sink();
        let session = runtime();

        let find = ActorFindRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.request".to_string(),
            rpc_id: "rpc-find".to_string(),
            runtime_id: "runtime-a".to_string(),
            activation_identity: activation_identity(),
            actor_key: key_metadata(),
        };
        sink.handle(
            &session,
            &encode_binary_frame(&find, &[]).expect("encode find"),
        )
        .expect("find handled");

        let remove = ActorRemoveRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.remove.request".to_string(),
            rpc_id: "rpc-remove".to_string(),
            runtime_id: "runtime-a".to_string(),
            activation_identity: activation_identity(),
            actor_key: key_metadata(),
        };
        sink.handle(
            &session,
            &encode_binary_frame(&remove, &[]).expect("encode remove"),
        )
        .expect("remove handled");

        let replace = ActorReplaceRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.replace.request".to_string(),
            rpc_id: "rpc-replace".to_string(),
            runtime_id: "runtime-a".to_string(),
            activation_identity: activation_identity(),
            actor_key: key_metadata(),
            actor_abi_identity: format!("skiff-actor-abi-v1:sha256:{}", digest("abi")),
            actor_implementation_identity: format!(
                "skiff-actor-implementation-v1:sha256:{}",
                digest("impl")
            ),
            bootstrap_encoding_version: "skiff-actor-bootstrap-v1".to_string(),
            declaration_owner: skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
                unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
                file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                actor_symbol: "main".to_string(),
            },
            deadline: None,
        };
        sink.handle(
            &session,
            &encode_binary_frame(&replace, &[]).expect("encode replace"),
        )
        .expect("replace handled");

        let frames = writer.frames.lock().unwrap().clone();
        let types = frames
            .iter()
            .map(|(_, bytes)| {
                skiff_runtime_transport::protocol::decode_binary_frame(bytes)
                    .expect("frame decodes")
                    .header
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            vec![
                "actor.find.response",
                "actor.remove.response",
                "actor.replace.error",
            ]
        );
    }

    #[tokio::test]
    async fn inbound_owner_control_frame_is_a_direction_violation() {
        let (sink, writer, _) = sink();
        let header = ActorOwnerControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.owner.control".to_string(),
            target_runtime_id: "runtime-a".to_string(),
            request_id: "control-1".to_string(),
            operation: ActorOwnerControlOperation::ActivateInitial,
            fence: ActorOwnerControlFenceFrameHeader {
                service_id: "example.com/actors".to_string(),
                actor_type_identity: "t".to_string(),
                actor_id_type_identity: "i".to_string(),
                actor_id_encoding_version: "v".to_string(),
                canonical_actor_id_key_bytes_base64: "YWxpY2U=".to_string(),
                actor_id_hash: "h".to_string(),
                epoch: 1,
                actor_abi_identity: abi_identity(),
                actor_implementation_identity: implementation_identity(),
                declaration_owner: skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
                    unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
                    file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                    actor_symbol: "main".to_string(),
                },
                owner_lease_id: "lease-1".to_string(),
                eviction_request_id: None,
            },
            route_authority: ActorOwnerRouteAuthorityFrameHeader {
                assembly_identity: assembly_identity_str(),
                assembly_generation: 1,
            },
            transition: None,
            bootstrap: Some(skiff_runtime_transport::actor_owner::ActorActivationBootstrapFrameHeader {
                encoding_version: skiff_runtime_transport::actor_owner::ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                payload_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"bootstrap"),
            }),
            deadline: Some(skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader {
                timeout_ms: 1000,
                expires_at: "2026-08-03T00:00:00Z".to_string(),
            }),
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let bytes = encode_actor_owner_control_frame(&header).expect("encode control");
        let result = sink.handle(&runtime(), &bytes);
        assert!(
            result.is_err(),
            "inbound actor.owner.control must be a direction violation"
        );
        assert!(writer.frames.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_actor_frame_type_is_malformed() {
        let (sink, writer, _) = sink();
        let bytes = skiff_runtime_transport::protocol::encode_binary_frame(
            &serde_json::json!({
                "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
                "type": "actor.unknown.frame",
                "requestId": "x",
            }),
            &[],
        )
        .expect("encode unknown frame");
        let result = sink.handle(&runtime(), &bytes);
        assert!(result.is_err());
        assert!(writer.frames.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn actor_method_invoke_unknown_projection_method_fails_closed_without_error_frame() {
        // E-actor-parity: actor method invocation admission reads the A0
        // projection catalog (A2 hard cut / C-actor §3.1). A miss must fail
        // closed exactly like the TS dispatcher's UnknownMethod rejection:
        // no synthetic error frame, no owner forward, no session terminal.
        let (sink, writer, _) = sink();
        let key = actor_key();
        let invoke = ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.invoke".to_string(),
            invocation_id: "invoke-unknown".to_string(),
            actor_ref: ActorLogicalRefFrameHeader {
                service_id: key.service_id,
                actor_type_identity: key.actor_type_identity,
                actor_id_type_identity: key.actor_id_type_identity,
                actor_id_encoding_version: key.actor_id_encoding_version,
                canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64,
                actor_id_hash: format!("sha256:{}", digest("hash")),
                epoch: 1,
            },
            declaration_owner: ActorDeclarationOwnerFrameHeader {
                unit: ActorOwnerUnitFrameHeader::Service,
                file: ActorOwnerFileFrameHeader::LoadedFileIndex(0),
                actor_symbol: "main".to_string(),
            },
            actor_abi_identity: skiff_artifact_model::ActorAbiIdentity::new(format!(
                "skiff-actor-abi-v1:sha256:{}",
                digest("abi")
            )),
            actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity::new(
                format!("skiff-actor-implementation-v1:sha256:{}", digest("impl")),
            ),
            method_identity: skiff_artifact_model::ActorMethodIdentity::new(format!(
                "skiff-actor-method-v1:sha256:{}",
                digest("method")
            )),
            arguments_encoding_version: "skiff-actor-arguments-v1".to_string(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: 30_000,
                expires_at: "2099-01-01T00:00:00.000Z".to_string(),
            },
            cancellation_correlation: "invoke-unknown:cancel".to_string(),
            trace_id: None,
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let bytes = encode_actor_method_frame(&ActorMethodFrame::Invoke(invoke, Vec::new()))
            .expect("encode invoke");
        let result = sink.handle(&runtime(), &bytes);
        assert!(result.is_ok());
        assert!(
            writer.frames.lock().unwrap().is_empty(),
            "unknown projection method must not produce a synthetic error or forward"
        );
    }
}
