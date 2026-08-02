//! W-actor consumer gate: the frozen 20-frame actor-wire corpus is consumed
//! through the canonical transport codecs byte-exact, and decoded frames
//! drive the real W-actor owners (control broker / invocation relay /
//! activation broker).

mod actor_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_router::actor::{
    ActivateInitialControlRequest, ActivationAckOutcome, ActivationControlPort,
    ActorActivationBrokerOptions, ActorActivationRequestBroker, ActorGetOrCreateRequest,
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, ActorLogicalKey,
    ActorOwnerControlBroker, ActorOwnerFence, ActorOwnerRouteAuthority, ControlAckOutcome,
    ControlBrokerOptions, GetOrCreateOutcome, OwnerControlRequest, OwnerSettleKind,
};
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, encode_actor_method_frame, ActorMethodFrame,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    decode_actor_owner_invoke_frame, encode_actor_owner_control_ack_frame,
    encode_actor_owner_control_frame, encode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, ActorFindRequestFrameHeader,
    ActorFindResponseFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorGetOrCreateResponseFrameHeader, ActorRemoveRequestFrameHeader,
    ActorRemoveResponseFrameHeader, ActorReplaceRequestFrameHeader,
    ActorReplaceResponseFrameHeader, ActorSpawnRuntimeErrorFrameHeader, FrameDirection,
    PayloadPresenceRule, RuntimeFrameFamily,
};

use actor_support::{actor_wire_dir, hex_bytes};

const REQUIRED_FRAMES: [&str; 20] = [
    "actor.getOrCreate.request",
    "actor.getOrCreate.response",
    "actor.getOrCreate.error",
    "actor.replace.request",
    "actor.replace.response",
    "actor.replace.error",
    "actor.find.request",
    "actor.find.response",
    "actor.find.error",
    "actor.remove.request",
    "actor.remove.response",
    "actor.remove.error",
    "actor.method.invoke",
    "actor.method.return",
    "actor.method.error",
    "actor.method.cancel",
    "actor.owner.invoke",
    "actor.owner.control.activateInitial",
    "actor.owner.control.ack",
    "actor.owner.failure",
];

