//! W-actor production assembly: the six owners plus the spawn consumer with
//! the outbound control ports wired through the session outbound registry,
//! and the A3 catalog captured into the routing epoch view.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::Engine;
use skiff_runtime_transport::actor_owner::{
    encode_actor_owner_control_frame, ActorActivationBootstrapFrameHeader,
    ActorOwnerControlFenceFrameHeader, ActorOwnerControlFrameHeader, ActorOwnerControlOperation,
    ActorOwnerRouteAuthorityFrameHeader, ACTOR_BOOTSTRAP_ENCODING_V1,
    ACTOR_OWNER_CONTROL_FRAME_TYPE,
};
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;

use crate::actor::ActorLogicalKey;
use crate::actor::{
    ActivateInitialControlRequest, ActivationControlPort, ActorActivationBrokerOptions,
    ActorActivationRequestBroker, ActorInvocationRelay, ActorInvocationRelayOptions,
    ActorLaneSpawnControl, ActorLeaseExpiryScheduler, ActorMethodCatalogView,
    ActorMethodSpawnExecutionSink, ActorOwnerControlBroker, ActorOwnershipRegistry,
    ActorSpawnParentResolver, ControlBrokerOptions, FunctionSpawnParentResolver,
    IdleEvictControlPort, LeaseSchedulerOptions, RelaySpawnParentLookup, SpawnParentLookup,
    SpawnParentSnapshot, SpawnSubmitAcceptance, SpawnSubmitRouter, DEFAULT_ACTOR_PENDING_BUDGET,
};
use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};

use super::session_ports::{LeaseIdMint, SessionHandle};

/// Fully assembled W-actor lane (composition owner; E-actor-rust adds the
/// inbound actor frame sink and the real spawn execution owner).
#[derive(Debug)]
pub struct ActorComponents {
    pub registry: Arc<ActorOwnershipRegistry>,
    pub activation_broker: Arc<ActorActivationRequestBroker>,
    pub relay: Arc<ActorInvocationRelay>,
    pub control_broker: Arc<ActorOwnerControlBroker>,
    pub lease_scheduler: Arc<ActorLeaseExpiryScheduler>,
    pub catalog_view: Arc<ActorMethodCatalogView>,
    pub spawn_router: Arc<SpawnSubmitRouter>,
    pub execution_sink: Arc<dyn ActorMethodSpawnExecutionSink>,
    pub actor_lane_spawn_control: Arc<ActorLaneSpawnControl>,
}

/// Assembles the actor lane against one captured routing epoch and the
/// session outbound registry.
pub fn assemble_actor_components(
    epoch: Arc<RoutingEpoch>,
    epoch_store: Arc<ActiveRoutingEpochStore>,
    session: SessionHandle,
) -> Result<Arc<ActorComponents>, String> {
    let registry = Arc::new(ActorOwnershipRegistry::new());
    let relay = Arc::new(ActorInvocationRelay::new(
        ActorInvocationRelayOptions::default(),
    ));
    let control_broker = Arc::new(ActorOwnerControlBroker::new(ControlBrokerOptions::default()));
    let catalog_view = Arc::new(ActorMethodCatalogView::new(Arc::clone(&epoch)));
    let lease_scheduler = Arc::new(ActorLeaseExpiryScheduler::new(
        Arc::clone(&registry),
        Arc::new(ActorIdleEvictControlPort::new(
            session.clone(),
            Arc::clone(&epoch_store),
        )),
        LeaseSchedulerOptions::default(),
    ));
    let execution_sink: Arc<dyn ActorMethodSpawnExecutionSink> =
        Arc::new(RecordingActorMethodSpawnExecutionSink::default());
    let spawn_router = Arc::new(SpawnSubmitRouter::new(
        Arc::new(FunctionSpawnParentResolver::new(Arc::new(
            UnavailableSpawnParentLookup,
        ))),
        Arc::new(ActorSpawnParentResolver::new(Arc::new(
            RelaySpawnParentLookup::new(Arc::clone(&relay)),
        ))),
        DEFAULT_ACTOR_PENDING_BUDGET,
    )?);
    let actor_lane_spawn_control = Arc::new(ActorLaneSpawnControl::new(
        Arc::clone(&relay),
        Arc::clone(&spawn_router),
        Arc::clone(&execution_sink),
    ));
    let activation_broker = Arc::new(ActorActivationRequestBroker::new(
        Arc::clone(&registry),
        Arc::new(ActorActivationControlPort::new(
            session,
            Arc::clone(&registry),
        )),
        ActorActivationBrokerOptions::default(),
    ));
    Ok(Arc::new(ActorComponents {
        registry,
        activation_broker,
        relay,
        control_broker,
        lease_scheduler,
        catalog_view,
        spawn_router,
        execution_sink,
        actor_lane_spawn_control,
    }))
}

