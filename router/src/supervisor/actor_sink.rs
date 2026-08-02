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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, encode_actor_method_frame, ActorMethodCancelFrameHeader,
    ActorMethodCancelReason, ActorMethodDeadlineFrameHeader, ActorMethodFrame,
    ActorMethodInvokeFrameHeader, ACTOR_ARGUMENTS_ENCODING_V1,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader, ActorOwnerControlOperation,
    ActorOwnerFenceFrameHeader, ActorOwnerInvokeFrameHeader, ActorOwnerRouteAuthorityFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_request_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_spawn_submit_error_frame, encode_spawn_submit_response_frame,
    ActorFindRequestFrameHeader, ActorFindResponseFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorGetOrCreateResponseFrameHeader, ActorRemoveRequestFrameHeader,
    ActorRemoveResponseFrameHeader, ActorReplaceRequestFrameHeader,
    ActorSpawnRuntimeErrorFrameHeader, RuntimeErrorFramePayload, RuntimeFrameFamily,
    SpawnCallerKind, SpawnSubmitRequestFrame, SpawnSubmitRequestFrameHeaderV2,
    SpawnSubmitResponseFrameHeader, SpawnTargetKind as WireSpawnTargetKind,
    RUNTIME_FRAME_SCHEMA_VERSION, SPAWN_SUBMIT_RESPONSE_STATUS_SUBMITTED,
};

use crate::actor::{
    ActivationAckOutcome, ActorGetOrCreateRequest, ActorInvokeInput, ActorLogicalKey,
    ActorMethodSpawnExecutionSink, ActorOwnerFence, ActorOwnerRouteAuthority, CatalogQuery,
    GetOrCreateOutcome, InvocationError, OwnerReleaseReason, OwnerSettleKind, SpawnErrorCode,
    SpawnSubmitAcceptance, SpawnSubmitError, DEFAULT_OWNER_LEASE_TTL_MS,
    SPAWNED_ACTOR_METHOD_DEADLINE_MS, SPAWNED_ACTOR_METHOD_LEASE_MS,
};
use crate::bootstrap::ActiveRoutingEpochStore;
use crate::dispatch::{
    derived_deadline, RequestDeadline, RequestDispatcher, SpawnSubmit, SpawnSubmitResult,
    SpawnTargetKind,
};
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
    /// Spawned actor-method invocations have no caller to forward to; the
    /// owner settle still goes through the relay so its pending returns to
    /// zero (E-actor-rust; C-spawn §4.5 accepted spawn lifecycle).
    spawn_invocations: Mutex<HashMap<String, SpawnInvocationCorrelation>>,
    spawn_seq: AtomicU64,
}

#[derive(Debug, Clone)]
struct InvocationCorrelation {
    caller: RuntimeSessionEpoch,
    owner: RuntimeSessionEpoch,
    fence: ActorOwnerFence,
}

