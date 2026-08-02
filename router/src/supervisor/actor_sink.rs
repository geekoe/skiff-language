//! Production actor inbound sink (plan §5.5, C-model-actor §3): decodes
//! Runtime→Router actor frames and drives the six W-actor owners through
//! their public APIs, then encodes Router→Runtime responses/forwards to the
//! exact session writer. No actor lane internals are modified.
//!
//! Known deferred edges (recorded in the leaf; E-actor-rust gate refines):
//! `actor.replace.request` fails closed with `ActorReplaceUnavailable`;
//! duplicate/saturated method admission drops without a synthetic error
//! frame (TS parity: those reasons return no `errorFrame`); spawn-family
//! frames are out of scope (M-spawn-repair shared-model node).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, encode_actor_method_frame, ActorMethodFrame,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader, ActorOwnerFenceFrameHeader,
    ActorOwnerInvokeFrameHeader, ActorOwnerRouteAuthorityFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, ActorFindRequestFrameHeader,
    ActorFindResponseFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorGetOrCreateResponseFrameHeader, ActorRemoveRequestFrameHeader,
    ActorRemoveResponseFrameHeader, ActorReplaceRequestFrameHeader,
    ActorSpawnRuntimeErrorFrameHeader, RuntimeFrameFamily, RUNTIME_FRAME_SCHEMA_VERSION,
};

use crate::actor::{
    ActivationAckOutcome, ActorGetOrCreateRequest, ActorInvokeInput, ActorLogicalKey,
    ActorOwnerFence, ActorOwnerRouteAuthority, GetOrCreateOutcome, InvocationError,
    OwnerReleaseReason, OwnerSettleKind,
};
use crate::bootstrap::ActiveRoutingEpochStore;
use crate::session::demux::InboundFrameSink;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::TerminalKind;
use crate::supervisor::actor::ActorComponents;
use crate::supervisor::ws::WsSessionWriter;
use crate::ws::Clock;

use super::session_ports::SessionHandle;

/// Actor frame sink installed in the session inbound sink bundle
/// (`InboundSinkSet.actor`). Owns only the waiter/invocation correlation maps
/// that the lane owners deliberately do not retain (rpc_id → requesting
/// session, invocation_id → caller/owner sessions and fence).
#[derive(Debug)]
pub struct ActorFrameSink {
    components: Arc<ActorComponents>,
    session: SessionHandle,
    epoch_store: Arc<ActiveRoutingEpochStore>,
    writer: Arc<dyn WsSessionWriter>,
    clock: Arc<dyn Clock>,
    waiters: Mutex<HashMap<String, (RuntimeSessionEpoch, ActorLogicalKey)>>,
    invocations: Mutex<HashMap<String, InvocationCorrelation>>,
}

#[derive(Debug, Clone)]
struct InvocationCorrelation {
    caller: RuntimeSessionEpoch,
    owner: RuntimeSessionEpoch,
    fence: ActorOwnerFence,
}

