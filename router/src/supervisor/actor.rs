//! W-actor production assembly: the six owners plus the spawn consumer with
//! the outbound control ports wired through the session outbound registry,
//! and the A3 catalog captured into the routing epoch view.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

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
    SpawnParentSnapshot, SpawnSubmitAcceptance, SpawnSubmitRouter, SpawnWireStore,
    DEFAULT_ACTOR_PENDING_BUDGET,
};
use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use crate::dispatch::RequestDispatcher;
use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;

use super::session_ports::SessionHandle;

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
    pub execution_sink: Arc<DeferredActorMethodSpawnExecutionSink>,
    pub actor_lane_spawn_control: Arc<ActorLaneSpawnControl>,
    /// Raw wire correlation for accepted actor-method spawns
    /// (E-actor-rust; M-spawn-repair `SpawnSubmitAcceptance` data surface).
    pub spawn_wire_store: Arc<SpawnWireStore>,
    /// `eviction_request_id -> actor key` registered by the idle-evict port
    /// and consumed by the actor frame sink on the ACK.
    pub idle_evictions: Arc<Mutex<HashMap<String, ActorLogicalKey>>>,
    /// Deferred dispatcher reference (assembled after the actor lane; the
    /// function-spawn parent lookup answers through it).
    pub deferred_dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
}

/// Execution sink installed before the `ActorFrameSink` exists. The
/// composition sets the real sink once the session/actor sink is assembled;
/// accepts before then fail closed (no silently dropped spawn).
#[derive(Debug, Default)]
pub struct DeferredActorMethodSpawnExecutionSink {
    inner: Mutex<Option<Arc<dyn ActorMethodSpawnExecutionSink>>>,
    uninstalled_accepts: AtomicU64,
}

impl DeferredActorMethodSpawnExecutionSink {
    pub fn set(&self, sink: Arc<dyn ActorMethodSpawnExecutionSink>) {
        *self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sink);
    }

    pub fn uninstalled_accepts(&self) -> u64 {
        self.uninstalled_accepts.load(Ordering::Relaxed)
    }
}