#[derive(Debug, Clone)]
struct SpawnInvocationCorrelation {
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
            spawn_invocations: Mutex::new(HashMap::new()),
            spawn_seq: AtomicU64::new(0),
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
        // E-actor-parity owner selection: the Router pins the owner runtime
        // deterministically over the registered session candidates with
        // `sha256(actorIdHash) % candidates.len()` (TS coordinator parity).
        // The wire `runtimeId` is the caller, not an owner preference; using
        // the first caller as owner would make concurrent creates
        // nondeterministic and diverge from the TS two-replica full chain.
        let owner = {
            let Some(layer) = self.session.layer() else {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    "OwnerUnavailable",
                    "no Runtime is available to own the Actor",
                )?;
                return self.write(session, bytes);
            };
            let Some(epoch) = self.epoch_store.capture() else {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    "OwnerUnavailable",
                    "no active routing epoch is available to select an Actor owner",
                )?;
                return self.write(session, bytes);
            };
            let candidates = layer.candidates(&epoch.registered_tuple());
            let Some(owner) = pick_owner_candidate(&candidates, &actor_key.actor_id_hash) else {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    "OwnerUnavailable",
                    "no Runtime is available to own the Actor",
                )?;
                return self.write(session, bytes);
            };
            owner.clone()
        };
        let owner_connection = Self::session_token(&owner);
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
            owner_runtime_id: owner.replica_id.clone(),
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

    /// Activation deadline terminal: fail every suspended getOrCreate waiter
    /// with `actor.getOrCreate.error` (ActivationTimeout) and drop the
    /// waiter correlation (timer sweep owner).
    pub fn resolve_activation_timeout(&self, outcome: &crate::actor::ActivationTimeoutOutcome) {
        let mut waiter_map = self
            .waiters
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for rpc_id in &outcome.waiters {
            let Some((session, _actor_key)) = waiter_map.remove(rpc_id) else {
                continue;
            };
            if let Ok(bytes) = self.error_frame(
                "actor.getOrCreate.error",
                rpc_id,
                "ActivationTimeout",
                "actor activation deadline elapsed without owner ACK",
            ) {
                let _ = self.writer.write(&session, bytes);
            }
        }
    }

    /// Relay invocation deadline: send `actor.method.cancel` to the exact
    /// owner (caller or spawn correlation) and remove the correlation so the
    /// relay pending and this sink's map return to zero.
    pub fn on_relay_deadline(&self, invocation_id: &str, correlation: &str) {
        let connection_owner = {
            let invocations = self
                .invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            invocations
                .get(invocation_id)
                .map(|entry| (entry.owner.clone(), entry.fence.clone()))
        };
        if let Some((owner, _fence)) = connection_owner {
            self.write_owner_cancel(
                invocation_id,
                &owner,
                correlation,
                ActorMethodCancelReason::DeadlineExceeded,
            );
            self.invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(invocation_id);
            return;
        }
        let spawn_owner = {
            let spawn_invocations = self
                .spawn_invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            spawn_invocations
                .get(invocation_id)
                .map(|entry| (entry.owner.clone(), entry.fence.clone()))
        };
        if let Some((owner, _fence)) = spawn_owner {
            self.write_owner_cancel(
                invocation_id,
                &owner,
                correlation,
                ActorMethodCancelReason::DeadlineExceeded,
            );
            self.spawn_invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(invocation_id);
        }
    }

    /// Exact runtime session closed (actor lane consumer): drop caller/owner
    /// correlations for this session epoch and write owner cancels for
    /// caller-side disconnects. Registry/relay/control cleanup is owned by
    /// [`super::actor::ActorSessionOwnerConsumer`].
    pub fn on_runtime_session_closed(&self, session: &RuntimeSessionEpoch) {
        let token = Self::session_token(session);
        let (caller_cancels, _caller_terminals) =
            self.components.relay.on_caller_disconnect(&token);
        let mut invocations = self
            .invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for cancel in caller_cancels {
            let Some(correlation) = invocations.get(&cancel.invocation_id).cloned() else {
                continue;
            };
            self.write_owner_cancel(
                &cancel.invocation_id,
                &correlation.owner,
                &cancel.correlation,
                ActorMethodCancelReason::Cancelled,
            );
            invocations.remove(&cancel.invocation_id);
        }
        invocations.retain(|_id, correlation| {
            correlation.owner != *session && correlation.caller != *session
        });
        drop(invocations);
        self.spawn_invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_id, correlation| correlation.owner != *session);
    }

    fn write_owner_cancel(
        &self,
        invocation_id: &str,
        owner: &RuntimeSessionEpoch,
        correlation: &str,
        reason: ActorMethodCancelReason,
    ) {
        let header = ActorMethodCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.cancel".to_string(),
            invocation_id: invocation_id.to_string(),
            cancellation_correlation: correlation.to_string(),
            reason,
        };
        if let Ok(bytes) = encode_actor_method_frame(&ActorMethodFrame::Cancel(header)) {
            let _ = self.writer.write(owner, bytes);
        }
    }

    fn settle_spawn_owner_return(
        &self,
        invocation_id: &str,
        session: &RuntimeSessionEpoch,
    ) -> Result<(), TerminalKind> {
        let spawn = self
            .spawn_invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(invocation_id)
            .cloned();
        let Some(spawn) = spawn else {
            return Ok(());
        };
        if self
            .components
            .relay
            .on_owner_settle(
                invocation_id,
                &spawn.fence,
                &Self::session_token(session),
                OwnerSettleKind::Return,
            )
            .is_ok()
        {
            self.spawn_invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(invocation_id);
        }
        Ok(())
    }

    fn settle_spawn_owner_error(
        &self,
        invocation_id: &str,
        session: &RuntimeSessionEpoch,
    ) -> Result<(), TerminalKind> {
        let spawn = self
            .spawn_invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(invocation_id)
            .cloned();
        let Some(spawn) = spawn else {
            return Ok(());
        };
        if self
            .components
            .relay
            .on_owner_settle(
                invocation_id,
                &spawn.fence,
                &Self::session_token(session),
                OwnerSettleKind::Error,
            )
            .is_ok()
        {
            self.spawn_invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(invocation_id);
        }
        Ok(())
    }

    /// Real actor-method spawn execution owner (C-spawn §3.3/§4.5): the
    /// accepted spawn is executed against the current owner fence of the
    /// target actor; the spawn outlives the parent lifecycle and this router
    /// stores no parent-child mapping. The caller-less invocation settles
    /// through the relay on the owner's return/error.
    pub fn spawn_actor_method_execution(
        &self,
        wire: &SpawnSubmitRequestFrame,
    ) -> Result<(), SpawnSubmitError> {
        let header = &wire.header;
        let actor_method = header
            .actor_method
            .as_ref()
            .ok_or_else(|| SpawnSubmitError::new(SpawnErrorCode::TargetKindMismatch))?;
        let key = ActorLogicalKey::from_actor_ref(&actor_method.actor_ref);
        let Some(fence) = self.components.registry.current_owner(&key) else {
            return Err(SpawnSubmitError::new(SpawnErrorCode::UnknownTarget));
        };
        if fence.epoch != actor_method.actor_ref.epoch {
            return Err(SpawnSubmitError::new(SpawnErrorCode::AuthorityMismatch));
        }
        let owner = self
            .owner_session(&fence.owner_runtime_id)
            .map_err(|_| SpawnSubmitError::new(SpawnErrorCode::ParentConnectionMismatch))?;
        let now = self.clock.now_ms();
        let fence = self
            .components
            .registry
            .renew(&key, &fence, SPAWNED_ACTOR_METHOD_LEASE_MS, now)
            .unwrap_or(fence);
        let owner_connection = Self::session_token(&owner);
        self.components
            .lease_scheduler
            .mark_live(&key, now, &owner_connection);
        let seq = self.spawn_seq.fetch_add(1, Ordering::Relaxed);
        let invocation_id = format!("actor-spawn-{seq}");
        let correlation = format!("actor-spawn-{seq}:cancel");
        let deadline = ActorMethodDeadlineFrameHeader {
            timeout_ms: SPAWNED_ACTOR_METHOD_DEADLINE_MS,
            expires_at: iso_timestamp(now.saturating_add(SPAWNED_ACTOR_METHOD_DEADLINE_MS)),
        };
        let route_authority = self
            .route_authority()
            .map_err(|_| SpawnSubmitError::new(SpawnErrorCode::AuthorityMismatch))?;
        let input = ActorInvokeInput {
            invocation_id: invocation_id.clone(),
            // A spawned method's parent authority is its owner runtime
            // connection: when the spawned method itself spawns
            // (`callerKind=actorInvocation`), the relay parent snapshot must
            // resolve to this exact session (C-spawn §4.2).
            caller_connection: owner_connection.clone(),
            caller_runtime_id: owner.replica_id.clone(),
            owner_fence: fence.clone(),
            owner_connection: owner_connection.clone(),
            route_authority: route_authority.clone(),
            correlation: correlation.clone(),
            deadline: Some(deadline.clone()),
            test_case_capability: None,
            now,
        };
        self.components
            .relay
            .invoke(&input)
            .map_err(|_| SpawnSubmitError::new(SpawnErrorCode::Saturated))?;
        let invoke_header = ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.invoke".to_string(),
            invocation_id: invocation_id.clone(),
            actor_ref: actor_method.actor_ref.clone(),
            declaration_owner: actor_method.declaration_owner.clone(),
            actor_abi_identity: actor_method.actor_abi_identity.clone(),
            actor_implementation_identity: actor_method.actor_implementation_identity.clone(),
            method_identity: actor_method.method_identity.clone(),
            arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.to_string(),
            deadline,
            cancellation_correlation: correlation.clone(),
            trace_id: header.trace_id.clone(),
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let route = ActorOwnerRouteAuthorityFrameHeader {
            assembly_identity: route_authority.assembly_identity,
            assembly_generation: route_authority.assembly_generation,
        };
        let owner_invoke = ActorOwnerInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.owner.invoke".to_string(),
            target_runtime_id: fence.owner_runtime_id.clone(),
            owner_fence: Self::fence_to_wire(&fence),
            invoke: invoke_header,
            route_authority: route,
            activation_bootstrap: None,
        };
        let bytes = encode_actor_owner_invoke_frame(&owner_invoke, &wire.payload)
            .map_err(|_| SpawnSubmitError::new(SpawnErrorCode::UnknownTarget))?;
        self.write(&owner, bytes)
            .map_err(|_| SpawnSubmitError::new(SpawnErrorCode::ParentConnectionMismatch))?;
        self.spawn_invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                invocation_id,
                SpawnInvocationCorrelation {
                    owner: owner.clone(),
                    fence,
                },
            );
        Ok(())
    }

    fn fence_to_wire(fence: &ActorOwnerFence) -> ActorOwnerFenceFrameHeader {
        ActorOwnerFenceFrameHeader {
            owner_runtime_id: fence.owner_runtime_id.clone(),
            epoch: fence.epoch,
            actor_abi_identity: fence.actor_abi_identity.clone(),
            actor_implementation_identity: fence.actor_implementation_identity.clone(),
            declaration_owner: fence.declaration_owner.clone(),
            owner_lease_id: fence.owner_lease_id.clone(),
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
                // Canonical projection admission (A2 hard cut / C-actor §3.1,
                // E-actor-parity): the router admits actor method invocations
                // exclusively through the A0 projection catalog. A miss fails
                // closed exactly like the TS dispatcher's UnknownMethod
                // rejection: no synthetic error frame is written; the caller
                // observes the relay deadline / provider-unavailable terminal.
                let query = CatalogQuery::new(
                    header.actor_ref.service_id.clone(),
                    header.actor_abi_identity.clone(),
                    header.actor_implementation_identity.clone(),
                    header.method_identity.clone(),
                );
                if !self.components.catalog_view.has_method(&query) {
                    return Ok(());
                }
                let Some(fence) = self.components.registry.current_owner(&key) else {
                    // TS parity: OwnerUnavailable produces no error frame; the
                    // caller observes the relay deadline.
                    return Ok(());
                };
                let owner = match self.owner_session(&fence.owner_runtime_id) {
                    Ok(owner) => owner,
                    Err(_) => return Ok(()),
                };
                let now = self.clock.now_ms();
                // Router-side owner lease renewal on method activity (TS
                // parity: no RenewLease wire operation; an idle actor's lease
                // expires and idle eviction reclaims it).
                let fence = self
                    .components
                    .registry
                    .renew(&key, &fence, DEFAULT_OWNER_LEASE_TTL_MS, now)
                    .unwrap_or(fence);
                let owner_connection = Self::session_token(&owner);
                self.components
                    .lease_scheduler
                    .mark_live(&key, now, &owner_connection);
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
                    now,
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
                if let Some(correlation) = correlation {
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
                } else {
                    self.settle_spawn_owner_return(&header.invocation_id, session)
                }
            }
            ActorMethodFrame::Error(header) => {
                let correlation = self
                    .invocations
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&header.invocation_id)
                    .cloned();
                if let Some(correlation) = correlation {
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
                } else {
                    self.settle_spawn_owner_error(&header.invocation_id, session)
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
                    // Spawned invocations have no caller to cancel; a stray
                    // cancel is dropped fail-closed (relay deadline owns the
                    // terminal).
                    self.spawn_invocations
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&header.invocation_id);
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
        let ack = self.components.control_broker.on_ack(
            &header.runtime_id,
            &header.request_id,
            header.operation,
            &connection,
            header.accepted,
        );
        if header.operation == ActorOwnerControlOperation::IdleEvict
            && matches!(ack, crate::actor::ControlAckOutcome::Accepted)
            && header.accepted
        {
            if let Some(eviction_request_id) = header.request_id.strip_prefix("control:idle-evict-")
            {
                let key = self
                    .components
                    .idle_evictions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(eviction_request_id);
                if let Some(key) = key {
                    let _ = self
                        .components
                        .lease_scheduler
                        .on_eviction_ack(&key, eviction_request_id);
                }
            }
        }
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
        if let Ok(frame) = decode_actor_method_frame(raw) {
            return self.handle_method_frame(session, frame);
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

impl ActorMethodSpawnExecutionSink for ActorFrameSink {
    fn on_accept(&self, acceptance: &SpawnSubmitAcceptance) {
        let Some(wire) = self.components.spawn_wire_store.get(&acceptance.spawn_id) else {
            self.components.spawn_wire_store.record_orphan_accept();
            return;
        };
        let outcome = self.spawn_actor_method_execution(&wire.frame);
        self.components
            .spawn_wire_store
            .set_outcome(&acceptance.spawn_id, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::pick_owner_candidate;
    use crate::session::identity::RuntimeSessionEpoch;

    #[test]
    fn owner_selection_pins_ts_hash_modulo_candidates() {
        // E-actor-parity: the Router pins the owner with
        // sha256(actorIdHash) big-endian first 4 bytes modulo the sorted
        // candidate count (TS coordinator pickOwner parity).
        let session = |replica: &str| RuntimeSessionEpoch {
            replica_id: replica.to_string(),
            connection_generation: 1,
        };
        let first = session("actor-parity-replica-1");
        let second = session("actor-parity-replica-2");
        let candidates = [first.clone(), second.clone()];
        let aaa = format!("sha256:{}", "a".repeat(64));
        let bbb = format!("sha256:{}", "b".repeat(64));
        assert_eq!(
            pick_owner_candidate(&candidates, &aaa).expect("owner"),
            &second
        );
        assert_eq!(
            pick_owner_candidate(&candidates, &bbb).expect("owner"),
            &first
        );
        assert_eq!(
            pick_owner_candidate(&candidates, &aaa).expect("owner"),
            pick_owner_candidate(&candidates, &aaa).expect("owner")
        );
        assert_eq!(pick_owner_candidate(&[], &aaa), None);
        let three = [first, second.clone(), session("actor-parity-replica-3")];
        assert_eq!(pick_owner_candidate(&three, &bbb).expect("owner"), &second);
    }
}

/// Deterministic router-side owner selection (E-actor-parity, TS coordinator
/// parity): `sha256(actorIdHash)` big-endian first four bytes modulo the
/// sorted registered candidate count. The candidates come from the session
/// admission pool for the captured committed tuple (routable, current,
/// non-cancelled, sorted by replica id), matching the TS
/// `actorRuntimeCandidates` ordering.
fn pick_owner_candidate<'a>(
    candidates: &'a [RuntimeSessionEpoch],
    actor_id_hash: &str,
) -> Option<&'a RuntimeSessionEpoch> {
    if candidates.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(actor_id_hash.as_bytes());
    let digest = hasher.finalize();
    let index = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as usize
        % candidates.len();
    candidates.get(index)
}

/// Production `spawn.submit.request` inbound sink (E-actor-rust). Installed
/// into `InboundSinkSet.spawn`; the demux already narrowed the frame-level
/// direction so only Runtime→Router `spawn.submit.request` reaches here.
///
/// Parent authority is resolved exactly (C-spawn §4):
/// - `callerKind=request` resolves through the `RequestDispatcher` pending
///   (function targets become dispatcher-owned derived spawns);
/// - `callerKind=actorInvocation` resolves through the
///   `ActorInvocationRelay` parent store;
/// - both namespaces hit → `ambiguous` fail closed; neither → fail closed;
/// - accepted actor-method spawns are handed to the real execution owner with
///   the raw wire request and the response is written from its synchronous
///   admission outcome; accepted spawns are separated from parent lifecycle.
#[derive(Debug)]
pub struct ActorSpawnFrameSink {
    components: Arc<ActorComponents>,
    dispatcher: Arc<RequestDispatcher>,
    epoch_store: Arc<ActiveRoutingEpochStore>,
    writer: Arc<dyn WsSessionWriter>,
    clock: Arc<dyn Clock>,
    default_spawn_deadline_ms: u64,
    seq: AtomicU64,
}

impl ActorSpawnFrameSink {
    pub fn new(
        components: Arc<ActorComponents>,
        dispatcher: Arc<RequestDispatcher>,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        writer: Arc<dyn WsSessionWriter>,
        clock: Arc<dyn Clock>,
        default_spawn_deadline_ms: u64,
    ) -> Self {
        Self {
            components,
            dispatcher,
            epoch_store,
            writer,
            clock,
            default_spawn_deadline_ms,
            seq: AtomicU64::new(0),
        }
    }

    fn write(&self, session: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), TerminalKind> {
        self.writer
            .write(session, bytes)
            .map_err(|_| TerminalKind::MalformedFrame)
    }

    fn response_frame(
        rpc_id: &str,
        spawn_id: &str,
        request_id: &str,
    ) -> Result<Vec<u8>, TerminalKind> {
        let header = SpawnSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.submit.response".to_string(),
            rpc_id: rpc_id.to_string(),
            spawn_id: spawn_id.to_string(),
            request_id: request_id.to_string(),
            status: SPAWN_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
        };
        encode_spawn_submit_response_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
    }

    fn error_frame(rpc_id: &str, code: &str, message: &str) -> Result<Vec<u8>, TerminalKind> {
        let header = ActorSpawnRuntimeErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.submit.error".to_string(),
            rpc_id: rpc_id.to_string(),
            error: RuntimeErrorFramePayload {
                code: code.to_string(),
                message: message.to_string(),
                status: None,
                details: None,
            },
        };
        encode_spawn_submit_error_frame(&header).map_err(|_| TerminalKind::MalformedFrame)
    }

    fn reject_code(reason: &crate::dispatch::SpawnRejectReason) -> (&'static str, &'static str) {
        use crate::dispatch::SpawnRejectReason as Reason;
        match reason {
            Reason::NoParent | Reason::WrongParentKind => (
                "ParentNotFound",
                "spawn callerRequestId does not identify an active parent",
            ),
            Reason::ParentTerminal => ("ParentTerminal", "spawn parent is terminal"),
            Reason::ParentAuthorityMismatch => (
                "AuthorityMismatch",
                "spawn parent authority does not match the request",
            ),
            Reason::QueueFull => ("Saturated", "spawn submit capacity is saturated"),
            Reason::Ambiguous => (
                "SpawnSubmitRejected",
                "spawn parent matched both typed namespaces",
            ),
            Reason::Duplicate => ("SpawnSubmitRejected", "spawn request id is already pending"),
            Reason::Shutdown => ("SpawnSubmitRejected", "router is shutting down"),
            Reason::CallbackError => ("SpawnSubmitRejected", "spawn derived request write failed"),
        }
    }

    fn route_spawn(
        &self,
        session: &RuntimeSessionEpoch,
        frame: &SpawnSubmitRequestFrame,
        spawn_request_id: &str,
    ) -> Result<(String, String), (String, String)> {
        let header = &frame.header;
        match (header.caller_kind, header.target_kind) {
            (SpawnCallerKind::Request, WireSpawnTargetKind::Function) => {
                self.route_request_function(session, header, spawn_request_id)
            }
            (SpawnCallerKind::Request, WireSpawnTargetKind::ActorMethod) => {
                self.route_request_actor_method(session, header, spawn_request_id)
            }
            (SpawnCallerKind::ActorInvocation, WireSpawnTargetKind::ActorMethod) => {
                self.route_actor_invocation_actor_method(session, header, spawn_request_id)
            }
            (SpawnCallerKind::ActorInvocation, WireSpawnTargetKind::Function) => Err((
                "CallerKindRejected".to_string(),
                "function spawn requires a runtime assembly request parent".to_string(),
            )),
        }
    }

    fn route_request_function(
        &self,
        session: &RuntimeSessionEpoch,
        header: &SpawnSubmitRequestFrameHeaderV2,
        spawn_request_id: &str,
    ) -> Result<(String, String), (String, String)> {
        let authority = self.request_authority(session, header).ok_or_else(|| {
            (
                "AuthorityMismatch".to_string(),
                "spawn activation identity does not match the active routing epoch".to_string(),
            )
        })?;
        let parent_deadline = self.dispatcher.pending_deadline(&header.caller_request_id);
        let default_deadline = RequestDeadline {
            timeout_ms: self.default_spawn_deadline_ms,
            expires_at: iso_timestamp(
                self.clock
                    .now_ms()
                    .saturating_add(self.default_spawn_deadline_ms),
            ),
        };
        let deadline = derived_deadline(parent_deadline.as_ref(), &default_deadline);
        let spawn = SpawnSubmit {
            spawn_request_id: spawn_request_id.to_string(),
            caller_request_id: header.caller_request_id.clone(),
            target_kind: SpawnTargetKind::Function,
            target: header.target.clone(),
            authority,
            deadline: Some(deadline),
        };
        match self.dispatcher.spawn_submit(spawn) {
            SpawnSubmitResult::AcceptedDerived(result) => Ok((
                spawn_request_id.to_string(),
                result.spawn_request_id.clone(),
            )),
            SpawnSubmitResult::Rejected { reason, .. } => {
                let (code, message) = Self::reject_code(&reason);
                Err((code.to_string(), message.to_string()))
            }
            SpawnSubmitResult::ForwardedActorMethod(_) => Err((
                "SpawnSubmitRejected".to_string(),
                "function spawn was misrouted to the actor lane".to_string(),
            )),
        }
    }

    fn route_request_actor_method(
        &self,
        session: &RuntimeSessionEpoch,
        header: &SpawnSubmitRequestFrameHeaderV2,
        spawn_request_id: &str,
    ) -> Result<(String, String), (String, String)> {
        if self
            .components
            .relay
            .is_active_parent(&header.caller_request_id)
        {
            return Err((
                "SpawnSubmitRejected".to_string(),
                "spawn parent matched both typed namespaces".to_string(),
            ));
        }
        if self
            .dispatcher
            .pending_epoch(&header.caller_request_id)
            .is_none()
        {
            return Err((
                "ParentNotFound".to_string(),
                "spawn callerRequestId does not identify an active request parent".to_string(),
            ));
        }
        let authority = self.request_authority(session, header).ok_or_else(|| {
            (
                "AuthorityMismatch".to_string(),
                "spawn activation identity does not match the active routing epoch".to_string(),
            )
        })?;
        let probe = crate::actor::SpawnAuthorityProbe {
            connection: Self::session_token(session),
            runtime_id: Some(header.runtime_id.clone()),
            assembly_generation: authority.assembly_generation,
            test_case_capability: None,
        };
        self.admit_actor_method(header, spawn_request_id, &probe)
    }

    fn route_actor_invocation_actor_method(
        &self,
        session: &RuntimeSessionEpoch,
        header: &SpawnSubmitRequestFrameHeaderV2,
        spawn_request_id: &str,
    ) -> Result<(String, String), (String, String)> {
        if self
            .dispatcher
            .pending_epoch(&header.caller_request_id)
            .is_some()
        {
            return Err((
                "SpawnSubmitRejected".to_string(),
                "spawn parent matched both typed namespaces".to_string(),
            ));
        }
        let Some(parent) = self
            .components
            .relay
            .parent_snapshot(&header.caller_request_id)
        else {
            return Err((
                "ParentNotFound".to_string(),
                "spawn callerRequestId does not identify an active actor invocation parent"
                    .to_string(),
            ));
        };
        let epoch = self.epoch_store.capture().ok_or_else(|| {
            (
                "AuthorityMismatch".to_string(),
                "no active routing epoch for actor invocation spawn".to_string(),
            )
        })?;
        if epoch.assembly_identity() != header.activation_identity.assembly_identity.as_str() {
            return Err((
                "AuthorityMismatch".to_string(),
                "spawn activation identity does not match the active routing epoch".to_string(),
            ));
        }
        let probe = crate::actor::SpawnAuthorityProbe {
            connection: Self::session_token(session),
            runtime_id: Some(header.runtime_id.clone()),
            assembly_generation: parent.assembly_generation,
            test_case_capability: parent.test_case_capability.clone(),
        };
        self.admit_actor_method(header, spawn_request_id, &probe)
    }

    fn admit_actor_method(
        &self,
        header: &SpawnSubmitRequestFrameHeaderV2,
        spawn_request_id: &str,
        probe: &crate::actor::SpawnAuthorityProbe,
    ) -> Result<(String, String), (String, String)> {
        let mut admitted_header = header.clone();
        admitted_header.spawn_id = Some(spawn_request_id.to_string());
        let acceptance = self
            .components
            .spawn_router
            .submit(&admitted_header, probe)
            .map_err(|error| (error.code().as_str().to_string(), error.to_string()))?;
        let request_id = acceptance.request_id.clone();
        self.components.execution_sink.on_accept(&acceptance);
        let outcome = self
            .components
            .spawn_wire_store
            .get(&acceptance.spawn_id)
            .and_then(|wire| wire.outcome);
        match outcome {
            Some(Ok(())) => Ok((acceptance.spawn_id.clone(), request_id)),
            Some(Err(error)) => Err((error.code().as_str().to_string(), error.to_string())),
            None => Err((
                "SpawnSubmitRejected".to_string(),
                "actor-method spawn execution did not report an admission outcome".to_string(),
            )),
        }
    }

    fn request_authority(
        &self,
        session: &RuntimeSessionEpoch,
        header: &SpawnSubmitRequestFrameHeaderV2,
    ) -> Option<crate::dispatch::RequestAuthority> {
        let epoch = self.epoch_store.capture()?;
        if epoch.assembly_identity() != header.activation_identity.assembly_identity.as_str()
            || epoch.assembly_generation() != header.activation_identity.generation
        {
            return None;
        }
        let deployment = epoch.deployment_projection().iter().find(|deployment| {
            deployment.service_id == header.service_id
                && deployment.contract_version == header.service_version
                && deployment.deployment_revision.as_str()
                    == header.activation_identity.deployment_revision
        })?;
        Some(crate::dispatch::RequestAuthority {
            assembly_identity: header.activation_identity.assembly_identity.clone(),
            assembly_generation: header.activation_identity.generation,
            deployment: deployment.clone(),
            session_epoch: session.clone(),
        })
    }

    fn session_token(session: &RuntimeSessionEpoch) -> String {
        format!("{}#{}", session.replica_id, session.connection_generation)
    }
}