/// `ActivationControlPort` production adapter: builds the canonical
/// `actor.owner.control` activateInitial frame from the broker request and
/// writes it to the exact owner runtime session through the session
/// outbound registry.
///
/// Alignment seams (documented for E-actor-rust/E-actor-parity): the fence
/// epoch resolves from the ownership registry entry at activation time, and
/// the wire `ownerLeaseId` is minted composition-side (the canonical corpus
/// mints it at activation; the Rust registry independently mints the
/// committed fence lease id at commit).
#[derive(Debug, Clone)]
pub struct ActorActivationControlPort {
    session: SessionHandle,
    registry: Arc<ActorOwnershipRegistry>,
    lease_mint: LeaseIdMint,
}

impl ActorActivationControlPort {
    pub fn new(session: SessionHandle, registry: Arc<ActorOwnershipRegistry>) -> Self {
        Self {
            session,
            registry,
            lease_mint: LeaseIdMint::new(),
        }
    }
}

impl ActivationControlPort for ActorActivationControlPort {
    fn send_activate_initial(&self, request: &ActivateInitialControlRequest) -> Result<(), String> {
        let epoch = self
            .registry
            .entry_epoch(&request.actor_key)
            .ok_or_else(|| "actor entry epoch is unavailable for activateInitial".to_string())?;
        let fence = ActorOwnerControlFenceFrameHeader {
            service_id: request.actor_key.service_id.clone(),
            actor_type_identity: request.actor_key.actor_type_identity.clone(),
            actor_id_type_identity: request.actor_key.actor_id_type_identity.clone(),
            actor_id_encoding_version: request.actor_key.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: request
                .actor_key
                .canonical_actor_id_key_bytes_base64
                .clone(),
            actor_id_hash: request.actor_key.actor_id_hash.clone(),
            epoch,
            actor_abi_identity: request.facts.actor_abi_identity.clone(),
            actor_implementation_identity: request.facts.actor_implementation_identity.clone(),
            declaration_owner: request.facts.declaration_owner.clone(),
            owner_lease_id: self.lease_mint.mint(),
            eviction_request_id: None,
        };
        let header = ActorOwnerControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.to_string(),
            target_runtime_id: request.owner_runtime_id.clone(),
            request_id: request.request_id.clone(),
            operation: ActorOwnerControlOperation::ActivateInitial,
            fence,
            route_authority: ActorOwnerRouteAuthorityFrameHeader {
                assembly_identity: request.route_authority.assembly_identity.clone(),
                assembly_generation: request.route_authority.assembly_generation,
            },
            transition: None,
            bootstrap: Some(ActorActivationBootstrapFrameHeader {
                encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                payload_base64: base64::engine::general_purpose::STANDARD
                    .encode(&request.bootstrap_bytes),
            }),
            deadline: Some(request.deadline.clone()),
            test_case_capability: request.test_case_capability.clone(),
            test_case_parent_request_id: request.test_case_parent_request_id.clone(),
        };
        let bytes = encode_actor_owner_control_frame(&header)
            .map_err(|error| format!("activateInitial encode failed: {error}"))?;
        let layer = self
            .session
            .layer()
            .ok_or_else(|| "session layer is not wired yet".to_string())?;
        let runtime = layer
            .current_session_by_replica(&request.owner_runtime_id)
            .ok_or_else(|| {
                format!(
                    "owner runtime {} has no registered session",
                    request.owner_runtime_id
                )
            })?;
        layer.write_session_frame(&runtime, bytes)
    }
}