const REQUIRED_SCENARIOS: [&str; 22] = [
    "claim-reserve-commit-single-owner",
    "claim-reserve-conflict-while-owner-held",
    "claim-abort-no-effect",
    "claim-commit-twice-rejected",
    "claim-reservation-not-owner",
    "lease-expire-releases-fence",
    "get-or-create-first-joins-same-outcome",
    "get-or-create-lineage-conflict",
    "get-or-create-existing-no-reserve",
    "get-or-create-ack-timeout-aborts-token",
    "invoke-return-exact-owner",
    "invoke-error-caller-forward",
    "invoke-cancel-correlation",
    "invoke-duplicate-settle-rejected",
    "invoke-owner-disconnect-terminals-pending",
    "control-ack-exact-correlation",
    "control-ack-timeout-rejected",
    "control-late-ack-tombstone",
    "control-ack-wrong-operation-rejected",
    "lease-sweep-expire-and-idle-evict",
    "lease-eviction-ack-clears-request",
    "lease-eviction-retry-bounded",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn catalog() -> Catalog {
    let raw = std::fs::read_to_string(actor_wire_dir().join("frames.json"))
        .expect("actor-wire frames.json must be readable");
    serde_json::from_str(&raw).expect("actor-wire frames.json must decode")
}

fn frame_payload(bytes: &[u8]) -> Vec<u8> {
    let header_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let payload_len = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    bytes[14 + header_len..14 + header_len + payload_len].to_vec()
}

fn fence_from_owner_frame(
    fence: &skiff_runtime_transport::actor_owner::ActorOwnerFenceFrameHeader,
) -> ActorOwnerFence {
    ActorOwnerFence {
        epoch: fence.epoch,
        owner_runtime_id: fence.owner_runtime_id.clone(),
        owner_lease_id: fence.owner_lease_id.clone(),
        lease_expires_at: 40_000,
        actor_abi_identity: fence.actor_abi_identity.clone(),
        actor_implementation_identity: fence.actor_implementation_identity.clone(),
        declaration_owner: fence.declaration_owner.clone(),
    }
}

fn fence_from_control_frame(
    fence: &skiff_runtime_transport::actor_owner::ActorOwnerControlFenceFrameHeader,
) -> ActorOwnerFence {
    ActorOwnerFence {
        epoch: fence.epoch,
        owner_runtime_id: "runtime-b".to_string(),
        owner_lease_id: fence.owner_lease_id.clone(),
        lease_expires_at: 40_000,
        actor_abi_identity: fence.actor_abi_identity.clone(),
        actor_implementation_identity: fence.actor_implementation_identity.clone(),
        declaration_owner: fence.declaration_owner.clone(),
    }
}

fn key_from_wire(
    key: &skiff_runtime_transport::protocol::ActorKeyFrameMetadata,
) -> ActorLogicalKey {
    ActorLogicalKey {
        service_id: key.service_id.clone(),
        actor_type_identity: key.actor_type_identity.clone(),
        actor_id_type_identity: key.actor_id_type_identity.clone(),
        actor_id_encoding_version: key.actor_id_encoding_version.clone(),
        canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64.clone(),
        actor_id_hash: key.actor_id_hash.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_wire_corpus_is_frozen() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "actor-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "required actor frame {required} is missing"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        assert_eq!(
            RuntimeFrameFamily::Actor.direction(),
            FrameDirection::Either
        );
        assert_eq!(
            RuntimeFrameFamily::Actor.payload_presence(),
            PayloadPresenceRule::Optional
        );
        assert_eq!(RuntimeFrameFamily::Actor.wire_type_prefix(), "actor.");
    }

    #[test]
    fn all_frames_roundtrip_byte_exact_through_canonical_codecs() {
        let catalog = catalog();
        for (name, entry) in &catalog.frames {
            let bytes = hex_bytes(&entry.frame_hex);
            let reencoded = match entry.decode_as.as_str() {
                "ActorGetOrCreateRequest" => {
                    let (header, payload): (ActorGetOrCreateRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorGetOrCreateResponse" => {
                    let (header, payload): (ActorGetOrCreateResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorSpawnRuntimeError" => {
                    let (header, payload): (ActorSpawnRuntimeErrorFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorReplaceRequest" => {
                    let (header, payload): (ActorReplaceRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorReplaceResponse" => {
                    let (header, payload): (ActorReplaceResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorFindRequest" => {
                    let (header, payload): (ActorFindRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorFindResponse" => {
                    let (header, payload): (ActorFindResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorRemoveRequest" => {
                    let (header, payload): (ActorRemoveRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorRemoveResponse" => {
                    let (header, payload): (ActorRemoveResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorMethodInvoke" | "ActorMethodReturn" | "ActorMethodError"
                | "ActorMethodCancel" => {
                    let frame = decode_actor_method_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_method_frame(&frame).expect("re-encode")
                }
                "ActorOwnerInvoke" => {
                    let (header, payload) = decode_actor_owner_invoke_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_invoke_frame(&header, &payload).expect("re-encode")
                }
                "ActorOwnerControl" => {
                    let header = decode_actor_owner_control_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_control_frame(&header).expect("re-encode")
                }
                "ActorOwnerControlAck" => {
                    let (header, payload): (ActorOwnerControlAckFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    assert!(payload.is_empty(), "{name}: ack payload must be empty");
                    encode_actor_owner_control_ack_frame(&header).expect("re-encode")
                }
                "ActorOwnerFailure" => {
                    let header = decode_actor_owner_failure_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_failure_frame(&header).expect("re-encode")
                }
                other => panic!("{name}: unknown decodeAs {other}"),
            };
            assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
            if entry.payload_presence == "empty" {
                assert!(
                    frame_payload(&bytes).is_empty(),
                    "{name}: empty-presence frame has payload"
                );
            }
            assert_eq!(
                entry.frame_type,
                expected_frame_type(name),
                "{name}: frameType"
            );
            assert_eq!(
                entry.decode_as,
                expected_decode_as(name),
                "{name}: decodeAs"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v3",
                "{name}"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
        }
    }

    #[test]
    fn all_frozen_actor_scenario_names_are_present() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(actor_wire_dir().join("scenarios")).expect("scenarios dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            names.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "required actor scenario {required} is missing"
            );
        }
    }

    #[test]
    fn decoded_control_frames_drive_the_real_control_broker() {
        let catalog = catalog();
        let control_bytes =
            hex_bytes(&catalog.frames["actor.owner.control.activateInitial"].frame_hex);
        let control =
            decode_actor_owner_control_frame(&control_bytes).expect("activateInitial must decode");
        let broker = ActorOwnerControlBroker::new(ControlBrokerOptions::default());
        broker
            .send_control(&OwnerControlRequest {
                request_id: control.request_id.clone(),
                operation: control.operation,
                runtime_id: control.target_runtime_id.clone(),
                connection: "conn-b".to_string(),
                fence: fence_from_control_frame(&control.fence),
                route_authority: ActorOwnerRouteAuthority::from_wire(&control.route_authority),
                deadline_at: 10_000,
            })
            .expect("send_control");
        let ack_bytes = hex_bytes(&catalog.frames["actor.owner.control.ack"].frame_hex);
        let ack: ActorOwnerControlAckFrameHeader = decode_typed_binary_frame(&ack_bytes)
            .expect("ack must decode")
            .0;
        assert_eq!(ack.request_id, control.request_id);
        assert!(
            broker.pending_snapshot(&control.request_id).is_some(),
            "pending snapshot must exist before the ACK"
        );
        let outcome = broker.on_ack(
            &ack.runtime_id,
            &ack.request_id,
            ack.operation,
            "conn-b",
            ack.accepted,
        );
        assert_eq!(outcome, ControlAckOutcome::Accepted);
        let health = broker.health();
        assert_eq!(health.pending, 0);
        assert_eq!(health.accepted, 1);
        assert!(
            broker.pending_snapshot(&control.request_id).is_none(),
            "settled id must leave the pending map"
        );
    }

    #[test]
    fn decoded_invocation_frames_drive_the_real_relay() {
        let catalog = catalog();
        let invoke_bytes = hex_bytes(&catalog.frames["actor.owner.invoke"].frame_hex);
        let (owner_invoke, _payload) =
            decode_actor_owner_invoke_frame(&invoke_bytes).expect("owner.invoke must decode");
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        let fence = fence_from_owner_frame(&owner_invoke.owner_fence);
        relay
            .invoke(&ActorInvokeInput {
                invocation_id: owner_invoke.invoke.invocation_id.clone(),
                caller_connection: "conn-caller".to_string(),
                caller_runtime_id: "runtime-caller".to_string(),
                owner_fence: fence.clone(),
                owner_connection: "conn-b".to_string(),
                route_authority: ActorOwnerRouteAuthority::from_wire(&owner_invoke.route_authority),
                correlation: owner_invoke.invoke.cancellation_correlation.clone(),
                deadline: Some(owner_invoke.invoke.deadline.clone()),
                test_case_capability: owner_invoke.invoke.test_case_capability.clone(),
                now: 0,
            })
            .expect("invoke admitted");
        assert!(relay.is_active_parent(&owner_invoke.invoke.invocation_id));

        let return_bytes = hex_bytes(&catalog.frames["actor.method.return"].frame_hex);
        let return_frame =
            decode_actor_method_frame(&return_bytes).expect("method.return must decode");
        let ActorMethodFrame::Return(return_header, _payload) = return_frame else {
            panic!("expected return frame");
        };
        let settled = relay
            .on_owner_settle(
                &return_header.invocation_id,
                &fence,
                "conn-b",
                OwnerSettleKind::Return,
            )
            .expect("exact owner settle");
        assert_eq!(settled.invocation_id, "inv:1");
        let health = relay.health();
        assert_eq!(health.pending, 0);
        assert_eq!(health.settled, 1);
        assert_eq!(health.tombstones, 1);
        assert!(!relay.is_active_parent("inv:1"));
        let duplicate = relay.on_owner_settle(
            &return_header.invocation_id,
            &fence,
            "conn-b",
            OwnerSettleKind::Return,
        );
        assert!(duplicate.is_err(), "duplicate settle must be rejected");
        assert_eq!(relay.health().rejected, 1);
    }

    #[test]
    fn decoded_get_or_create_frame_drives_the_real_activation_broker() {
        #[derive(Debug, Default)]
        struct FakeControl {
            sent: Mutex<Vec<ActivateInitialControlRequest>>,
        }
        impl ActivationControlPort for FakeControl {
            fn send_activate_initial(
                &self,
                request: &ActivateInitialControlRequest,
            ) -> Result<(), String> {
                self.sent
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(request.clone());
                Ok(())
            }
        }
        let catalog = catalog();
        let bytes = hex_bytes(&catalog.frames["actor.getOrCreate.request"].frame_hex);
        let (header, payload) =
            decode_typed_binary_frame::<ActorGetOrCreateRequestFrameHeader>(&bytes)
                .expect("getOrCreate.request must decode");
        let control = Arc::new(FakeControl::default());
        let broker = ActorActivationRequestBroker::new(
            Arc::new(skiff_router::actor::ActorOwnershipRegistry::new()),
            Arc::clone(&control) as Arc<dyn ActivationControlPort>,
            ActorActivationBrokerOptions::default(),
        );
        let outcome = broker.get_or_create(&ActorGetOrCreateRequest {
            rpc_id: header.rpc_id.clone(),
            actor_key: key_from_wire(&header.actor_key),
            actor_abi_identity: ActorAbiIdentity::new(header.actor_abi_identity.clone()),
            actor_implementation_identity: ActorImplementationIdentity::new(
                header.actor_implementation_identity.clone(),
            ),
            declaration_owner: header.declaration_owner.clone(),
            bootstrap_bytes: payload.clone(),
            owner_runtime_id: header.runtime_id.clone(),
            owner_connection: "conn-runtime-a".to_string(),
            route_authority: ActorOwnerRouteAuthority {
                assembly_identity: header.activation_identity.assembly_identity.clone(),
                assembly_generation: header.activation_identity.generation,
            },
            deadline: header.deadline.clone(),
            test_case_capability: header.test_case_capability.clone(),
            test_case_parent_request_id: header.test_case_parent_request_id.clone(),
            now: 0,
        });
        let GetOrCreateOutcome::StartedActivation { request_id } = outcome else {
            panic!("expected started activation");
        };
        let sent = control
            .sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].bootstrap_bytes, payload);
        assert_eq!(sent[0].actor_key.service_id, "example.com/docs");
        let ack = broker.on_activation_ack(&request_id, "runtime-a", "conn-runtime-a", true, 2000);
        assert!(matches!(
            ack,
            ActivationAckOutcome::Committed { epoch: 1, .. }
        ));
        assert_eq!(
            broker.outcome_for(&header.rpc_id).as_deref(),
            Some("resolved:1")
        );
        let health = broker.health();
        assert_eq!(health.pending_claims, 0);
        assert_eq!(health.tombstones, 1);
    }

    fn expected_decode_as(name: &str) -> &'static str {
        match name {
            "actor.getOrCreate.request" => "ActorGetOrCreateRequest",
            "actor.getOrCreate.response" => "ActorGetOrCreateResponse",
            "actor.getOrCreate.error" => "ActorSpawnRuntimeError",
            "actor.replace.request" => "ActorReplaceRequest",
            "actor.replace.response" => "ActorReplaceResponse",
            "actor.replace.error" => "ActorSpawnRuntimeError",
            "actor.find.request" => "ActorFindRequest",
            "actor.find.response" => "ActorFindResponse",
            "actor.find.error" => "ActorSpawnRuntimeError",
            "actor.remove.request" => "ActorRemoveRequest",
            "actor.remove.response" => "ActorRemoveResponse",
            "actor.remove.error" => "ActorSpawnRuntimeError",
            "actor.method.invoke" => "ActorMethodInvoke",
            "actor.method.return" => "ActorMethodReturn",
            "actor.method.error" => "ActorMethodError",
            "actor.method.cancel" => "ActorMethodCancel",
            "actor.owner.invoke" => "ActorOwnerInvoke",
            "actor.owner.control.activateInitial" => "ActorOwnerControl",
            "actor.owner.control.ack" => "ActorOwnerControlAck",
            "actor.owner.failure" => "ActorOwnerFailure",
            _ => panic!("unexpected frame {name}"),
        }
    }

    fn expected_frame_type(name: &str) -> &str {
        if name == "actor.owner.control.activateInitial" {
            "actor.owner.control"
        } else {
            name
        }
    }
}
