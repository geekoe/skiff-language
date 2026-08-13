//! Production actor inbound sink (plan §5.5, C-model-actor §3): decodes
//! Runtime→Router actor frames and drives the six W-actor owners through
//! their public APIs, then encodes Router→Runtime responses/forwards to the
//! exact session writer. No actor lane internals are modified.
//!
//! Known deferred edges (recorded in the leaf; E-actor-rust gate refines):
//! `actor.replace.request` fails closed with `ActorReplaceUnavailable`;
//! duplicate/saturated method admission drops without a synthetic error
//! frame (TS parity: those reasons return no `errorFrame`); task-family
//! frames are out of scope (M-task-repair shared-model node).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_deployment::projection::actor_routing::ActorRoutingRef;
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, encode_actor_method_frame, ActorMethodCancelFrameHeader,
    ActorMethodCancelReason, ActorMethodErrorFramePayload, ActorMethodFrame,
    ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader, ActorOwnerControlOperation,
    ActorOwnerFenceFrameHeader, ActorOwnerInvokeFrameHeader, ActorOwnerRouteAuthorityFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, ActorFindRequestFrameHeader,
    ActorFindResponseFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorGetOrCreateResponseFrameHeader, ActorRemoveRequestFrameHeader,
    ActorRemoveResponseFrameHeader, ActorReplaceRequestFrameHeader,
    ActorTaskRuntimeErrorFrameHeader, RuntimeFrameFamily, RUNTIME_FRAME_SCHEMA_VERSION,
};

use crate::actor::{
    pick_owner_candidate, ActivationAckOutcome, ActorGetOrCreateRequest, ActorInvokeInput,
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, CatalogQuery, GetOrCreateOutcome,
    InvocationError, OwnerReleaseReason, OwnerSettleKind, DEFAULT_OWNER_LEASE_TTL_MS,
};
use crate::session::demux::InboundFrameSink;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::TerminalKind;
use crate::supervisor::actor::ActorComponents;
use crate::supervisor::ws::WsSessionWriter;
use crate::task::{
    ActorAttemptTerminal, ActorAttemptTerminalSink, TaskAttemptInvocationCorrelation,
};
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
    writer: Arc<dyn WsSessionWriter>,
    clock: Arc<dyn Clock>,
    task_attempt_terminal: Arc<dyn ActorAttemptTerminalSink>,
    waiters: Mutex<HashMap<String, (RuntimeSessionEpoch, ActorLogicalKey)>>,
    invocations: Mutex<HashMap<String, InvocationCorrelation>>,
}

#[derive(Debug, Clone)]
struct InvocationCorrelation {
    caller: Option<RuntimeSessionEpoch>,
    owner: RuntimeSessionEpoch,
    fence: ActorOwnerFence,
    task_attempt: Option<TaskAttemptInvocationCorrelation>,
}

impl ActorFrameSink {
    pub fn new(
        components: Arc<ActorComponents>,
        session: SessionHandle,
        writer: Arc<dyn WsSessionWriter>,
        clock: Arc<dyn Clock>,
        task_attempt_terminal: Arc<dyn ActorAttemptTerminalSink>,
    ) -> Self {
        Self {
            components,
            session,
            writer,
            clock,
            task_attempt_terminal,
            waiters: Mutex::new(HashMap::new()),
            invocations: Mutex::new(HashMap::new()),
        }
    }

    /// Registers one task-attempt actor invocation correlation (E2b). The
    /// task admission lane has already relayed the invocation; the sink owns
    /// the terminal correlation and routes terminals back to the task control
    /// plane through [`ActorAttemptTerminalSink`].
    pub fn register_task_attempt_invocation(
        &self,
        invocation_id: &str,
        task_attempt: TaskAttemptInvocationCorrelation,
        owner: RuntimeSessionEpoch,
        fence: ActorOwnerFence,
    ) -> Result<(), String> {
        let mut invocations = self
            .invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if invocations.contains_key(invocation_id) {
            return Err(format!(
                "task-attempt actor invocation {invocation_id} is already registered"
            ));
        }
        invocations.insert(
            invocation_id.to_string(),
            InvocationCorrelation {
                caller: None,
                owner,
                fence,
                task_attempt: Some(task_attempt),
            },
        );
        Ok(())
    }

    /// Drops one invocation correlation (used when the owner frame write
    /// failed before the attempt became definite).
    pub fn unregister_invocation(&self, invocation_id: &str) {
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(invocation_id);
    }