impl InboundFrameSink for ActorSpawnFrameSink {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Spawn
    }

    fn accepts_frame_type(&self, frame_type: &str) -> bool {
        frame_type == "spawn.submit.request"
    }

    fn handle(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        let (header, payload) =
            decode_spawn_submit_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
        let frame = SpawnSubmitRequestFrame { header, payload };
        let spawn_request_id = frame
            .header
            .spawn_id
            .clone()
            .unwrap_or_else(|| format!("spawn-{}", self.seq.fetch_add(1, Ordering::Relaxed)));
        self.components
            .spawn_wire_store
            .register(&spawn_request_id, frame.clone());
        let result = self.route_spawn(session, &frame, &spawn_request_id);
        let rpc_id = frame.header.rpc_id.clone();
        let outcome = match result {
            Ok((spawn_id, request_id)) => Self::response_frame(&rpc_id, &spawn_id, &request_id),
            Err((code, message)) => Self::error_frame(&rpc_id, &code, &message),
        };
        self.components.spawn_wire_store.remove(&spawn_request_id);
        let bytes = outcome?;
        self.write(session, bytes)
    }
}

fn iso_timestamp(epoch_ms: u64) -> String {
    let seconds = (epoch_ms / 1000) as i64;
    let millis = epoch_ms % 1000;
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}