impl ActorFrameSink {
    pub fn new(
        components: Arc<ActorComponents>,
        session: SessionHandle,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        writer: Arc<dyn WsSessionWriter>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            components,
            session,
            epoch_store,
            writer,
            clock,
            waiters: Mutex::new(HashMap::new()),
            invocations: Mutex::new(HashMap::new()),
        }
    }

    fn session_token(session: &RuntimeSessionEpoch) -> String {
        format!("{}#{}", session.replica_id, session.connection_generation)
    }

    fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), TerminalKind> {
        self.writer
            .write(runtime, bytes)
            .map_err(|_| TerminalKind::MalformedFrame)
    }

    fn owner_session(&self, runtime_id: &str) -> Result<RuntimeSessionEpoch, TerminalKind> {
        let layer = self
            .session
            .layer()
            .ok_or(TerminalKind::UnimplementedFamily)?;
        layer
            .current_session_by_replica(runtime_id)
            .ok_or(TerminalKind::UnimplementedFamily)
    }

    fn route_authority(&self) -> Result<ActorOwnerRouteAuthority, TerminalKind> {
        let epoch = self
            .epoch_store
            .capture()
            .ok_or(TerminalKind::UnimplementedFamily)?;
        Ok(ActorOwnerRouteAuthority {
            assembly_identity: epoch.assembly_identity().to_string(),
            assembly_generation: epoch.assembly_generation(),
        })
    }

    fn error_frame(
        &self,
        frame_type: &str,
        rpc_id: &str,
        code: &str,
        message: &str,
    ) -> Result<Vec<u8>, TerminalKind> {
        let header = ActorSpawnRuntimeErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: frame_type.to_string(),
            rpc_id: rpc_id.to_string(),
            error: skiff_runtime_transport::protocol::RuntimeErrorFramePayload {
                code: code.to_string(),
                message: message.to_string(),
                status: None,
                details: None,
            },
        };
        encode_binary_frame(&header, &[]).map_err(|_| TerminalKind::MalformedFrame)
    }

    fn handle_get_or_create(
        &self,
        session: &RuntimeSessionEpoch,
        raw: &[u8],
    ) -> Result<(), TerminalKind> {
        let (header, payload) =
            decode_typed_binary_frame::<ActorGetOrCreateRequestFrameHeader>(raw)
                .map_err(|_| TerminalKind::MalformedFrame)?;
        let actor_key = ActorLogicalKey::from_wire(&header.actor_key);
        let owner_connection = Self::session_token(session);
        let route_authority = ActorOwnerRouteAuthority {
            assembly_identity: header.activation_identity.assembly_identity.clone(),
            assembly_generation: header.activation_identity.generation,
        };
        let request = ActorGetOrCreateRequest {
            rpc_id: header.rpc_id.clone(),
            actor_key: actor_key.clone(),
            actor_abi_identity: ActorAbiIdentity::new(header.actor_abi_identity),
            actor_implementation_identity: ActorImplementationIdentity::new(
                header.actor_implementation_identity,
            ),
            declaration_owner: header.declaration_owner.clone(),
            bootstrap_bytes: payload,
            owner_runtime_id: header.runtime_id.clone(),
            owner_connection: owner_connection.clone(),
            route_authority: route_authority.clone(),
            deadline: header.deadline.clone(),
            test_case_capability: header.test_case_capability.clone(),
            test_case_parent_request_id: header.test_case_parent_request_id.clone(),
            now: self.clock.now_ms(),
        };
        match self.components.activation_broker.get_or_create(&request) {
            GetOrCreateOutcome::Resolved(actor_ref) => {
                let response = ActorGetOrCreateResponseFrameHeader {
                    schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                    envelope_type: "actor.getOrCreate.response".to_string(),
                    rpc_id: header.rpc_id,
                    actor_ref: actor_ref.to_wire(),
                };
                let bytes = encode_binary_frame(&response, &[])
                    .map_err(|_| TerminalKind::MalformedFrame)?;
                self.write(session, bytes)
            }
            GetOrCreateOutcome::Joined | GetOrCreateOutcome::StartedActivation { .. } => {
                self.waiters
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(header.rpc_id.clone(), (session.clone(), actor_key));
                Ok(())
            }
            GetOrCreateOutcome::LineageConflict => {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    "ActorCreateLineageConflict",
                    "different actor test lineage while a claim is in flight",
                )?;
                self.write(session, bytes)
            }
            GetOrCreateOutcome::Saturated => {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    "ActorActivationSaturated",
                    "actor activation claim budget reached",
                )?;
                self.write(session, bytes)
            }
            GetOrCreateOutcome::Failed { code } => {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    &code,
                    "actor get-or-create failed closed",
                )?;
                self.write(session, bytes)
            }
        }
    }

    fn resolve_activation_ack(&self, ack: ActivationAckOutcome) {
        let waiters = match ack {
            ActivationAckOutcome::Committed { waiters, .. }
            | ActivationAckOutcome::Aborted { waiters }
            | ActivationAckOutcome::CommitRejected { waiters } => waiters,
            _ => return,
        };
        let mut waiter_map = self
            .waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for rpc_id in waiters {
            let Some((session, actor_key)) = waiter_map.remove(&rpc_id) else {
                continue;
            };
            let outcome = self.components.activation_broker.outcome_for(&rpc_id);
            match outcome.as_deref() {
                Some(value) if value.starts_with("resolved:") => {
                    if let Ok(epoch) = value["resolved:".len()..].parse::<u64>() {
                        let response = ActorGetOrCreateResponseFrameHeader {
                            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                            envelope_type: "actor.getOrCreate.response".to_string(),
                            rpc_id,
                            actor_ref: actor_key.to_actor_ref(epoch).to_wire(),
                        };
                        if let Ok(bytes) = encode_binary_frame(&response, &[]) {
                            let _ = self.writer.write(&session, bytes);
                        }
                    }
                }
                Some(value) if value.starts_with("failed:") => {
                    let code = value["failed:".len()..].to_string();
                    if let Ok(bytes) = self.error_frame(
                        "actor.getOrCreate.error",
                        &rpc_id,
                        &code,
                        "actor get-or-create failed closed",
                    ) {
                        let _ = self.writer.write(&session, bytes);
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_find(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        let (header, _) = decode_typed_binary_frame::<ActorFindRequestFrameHeader>(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?;
        let key = ActorLogicalKey::from_wire(&header.actor_key);
        let fence = self.components.registry.current_owner(&key);
        let response = ActorFindResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.response".to_string(),
            rpc_id: header.rpc_id,
            found: fence.is_some(),
            actor_ref: fence.map(|fence| key.to_actor_ref(fence.epoch).to_wire()),
        };
        let bytes =
            encode_binary_frame(&response, &[]).map_err(|_| TerminalKind::MalformedFrame)?;
        self.write(session, bytes)
    }

    fn handle_remove(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        let (header, _) = decode_typed_binary_frame::<ActorRemoveRequestFrameHeader>(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?;
        let key = ActorLogicalKey::from_wire(&header.actor_key);
        let removed = match self.components.registry.current_owner(&key) {
            Some(fence) => self
                .components
                .registry
                .release(&key, &fence, OwnerReleaseReason::Upgraded)
                .is_ok(),
            None => false,
        };
        let response = ActorRemoveResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.remove.response".to_string(),
            rpc_id: header.rpc_id,
            removed,
        };
        let bytes =
            encode_binary_frame(&response, &[]).map_err(|_| TerminalKind::MalformedFrame)?;
        self.write(session, bytes)
    }

    fn handle_replace(
        &self,
        session: &RuntimeSessionEpoch,
        raw: &[u8],
    ) -> Result<(), TerminalKind> {
        let (header, _) = decode_typed_binary_frame::<ActorReplaceRequestFrameHeader>(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?;
        let bytes = self.error_frame(
            "actor.replace.error",
            &header.rpc_id,
            "ActorReplaceUnavailable",
            "actor replace routing is not wired until E-actor-rust",
        )?;
        self.write(session, bytes)
    }

    fn handle_method_frame(
        &self,
        session: &RuntimeSessionEpoch,
        frame: ActorMethodFrame,
    ) -> Result<(), TerminalKind> {
        match frame {
            ActorMethodFrame::Invoke(header, payload) => {
                let key = ActorLogicalKey::from_actor_ref(&header.actor_ref);
                let Some(fence) = self.components.registry.current_owner(&key) else {
                    // TS parity: OwnerUnavailable produces no error frame; the
                    // caller observes the relay deadline.
                    return Ok(());
                };
                let owner = match self.owner_session(&fence.owner_runtime_id) {
                    Ok(owner) => owner,
                    Err(_) => return Ok(()),
                };
                let owner_connection = Self::session_token(&owner);
                let input = ActorInvokeInput {
                    invocation_id: header.invocation_id.clone(),
                    caller_connection: Self::session_token(session),
                    caller_runtime_id: session.replica_id.clone(),
                    owner_fence: fence.clone(),
                    owner_connection: owner_connection.clone(),
                    route_authority: self.route_authority()?,
                    correlation: header.cancellation_correlation.clone(),
                    deadline: Some(header.deadline.clone()),
                    test_case_capability: header.test_case_capability.clone(),
                    now: self.clock.now_ms(),
                };
                match self.components.relay.invoke(&input) {
                    Ok(()) => {
                        self.invocations
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert(
                                header.invocation_id.clone(),
                                InvocationCorrelation {
                                    caller: session.clone(),
                                    owner: owner.clone(),
                                    fence: fence.clone(),
                                },
                            );
                        let route = ActorOwnerRouteAuthorityFrameHeader {
                            assembly_identity: input.route_authority.assembly_identity,
                            assembly_generation: input.route_authority.assembly_generation,
                        };
                        let owner_invoke = ActorOwnerInvokeFrameHeader {
                            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                            envelope_type: "actor.owner.invoke".to_string(),
                            target_runtime_id: fence.owner_runtime_id.clone(),
                            owner_fence: ActorOwnerFenceFrameHeader {
                                owner_runtime_id: fence.owner_runtime_id.clone(),
                                owner_lease_id: fence.owner_lease_id.clone(),
                                epoch: fence.epoch,
                                actor_abi_identity: fence.actor_abi_identity.clone(),
                                actor_implementation_identity: fence
                                    .actor_implementation_identity
                                    .clone(),
                                declaration_owner: fence.declaration_owner.clone(),
                            },
                            invoke: header,
                            route_authority: route,
                            activation_bootstrap: None,
                        };
                        let bytes = encode_actor_owner_invoke_frame(&owner_invoke, &payload)
                            .map_err(|_| TerminalKind::MalformedFrame)?;
                        self.write(&owner, bytes)
                    }
                    Err(InvocationError::Duplicate | InvocationError::Saturated) => Ok(()),
                }
            }
            ActorMethodFrame::Return(header, payload) => {
                let correlation = self
                    .invocations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&header.invocation_id)
                    .cloned();
                let Some(correlation) = correlation else {
                    return Ok(());
                };
                match self.components.relay.on_owner_settle(
                    &header.invocation_id,
                    &correlation.fence,
                    &Self::session_token(session),
                    OwnerSettleKind::Return,
                ) {
                    Ok(_) => {
                        self.invocations
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .remove(&header.invocation_id);
                        let forward = ActorMethodFrame::Return(header, payload);
                        let bytes = encode_actor_method_frame(&forward)
                            .map_err(|_| TerminalKind::MalformedFrame)?;
                        self.write(&correlation.caller, bytes)
                    }
                    Err(_) => Ok(()),
                }
            }
            ActorMethodFrame::Error(header) => {
                let correlation = self
                    .invocations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&header.invocation_id)
                    .cloned();
                let Some(correlation) = correlation else {
                    return Ok(());
                };
                match self.components.relay.on_owner_settle(
                    &header.invocation_id,
                    &correlation.fence,
                    &Self::session_token(session),
                    OwnerSettleKind::Error,
                ) {
                    Ok(_) => {
                        self.invocations
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .remove(&header.invocation_id);
                        let bytes = encode_actor_method_frame(&ActorMethodFrame::Error(header))
                            .map_err(|_| TerminalKind::MalformedFrame)?;
                        self.write(&correlation.caller, bytes)
                    }
                    Err(_) => Ok(()),
                }
            }
            ActorMethodFrame::Cancel(header) => {
                let correlation = self
                    .invocations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&header.invocation_id)
                    .cloned();
                let Some(correlation) = correlation else {
                    return Ok(());
                };
                match self.components.relay.on_caller_cancel(
                    &Self::session_token(session),
                    &header.invocation_id,
                    &header.cancellation_correlation,
                ) {
                    Ok(_) => {
                        self.invocations
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .remove(&header.invocation_id);
                        let bytes = encode_actor_method_frame(&ActorMethodFrame::Cancel(header))
                            .map_err(|_| TerminalKind::MalformedFrame)?;
                        self.write(&correlation.owner, bytes)
                    }
                    Err(_) => Ok(()),
                }
            }
        }
    }

    fn handle_owner_failure(
        &self,
        session: &RuntimeSessionEpoch,
        raw: &[u8],
    ) -> Result<(), TerminalKind> {
        let header =
            decode_actor_owner_failure_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
        let correlation = self
            .invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&header.invocation_id)
            .cloned();
        let Some(correlation) = correlation else {
            return Ok(());
        };
        if correlation.fence.owner_runtime_id != header.owner_runtime_id
            || correlation.fence.owner_lease_id != header.owner_lease_id
            || correlation.fence.epoch != header.epoch
        {
            return Ok(());
        }
        match self.components.relay.on_owner_settle(
            &header.invocation_id,
            &correlation.fence,
            &Self::session_token(session),
            OwnerSettleKind::Error,
        ) {
            Ok(_) => {
                self.invocations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&header.invocation_id);
                let bytes =
                    encode_binary_frame(&header, &[]).map_err(|_| TerminalKind::MalformedFrame)?;
                self.write(&correlation.caller, bytes)
            }
            Err(_) => Ok(()),
        }
    }

    fn handle_owner_control_ack(
        &self,
        session: &RuntimeSessionEpoch,
        raw: &[u8],
    ) -> Result<(), TerminalKind> {
        let (header, _) = decode_typed_binary_frame::<ActorOwnerControlAckFrameHeader>(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?;
        let connection = Self::session_token(session);
        if header.operation
            == skiff_runtime_transport::actor_owner::ActorOwnerControlOperation::ActivateInitial
        {
            let ack = self.components.activation_broker.on_activation_ack(
                &header.request_id,
                &header.runtime_id,
                &connection,
                header.accepted,
                self.clock.now_ms(),
            );
            self.resolve_activation_ack(ack);
            return Ok(());
        }
        let _ = self.components.control_broker.on_ack(
            &header.runtime_id,
            &header.request_id,
            header.operation,
            &connection,
            header.accepted,
        );
        Ok(())
    }
}

impl InboundFrameSink for ActorFrameSink {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Actor
    }

    fn accepts_frame_type(&self, frame_type: &str) -> bool {
        frame_type.starts_with("actor.")
    }

    fn handle(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        if let Ok(control) = decode_actor_owner_control_frame(raw) {
            // `actor.owner.control` is Router→Runtime only; an inbound control
            // frame is a direction violation.
            let _ = control;
            return Err(TerminalKind::MalformedFrame);
        }
        match decode_actor_method_frame(raw) {
            Ok(frame) => return self.handle_method_frame(session, frame),
            Err(_) => {}
        }
        let frame_type = skiff_runtime_transport::protocol::decode_binary_frame(raw)
            .map_err(|_| TerminalKind::MalformedFrame)?
            .header
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        match frame_type.as_str() {
            "actor.getOrCreate.request" => self.handle_get_or_create(session, raw),
            "actor.find.request" => self.handle_find(session, raw),
            "actor.remove.request" => self.handle_remove(session, raw),
            "actor.replace.request" => self.handle_replace(session, raw),
            "actor.owner.control.ack" => self.handle_owner_control_ack(session, raw),
            "actor.owner.failure" => self.handle_owner_failure(session, raw),
            _ => Err(TerminalKind::MalformedFrame),
        }
    }
}