/// `IdleEvictControlPort` production adapter: builds the canonical IdleEvict
/// control frame (fence + eviction request id + captured epoch route
/// authority) and writes it to the exact owner runtime session.
#[derive(Debug, Clone)]
pub struct ActorIdleEvictControlPort {
    session: SessionHandle,
    epoch_store: Arc<ActiveRoutingEpochStore>,
}

impl ActorIdleEvictControlPort {
    pub fn new(session: SessionHandle, epoch_store: Arc<ActiveRoutingEpochStore>) -> Self {
        Self {
            session,
            epoch_store,
        }
    }
}

impl IdleEvictControlPort for ActorIdleEvictControlPort {
    fn send_idle_evict(
        &self,
        key: &ActorLogicalKey,
        fence: &crate::actor::ActorOwnerFence,
        eviction_request_id: &str,
        _connection: &str,
    ) -> Result<(), String> {
        let epoch = self
            .epoch_store
            .capture()
            .ok_or_else(|| "no active routing epoch for idleEvict".to_string())?;
        let fence_header = ActorOwnerControlFenceFrameHeader {
            service_id: key.service_id.clone(),
            actor_type_identity: key.actor_type_identity.clone(),
            actor_id_type_identity: key.actor_id_type_identity.clone(),
            actor_id_encoding_version: key.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64.clone(),
            actor_id_hash: key.actor_id_hash.clone(),
            epoch: fence.epoch,
            actor_abi_identity: fence.actor_abi_identity.clone(),
            actor_implementation_identity: fence.actor_implementation_identity.clone(),
            declaration_owner: fence.declaration_owner.clone(),
            owner_lease_id: fence.owner_lease_id.clone(),
            eviction_request_id: Some(eviction_request_id.to_string()),
        };
        let header = ActorOwnerControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.to_string(),
            target_runtime_id: fence.owner_runtime_id.clone(),
            request_id: format!("control:idle-evict-{eviction_request_id}"),
            operation: ActorOwnerControlOperation::IdleEvict,
            fence: fence_header,
            route_authority: ActorOwnerRouteAuthorityFrameHeader {
                assembly_identity: epoch.assembly_identity().to_string(),
                assembly_generation: epoch.assembly_generation(),
            },
            transition: None,
            bootstrap: None,
            deadline: None,
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        let bytes = encode_actor_owner_control_frame(&header)
            .map_err(|error| format!("idleEvict encode failed: {error}"))?;
        let layer = self
            .session
            .layer()
            .ok_or_else(|| "session layer is not wired yet".to_string())?;
        let runtime = layer
            .current_session_by_replica(&fence.owner_runtime_id)
            .ok_or_else(|| {
                format!(
                    "owner runtime {} has no registered session",
                    fence.owner_runtime_id
                )
            })?;
        layer.write_session_frame(&runtime, bytes)
    }
}

/// Spawn execution sink placeholder: records accepted actor-method spawns.
/// The real execution owner is wired by E-actor-rust; the acceptance is
/// already separated from the parent lifecycle (C-spawn §3.3).
#[derive(Debug, Default)]
pub struct RecordingActorMethodSpawnExecutionSink {
    accepted: AtomicU64,
}

impl RecordingActorMethodSpawnExecutionSink {
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }
}

impl ActorMethodSpawnExecutionSink for RecordingActorMethodSpawnExecutionSink {
    fn on_accept(&self, _acceptance: &SpawnSubmitAcceptance) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }
}

/// Function-spawn parent lookup is fail-closed until E-actor-rust wires the
/// dispatcher pending view into `SpawnParentLookup`.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableSpawnParentLookup;

impl SpawnParentLookup for UnavailableSpawnParentLookup {
    fn find_parent(&self, _caller_request_id: &str) -> Option<SpawnParentSnapshot> {
        None
    }
}