    fn report_task_attempt_terminal(
        &self,
        correlation: &InvocationCorrelation,
        terminal: ActorAttemptTerminal,
    ) {
        if let Some(task_attempt) = &correlation.task_attempt {
            self.task_attempt_terminal.on_actor_terminal(
                &task_attempt.request_id,
                &task_attempt.task_id,
                &task_attempt.attempt_id,
                &task_attempt.lease_id,
                terminal,
            );
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

    /// Deployment-anchored route authority for one actor method invocation
    /// (M4: resolved from the actor routing catalog; no epoch).
    fn route_authority(
        &self,
        query: &CatalogQuery,
    ) -> Result<ActorOwnerRouteAuthority, TerminalKind> {
        let build_id = self
            .components
            .catalog_view
            .deployment_build_id_for(
                &query.service_id,
                &query.actor_abi_identity,
                &query.actor_implementation_identity,
            )
            .ok_or(TerminalKind::UnimplementedFamily)?;
        Ok(ActorOwnerRouteAuthority { build_id })
    }

    fn error_frame(
        &self,
        frame_type: &str,
        rpc_id: &str,
        code: &str,
        message: &str,
    ) -> Result<Vec<u8>, TerminalKind> {
        let header = ActorTaskRuntimeErrorFrameHeader {
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
        let actor_key = match self.actor_owner_service_id(&header) {
            Ok(service_id) => {
                let mut key = ActorLogicalKey::from_wire(&header.actor_key);
                key.service_id = service_id;
                key
            }
            Err(code) => {
                let bytes = self.error_frame(
                    "actor.getOrCreate.error",
                    &header.rpc_id,
                    code,
                    "actor declaration owner service cannot be resolved",
                )?;
                return self.write(session, bytes);
            }
        };
        // E-actor-parity owner selection: the Router pins the owner runtime
        // deterministically over the registered session candidates with
        // `sha256(actorIdHash) % candidates.len()` (TS coordinator parity).
        // The wire `runtimeId` is the caller, not an owner preference; using
        // the first caller as owner would make concurrent creates
        // nondeterministic and diverge from the TS two-replica full chain.
        let actor_build_id = self
            .components
            .catalog_view
            .deployment_build_id_for(
                &actor_key.service_id,
                &ActorAbiIdentity::new(header.actor_abi_identity.clone()),
                &ActorImplementationIdentity::new(header.actor_implementation_identity.clone()),
            )
            .ok_or(TerminalKind::UnimplementedFamily)?;
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
            let candidates = layer.candidates_by_build_id(&actor_build_id);
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
            build_id: actor_build_id,
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

    /// Resolves the service id an actor's logical key must use: the actor's
    /// own declaration owner service, not the caller's service. Same-service
    /// actors keep the wire caller id; package-owned actors are normalized to
    /// the deployment service that owns the declaration owner's package (M4:
    /// the actor routing catalog; no assembly).
    ///
    /// The wire package slot is the runtime's assembly code-slot index; the
    /// deployment record's `packageBindings` is a per-binding table with a
    /// different, revision-varying order, so the slot is not interpretable
    /// against it. The catalog pins the actor's declaration package and its
    /// owning deployment directly: the caller's own deployment entry is
    /// preferred (the caller's request already runs in that deployment's
    /// service database), with a cross-package fallback to the first catalog
    /// entry for the actor's ABI.
    fn actor_owner_service_id(
        &self,
        header: &ActorGetOrCreateRequestFrameHeader,
    ) -> Result<String, &'static str> {
        match &header.declaration_owner.unit {
            ActorOwnerUnitFrameHeader::Service => Ok(header.actor_key.service_id.clone()),
            ActorOwnerUnitFrameHeader::Package(_slot) => {
                let actor = ActorRoutingRef {
                    service_id: header.actor_key.service_id.clone(),
                    actor_abi_identity: ActorAbiIdentity::new(header.actor_abi_identity.clone()),
                };
                let catalog = self
                    .components
                    .catalog_view
                    .catalog_snapshot()
                    .ok_or("ActorOwnerServiceUnresolved")?;
                let owning = catalog
                    .methods_for_actor(&actor)
                    .next()
                    .or_else(|| {
                        catalog.entries().iter().find(|entry| {
                            entry.actor.actor_abi_identity == actor.actor_abi_identity
                        })
                    })
                    .ok_or("ActorOwnerServiceUnresolved")?;
                Ok(owning.deployment.service_id.clone())
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
    /// owner and remove the correlation so the relay pending and this sink's
    /// map return to zero.
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
            let correlation = self
                .invocations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(invocation_id);
            if let Some(correlation) = correlation {
                self.report_task_attempt_terminal(
                    &correlation,
                    ActorAttemptTerminal::TargetFailed {
                        message: "actor method invocation deadline exceeded".to_string(),
                    },
                );
            }
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
        let task_attempt_uncertain = {
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
            let uncertain = invocations
                .iter()
                .filter(|(_, correlation)| {
                    correlation.task_attempt.is_some() && correlation.owner == *session
                })
                .map(|(invocation_id, correlation)| (invocation_id.clone(), correlation.clone()))
                .collect::<Vec<_>>();
            for (invocation_id, _correlation) in &uncertain {
                invocations.remove(invocation_id);
            }
            uncertain
        };
        for (_invocation_id, correlation) in task_attempt_uncertain {
            self.report_task_attempt_terminal(
                &correlation,
                ActorAttemptTerminal::Uncertain {
                    reason: "actor owner runtime session closed".to_string(),
                },
            );
        }
        self.invocations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_id, correlation| {
                correlation.owner != *session
                    && correlation
                        .caller
                        .as_ref()
                        .is_none_or(|caller| caller != session)
            });
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
                    route_authority: self.route_authority(&query)?,
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
                                    caller: Some(session.clone()),
                                    owner: owner.clone(),
                                    fence: fence.clone(),
                                    task_attempt: None,
                                },
                            );
                        let route = ActorOwnerRouteAuthorityFrameHeader {
                            build_id: input.route_authority.build_id,
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
                            if correlation.task_attempt.is_some() {
                                self.report_task_attempt_terminal(
                                    &correlation,
                                    ActorAttemptTerminal::Succeeded,
                                );
                                return Ok(());
                            }
                            let Some(caller) = correlation.caller else {
                                return Ok(());
                            };
                            let forward = ActorMethodFrame::Return(header, payload);
                            let bytes = encode_actor_method_frame(&forward)
                                .map_err(|_| TerminalKind::MalformedFrame)?;
                            self.write(&caller, bytes)
                        }
                        Err(_) => Ok(()),
                    }
                } else {
                    Ok(())
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
                            if let Some(task_attempt) = &correlation.task_attempt {
                                self.report_task_attempt_terminal(
                                    &correlation,
                                    Self::actor_error_terminal(
                                        header.error,
                                        &task_attempt.request_id,
                                    ),
                                );
                                return Ok(());
                            }
                            let Some(caller) = correlation.caller else {
                                return Ok(());
                            };
                            let bytes = encode_actor_method_frame(&ActorMethodFrame::Error(header))
                                .map_err(|_| TerminalKind::MalformedFrame)?;
                            self.write(&caller, bytes)
                        }
                        Err(_) => Ok(()),
                    }
                } else {
                    Ok(())
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
                    // A stray cancel has no caller correlation; it is dropped
                    // fail-closed (relay deadline owns the terminal).
                    return Ok(());
                };
                if correlation.caller.is_none() {
                    // Task-attempt invocations have no Runtime caller to
                    // cancel from; the owner cancel is owned by deadline /
                    // disconnect terminals.
                    return Ok(());
                }
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
                if let Some(task_attempt) = &correlation.task_attempt {
                    self.report_task_attempt_terminal(
                        &correlation,
                        ActorAttemptTerminal::TargetFailed {
                            message: format!(
                                "actor owner failed attempt {}: {}",
                                task_attempt.request_id, header.reason.message
                            ),
                        },
                    );
                    return Ok(());
                }
                let Some(caller) = correlation.caller else {
                    return Ok(());
                };
                let bytes =
                    encode_binary_frame(&header, &[]).map_err(|_| TerminalKind::MalformedFrame)?;
                self.write(&caller, bytes)
            }
            Err(_) => Ok(()),
        }
    }

    fn actor_error_terminal(
        error: ActorMethodErrorFramePayload,
        request_id: &str,
    ) -> ActorAttemptTerminal {
        match error {
            ActorMethodErrorFramePayload::ActorUpgradingError { retry_after_ms, .. } => {
                ActorAttemptTerminal::Upgrading { retry_after_ms }
            }
            ActorMethodErrorFramePayload::ActorVersionRejectedError {
                requested_implementation_identity,
                accepted_implementation_identity,
                ..
            } => ActorAttemptTerminal::VersionRejected {
                message: format!(
                    "ActorVersionRejectedError: task attempt {request_id} requested {} but {} is accepted",
                    requested_implementation_identity.as_str(),
                    accepted_implementation_identity.as_str()
                ),
            },
            ActorMethodErrorFramePayload::ActorIncarnationReplacedError { current_epoch, .. } => {
                ActorAttemptTerminal::VersionRejected {
                    message: format!(
                        "ActorIncarnationReplacedError: task attempt {request_id} incarnation advanced to epoch {current_epoch}"
                    ),
                }
            }
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