impl ActorMethodSpawnExecutionSink for DeferredActorMethodSpawnExecutionSink {
    fn on_accept(&self, acceptance: &SpawnSubmitAcceptance) {
        match self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            Some(sink) => sink.on_accept(acceptance),
            None => {
                self.uninstalled_accepts.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// `callerKind=request` parent lookup backed by the `RequestDispatcher`
/// pending (C-dispatch §5.1). The dispatcher owns request pending truth; this
/// adapter only reads its public fenced snapshot ports.
#[derive(Debug, Clone)]
pub struct DispatcherSpawnParentLookup {
    dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>,
}

impl DispatcherSpawnParentLookup {
    pub fn new(dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>>) -> Self {
        Self { dispatcher }
    }
}

impl SpawnParentLookup for DispatcherSpawnParentLookup {
    fn find_parent(&self, caller_request_id: &str) -> Option<SpawnParentSnapshot> {
        let dispatcher = self
            .dispatcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()?;
        let (epoch, session) = match dispatcher.spawn_parent_facts(caller_request_id) {
            Some(facts) => facts,
            None => {
                let epoch = dispatcher.pending_epoch(caller_request_id)?;
                let lease = dispatcher.pending_lease(caller_request_id)?;
                (epoch, lease.session_epoch)
            }
        };
        Some(SpawnParentSnapshot {
            runtime_id: session.replica_id.clone(),
            connection: format!("{}#{}", session.replica_id, session.connection_generation),
            assembly_generation: epoch.assembly_generation(),
            test_case_capability: None,
            active: true,
            replaced: false,
        })
    }
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
    let spawn_wire_store = Arc::new(SpawnWireStore::new());
    let idle_evictions: Arc<Mutex<HashMap<String, ActorLogicalKey>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let deferred_dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>> =
        Arc::new(Mutex::new(None));
    let execution_sink = Arc::new(DeferredActorMethodSpawnExecutionSink::default());
    let lease_scheduler = Arc::new(ActorLeaseExpiryScheduler::new(
        Arc::clone(&registry),
        Arc::new(ActorIdleEvictControlPort::with_idle_evictions(
            session.clone(),
            Arc::clone(&epoch_store),
            Arc::clone(&idle_evictions),
        )),
        LeaseSchedulerOptions::default(),
    ));
    let spawn_router = Arc::new(SpawnSubmitRouter::new(
        Arc::new(FunctionSpawnParentResolver::new(Arc::new(
            DispatcherSpawnParentLookup::new(Arc::clone(&deferred_dispatcher)),
        ))),
        Arc::new(ActorSpawnParentResolver::new(Arc::new(
            RelaySpawnParentLookup::new(Arc::clone(&relay)),
        ))),
        DEFAULT_ACTOR_PENDING_BUDGET,
    )?);
    let actor_lane_spawn_control = Arc::new(ActorLaneSpawnControl::new(
        Arc::clone(&relay),
        Arc::clone(&spawn_router),
        Arc::clone(&execution_sink) as Arc<dyn ActorMethodSpawnExecutionSink>,
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
        spawn_wire_store,
        idle_evictions,
        deferred_dispatcher,
    }))
}

/// `ActivationControlPort` production adapter: builds the canonical
/// `actor.owner.control` activateInitial frame from the broker request and
/// writes it to the exact owner runtime session through the session
/// outbound registry.
///
/// E-actor-parity: the wire `ownerLeaseId` is the broker-minted lease id
/// carried on the control request facts (identical to the committed registry
/// fence lease id; single mint per activation).
#[derive(Debug, Clone)]
pub struct ActorActivationControlPort {
    session: SessionHandle,
    registry: Arc<ActorOwnershipRegistry>,
}

impl ActorActivationControlPort {
    pub fn new(session: SessionHandle, registry: Arc<ActorOwnershipRegistry>) -> Self {
        Self { session, registry }
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
            owner_lease_id: request.facts.owner_lease_id.clone(),
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
    idle_evictions: Arc<Mutex<HashMap<String, ActorLogicalKey>>>,
}

impl ActorIdleEvictControlPort {
    pub fn new(session: SessionHandle, epoch_store: Arc<ActiveRoutingEpochStore>) -> Self {
        Self::with_idle_evictions(session, epoch_store, Arc::new(Mutex::new(HashMap::new())))
    }

    /// E-actor-rust: registers every sent idle-eviction so the actor frame
    /// sink can correlate the ACK to the exact actor key.
    pub fn with_idle_evictions(
        session: SessionHandle,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        idle_evictions: Arc<Mutex<HashMap<String, ActorLogicalKey>>>,
    ) -> Self {
        Self {
            session,
            epoch_store,
            idle_evictions,
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
        self.idle_evictions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(eviction_request_id.to_string(), key.clone());
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

/// Session-keyed actor lane cleanup (C-session §5.1 `ActorSessionOwner`):
/// runtime disconnect/replacement resolves every actor owner pending and
/// releases exact owner fences so invocation/control/lease/timer occupancy
/// returns to zero (E-actor-rust).
#[derive(Debug, Clone)]
pub struct ActorSessionOwnerConsumer {
    components: Arc<ActorComponents>,
    sink: Arc<Mutex<Option<Arc<super::actor_sink::ActorFrameSink>>>>,
}

impl ActorSessionOwnerConsumer {
    pub fn new(components: Arc<ActorComponents>) -> Self {
        Self {
            components,
            sink: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_sink(&self, sink: Arc<super::actor_sink::ActorFrameSink>) {
        *self
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sink);
    }
}

impl SessionConsumer for ActorSessionOwnerConsumer {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::ActorSessionOwner
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        let connection = format!("{}#{}", session.replica_id, session.connection_generation);
        if let Some(sink) = self
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            sink.on_runtime_session_closed(session);
        }
        // Caller-side invocation cancel/terminal (exact connection).
        let _ = self.components.relay.on_caller_disconnect(&connection);
        // Owner-side invocation/control/activation terminal.
        let _ = self
            .components
            .relay
            .on_owner_disconnect(&session.replica_id, &connection);
        let _ = self
            .components
            .control_broker
            .on_owner_disconnect(&session.replica_id, &connection);
        self.components
            .activation_broker
            .on_owner_disconnect(&session.replica_id, &connection);
        // Exact owner fence release + scheduler bookkeeping.
        let keys = self.components.registry.owned_keys();
        for key in keys {
            let Some(fence) = self.components.registry.current_owner(&key) else {
                continue;
            };
            // The registry fence does not retain the claim-time connection
            // token; a closed session releases every fence of its replica so
            // a replacement connection re-activates through the normal claim
            // path (fail closed, never a stale owner).
            if fence.owner_runtime_id == session.replica_id
                && self
                    .components
                    .registry
                    .release(&key, &fence, crate::actor::OwnerReleaseReason::Disconnected)
                    .is_ok()
            {
                self.components.lease_scheduler.forget(&key);
            }
        }
        Ok(())
    }
}

/// E-actor-rust timer pump: one tokio task owning the actor lane deadline
/// sweeps (C-actor §6/§8). Every tick expires activation claims, owner-control
/// ACK deadlines and invocation deadlines, and sweeps lease/idle evictions.
/// Outcome frame writes (waiter errors / owner cancels) go through the sink.
pub fn spawn_actor_lane_timer_pump(
    components: Arc<ActorComponents>,
    sink: Arc<super::actor_sink::ActorFrameSink>,
    interval: Duration,
    now: impl Fn() -> u64 + Send + Sync + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            actor_lane_timer_tick(&components, &sink, now());
        }
    })
}

fn actor_lane_timer_tick(
    components: &Arc<ActorComponents>,
    sink: &Arc<super::actor_sink::ActorFrameSink>,
    now_ms: u64,
) {
    components.lease_scheduler.sweep(now_ms);
    let _ = components.control_broker.expire_deadlines(now_ms);
    for outcome in components.activation_broker.expire_deadlines(now_ms) {
        sink.resolve_activation_timeout(&outcome);
    }
    for (cancel, _terminal) in components.relay.expire_deadlines(now_ms) {
        sink.on_relay_deadline(&cancel.invocation_id, &cancel.correlation);
    }
}
