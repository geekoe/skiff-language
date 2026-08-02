//! `WebSocketRequestBroker`: peer request/response correlation, deadline,
//! tombstone, captured writer fence and late result isolation
//! (C-ws §4, authority design §3.2).
//!
//! Peer correlation lives only here; ordinary `RequestDispatcher` pending is
//! never shared. The broker is a pure synchronous reducer: timers are fired
//! through explicit `fire_*` methods by the lane, and inbound dispatcher
//! terminals arrive through [`WebSocketRequestBroker::complete_inbound`]
//! carrying the exact `InboundExecutionToken`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::connection_protocol::{
    ClientSocketGeneration, OpaquePeerId, ProfileAction, WebSocketRpcProfile,
};
use skiff_runtime_transport::protocol::RuntimeDeadlineFrameHeader;

use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;

use super::profile::{JsonRpc20TextProfile, PlatformErrorKind};
use super::types::{
    BrokerConnectionGeneration, BrokerRuntimeResponse, BrokerRuntimeSource, Clock, DispatchInbound,
    InboundDispatchAction, InboundDispatchResult, InboundExecutionToken, MethodCatalog,
    NotificationObserver, OwnerToken, PeerWriter, RuntimeViolationSink, SystemClock,
    WebSocketLifecycleClose, CLOSE_BINARY_FRAME, CLOSE_TRANSPORT_ERROR,
};

#[derive(Debug, Clone)]
pub struct WebSocketRequestBrokerOptions {
    pub outbound_global_capacity: usize,
    pub outbound_per_generation_capacity: usize,
    pub inbound_global_capacity: usize,
    pub inbound_per_generation_capacity: usize,
    pub tombstone_capacity: usize,
    pub tombstone_ttl_ms: u64,
    pub inbound_timeout_ms: u64,
}

impl Default for WebSocketRequestBrokerOptions {
    fn default() -> Self {
        Self {
            outbound_global_capacity: super::types::OUTBOUND_GLOBAL_CAPACITY_DEFAULT,
            outbound_per_generation_capacity:
                super::types::OUTBOUND_PER_GENERATION_CAPACITY_DEFAULT,
            inbound_global_capacity: super::types::INBOUND_GLOBAL_CAPACITY_DEFAULT,
            inbound_per_generation_capacity: super::types::INBOUND_PER_GENERATION_CAPACITY_DEFAULT,
            tombstone_capacity: super::types::TOMBSTONE_CAPACITY_DEFAULT,
            tombstone_ttl_ms: super::types::TOMBSTONE_TTL_MS_DEFAULT,
            inbound_timeout_ms: super::types::INBOUND_TIMEOUT_MS_DEFAULT,
        }
    }
}

/// Runtime-initiated `connection.request` (C-model-connection §3.1), already
/// codec-validated by the shared transport.
#[derive(Debug, Clone)]
pub struct RuntimeRequest {
    pub request_id: String,
    pub service_id: String,
    pub websocket_entry_id: String,
    pub owner_token: u64,
    pub profile: WebSocketRpcProfile,
    pub method: String,
    pub payload: Vec<u8>,
    pub deadline: Option<RuntimeDeadlineFrameHeader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRequestOutcome {
    Success,
    ConnectionUnavailable,
    ResourceLimit,
    ProtocolError,
    TransportUnavailable,
}

/// Outcome of one peer text/binary frame; `Close` must be routed into the
/// connection finalizer by the lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerTextOutcome {
    Ok,
    Close(WebSocketLifecycleClose),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundCompletionOutcome {
    Completed,
    IgnoredLate,
    Close(WebSocketLifecycleClose),
}

#[derive(Debug, Clone, Default)]
pub struct BrokerHealthSnapshot {
    pub generation_count: usize,
    pub outbound_pending: usize,
    pub inbound_pending: usize,
    pub outbound_tombstones: usize,
    pub inbound_tombstones: usize,
    pub timer_count: usize,
    pub protocol_violations: usize,
    pub runtime_disconnect_detached: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeKey {
    sender: RuntimeSessionEpoch,
    session_token: String,
    request_id: String,
}

#[derive(Debug)]
struct GenerationState {
    handle: BrokerConnectionGeneration,
    generation: u64,
    uid: u64,
    owner_token: u64,
    writer: Arc<dyn PeerWriter>,
    open: bool,
    outbound_active: usize,
    inbound_active: usize,
    sequence: u64,
    execution_sequence: u64,
}

#[derive(Debug, Clone)]
struct OutboundEntry {
    connection_id: String,
    peer_key: String,
    runtime_key: RuntimeKey,
    source: BrokerRuntimeSource,
    request_id: String,
    deadline_at_ms: Option<u64>,
    generation_uid: u64,
}

#[derive(Debug, Clone)]
struct InboundEntry {
    peer_key: String,
    peer_id: OpaquePeerId,
    execution_token: InboundExecutionToken,
    cancel_tx: tokio::sync::watch::Sender<bool>,
    deadline_at_ms: u64,
    generation_uid: u64,
}

#[derive(Debug)]
struct TombstoneEntry {
    key: String,
    generation_uid: u64,
    created_at_ms: u64,
}

#[derive(Debug)]
struct TombstoneStore {
    capacity: usize,
    ttl_ms: u64,
    entries: VecDeque<TombstoneEntry>,
}

impl TombstoneStore {
    fn new(capacity: usize, ttl_ms: u64) -> Self {
        Self {
            capacity,
            ttl_ms,
            entries: VecDeque::new(),
        }
    }

    fn add(&mut self, key: String, generation_uid: u64, now_ms: u64) {
        self.entries.push_back(TombstoneEntry {
            key,
            generation_uid,
            created_at_ms: now_ms,
        });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    fn contains(&self, key: &str, now_ms: u64) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.key == key && !self.expired(entry, now_ms))
    }

    fn remove_generation(&mut self, generation_uid: u64) {
        self.entries
            .retain(|entry| entry.generation_uid != generation_uid);
    }

    fn sweep(&mut self, now_ms: u64) {
        while self
            .entries
            .front()
            .is_some_and(|entry| self.expired(entry, now_ms))
        {
            self.entries.pop_front();
        }
    }

    fn live_len(&self, now_ms: u64) -> usize {
        self.entries
            .iter()
            .filter(|entry| !self.expired(entry, now_ms))
            .count()
    }

    fn expired(&self, entry: &TombstoneEntry, now_ms: u64) -> bool {
        now_ms.saturating_sub(entry.created_at_ms) >= self.ttl_ms
    }
}

#[derive(Debug)]
struct BrokerInner {
    generations: HashMap<String, GenerationState>,
    outbound_by_peer: HashMap<String, OutboundEntry>,
    outbound_by_runtime: HashMap<RuntimeKey, String>,
    outbound_by_generation_request: HashMap<(String, String), String>,
    inbound_by_peer: HashMap<String, InboundEntry>,
    inbound_by_token: HashMap<InboundExecutionToken, String>,
    outbound_tombstones: TombstoneStore,
    inbound_tombstones: TombstoneStore,
    next_generation_uid: u64,
    next_owner_token: u64,
    protocol_violations: usize,
    runtime_disconnect_detached: u64,
}

impl BrokerInner {
    fn new(options: &WebSocketRequestBrokerOptions) -> Self {
        Self {
            generations: HashMap::new(),
            outbound_by_peer: HashMap::new(),
            outbound_by_runtime: HashMap::new(),
            outbound_by_generation_request: HashMap::new(),
            inbound_by_peer: HashMap::new(),
            inbound_by_token: HashMap::new(),
            outbound_tombstones: TombstoneStore::new(
                options.tombstone_capacity,
                options.tombstone_ttl_ms,
            ),
            inbound_tombstones: TombstoneStore::new(
                options.tombstone_capacity,
                options.tombstone_ttl_ms,
            ),
            next_generation_uid: 0,
            next_owner_token: 1,
            protocol_violations: 0,
            runtime_disconnect_detached: 0,
        }
    }
}

/// Unique owner of peer request/response correlation and generation
/// attachments (C-ws §4, authority design §3.2).
#[derive(Debug)]
pub struct WebSocketRequestBroker {
    inner: Mutex<BrokerInner>,
    profile: JsonRpc20TextProfile,
    methods: Arc<dyn MethodCatalog>,
    notifications: Arc<dyn NotificationObserver>,
    violations: Arc<dyn RuntimeViolationSink>,
    dispatch: Arc<dyn DispatchInbound>,
    clock: Arc<dyn Clock>,
    options: WebSocketRequestBrokerOptions,
}

impl WebSocketRequestBroker {
    pub fn new(
        methods: Arc<dyn MethodCatalog>,
        notifications: Arc<dyn NotificationObserver>,
        violations: Arc<dyn RuntimeViolationSink>,
        dispatch: Arc<dyn DispatchInbound>,
        options: WebSocketRequestBrokerOptions,
    ) -> Self {
        Self::with_clock(
            methods,
            notifications,
            violations,
            dispatch,
            options,
            Arc::new(SystemClock),
        )
    }

    pub fn with_clock(
        methods: Arc<dyn MethodCatalog>,
        notifications: Arc<dyn NotificationObserver>,
        violations: Arc<dyn RuntimeViolationSink>,
        dispatch: Arc<dyn DispatchInbound>,
        options: WebSocketRequestBrokerOptions,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Mutex::new(BrokerInner::new(&options)),
            profile: JsonRpc20TextProfile::default(),
            methods,
            notifications,
            violations,
            dispatch,
            clock,
            options,
        }
    }

    /// Attaches one broker generation with its captured writer
    /// (C-ws §4.1). Returns the owner token for Runtime `connection.request`
    /// fencing.
    pub fn attach_generation(
        &self,
        handle: BrokerConnectionGeneration,
        writer: Arc<dyn PeerWriter>,
        generation: u64,
    ) -> Result<OwnerToken, String> {
        let mut inner = self.lock();
        if inner.generations.contains_key(&handle.connection_id) {
            return Err(format!(
                "broker generation for {} is already attached",
                handle.connection_id
            ));
        }
        let uid = inner.next_generation_uid;
        inner.next_generation_uid += 1;
        let owner_token = inner.next_owner_token;
        inner.next_owner_token += 1;
        inner.generations.insert(
            handle.connection_id.clone(),
            GenerationState {
                handle,
                generation,
                uid,
                owner_token,
                writer,
                open: true,
                outbound_active: 0,
                inbound_active: 0,
                sequence: 0,
                execution_sequence: 0,
            },
        );
        Ok(OwnerToken(owner_token))
    }

    pub fn owner_token(&self, connection_id: &str) -> Option<OwnerToken> {
        self.lock()
            .generations
            .get(connection_id)
            .map(|generation| OwnerToken(generation.owner_token))
    }

    /// Runtime `connection.request` (C-ws §4.2).
    pub fn handle_runtime_request(
        &self,
        connection_id: &str,
        source: &BrokerRuntimeSource,
        request: &RuntimeRequest,
    ) -> RuntimeRequestOutcome {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        let Some(state) = inner.generations.get(connection_id) else {
            self.respond(source, &connection_unavailable(source, request));
            return RuntimeRequestOutcome::ConnectionUnavailable;
        };
        if !state.open {
            self.respond(source, &connection_unavailable(source, request));
            return RuntimeRequestOutcome::ConnectionUnavailable;
        }
        let service_id = state.handle.service_id.clone();
        let websocket_entry_id = state.handle.websocket_entry_id.clone();
        let owner_token = state.owner_token;
        let profile = state.handle.profile;
        let outbound_active = state.outbound_active;
        if request.service_id != service_id
            || request.websocket_entry_id != websocket_entry_id
            || request.owner_token != owner_token
        {
            self.respond(source, &connection_unavailable(source, request));
            return RuntimeRequestOutcome::ConnectionUnavailable;
        }
        if request.request_id.is_empty() || request.method.is_empty() || request.profile != profile
        {
            self.respond(source, &protocol_error(source, request));
            self.violations
                .on_violation(source, "invalid connection.request broker metadata");
            inner.protocol_violations += 1;
            return RuntimeRequestOutcome::ProtocolError;
        }
        if request
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            self.respond(source, &protocol_error(source, request));
            self.violations
                .on_violation(source, "invalid connection.request deadline");
            inner.protocol_violations += 1;
            return RuntimeRequestOutcome::ProtocolError;
        }
        let runtime_key = RuntimeKey {
            sender: source.sender.clone(),
            session_token: source.session_token.clone(),
            request_id: request.request_id.clone(),
        };
        if inner.outbound_by_runtime.contains_key(&runtime_key) {
            self.respond(source, &protocol_error(source, request));
            self.violations
                .on_violation(source, "duplicate active connection.request correlation");
            inner.protocol_violations += 1;
            return RuntimeRequestOutcome::ProtocolError;
        }
        if inner.outbound_by_peer.len() >= self.options.outbound_global_capacity
            || outbound_active >= self.options.outbound_per_generation_capacity
        {
            self.respond(source, &resource_limit(source, request));
            return RuntimeRequestOutcome::ResourceLimit;
        }
        let params = match self.profile.materialize_outbound_params(&request.payload) {
            Ok(params) => params,
            Err(_) => {
                self.respond(source, &resource_limit(source, request));
                return RuntimeRequestOutcome::ResourceLimit;
            }
        };
        let (peer_id, generation_uid) = {
            let state = inner
                .generations
                .get_mut(connection_id)
                .expect("generation");
            let peer_id = OpaquePeerId::String(format!(
                "{}:{}",
                state.handle.socket_generation, state.sequence
            ));
            state.sequence += 1;
            (peer_id, state.uid)
        };
        let peer_key = generation_peer_key(generation_uid, &peer_id);
        if inner.outbound_by_peer.contains_key(&peer_key)
            || inner.outbound_tombstones.contains(&peer_key, now)
        {
            self.respond(source, &protocol_error(source, request));
            self.violations
                .on_violation(source, "outbound peer id generator reused an id");
            inner.protocol_violations += 1;
            return RuntimeRequestOutcome::ProtocolError;
        }
        let deadline_at_ms = request
            .deadline
            .as_ref()
            .map(|deadline| now.saturating_add(deadline.timeout_ms));
        let entry = OutboundEntry {
            connection_id: connection_id.to_string(),
            peer_key: peer_key.clone(),
            runtime_key: runtime_key.clone(),
            source: source.clone(),
            request_id: request.request_id.clone(),
            deadline_at_ms,
            generation_uid,
        };
        inner.outbound_by_peer.insert(peer_key.clone(), entry);
        inner
            .outbound_by_runtime
            .insert(runtime_key, peer_key.clone());
        inner.outbound_by_generation_request.insert(
            (connection_id.to_string(), request.request_id.clone()),
            peer_key.clone(),
        );
        let generation = inner
            .generations
            .get_mut(connection_id)
            .expect("generation");
        generation.outbound_active += 1;
        let frame = self
            .profile
            .encode_outbound_request(&peer_id, &request.method, &params);
        match frame.and_then(|frame| {
            inner
                .generations
                .get(connection_id)
                .expect("generation")
                .writer
                .write_text(frame)
        }) {
            Ok(()) => RuntimeRequestOutcome::Success,
            Err(_) => {
                let entry = inner
                    .outbound_by_peer
                    .get(&peer_key)
                    .cloned()
                    .expect("outbound entry");
                self.detach_outbound(&mut inner, &entry);
                self.respond(
                    &entry.source,
                    &transport_unavailable(&entry.source, &entry.request_id),
                );
                RuntimeRequestOutcome::TransportUnavailable
            }
        }
    }

    /// Runtime `connection.request.cancel` (C-ws §4.3): detach exact outbound,
    /// no peer write.
    pub fn handle_runtime_cancel(&self, source: &BrokerRuntimeSource, request_id: &str) -> bool {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        let runtime_key = RuntimeKey {
            sender: source.sender.clone(),
            session_token: source.session_token.clone(),
            request_id: request_id.to_string(),
        };
        let Some(peer_key) = inner.outbound_by_runtime.get(&runtime_key).cloned() else {
            return false;
        };
        let entry = inner
            .outbound_by_peer
            .get(&peer_key)
            .cloned()
            .expect("outbound owner");
        self.detach_outbound(&mut inner, &entry);
        true
    }

    /// Runtime disconnect: detach every outbound for this sender+session
    /// (C-ws §4.3), no peer write.
    pub fn handle_runtime_disconnect(&self, source: &BrokerRuntimeSource) -> usize {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        let affected = inner
            .outbound_by_peer
            .values()
            .filter(|entry| {
                entry.source.sender == source.sender
                    && entry.source.session_token == source.session_token
            })
            .cloned()
            .collect::<Vec<_>>();
        let count = affected.len();
        for entry in affected {
            self.detach_outbound(&mut inner, &entry);
        }
        inner.runtime_disconnect_detached += count as u64;
        count
    }

    /// Session-keyed cleanup for `SessionConsumer` (C-ws §4.3): detach every
    /// outbound for the exact Runtime session.
    pub fn runtime_disconnected_sender(&self, sender: &RuntimeSessionEpoch) -> usize {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        let affected = inner
            .outbound_by_peer
            .values()
            .filter(|entry| entry.source.sender == *sender)
            .cloned()
            .collect::<Vec<_>>();
        let count = affected.len();
        for entry in affected {
            self.detach_outbound(&mut inner, &entry);
        }
        inner.runtime_disconnect_detached += count as u64;
        count
    }

    /// One peer text frame (C-ws §4.5 / §5.1).
    pub fn handle_peer_text(&self, connection_id: &str, frame: &[u8]) -> PeerTextOutcome {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        if !inner.generations.contains_key(connection_id) {
            return PeerTextOutcome::Ok;
        }
        match self.profile.classify_text(frame) {
            ProfileAction::Request { id, method } => {
                self.handle_inbound_request(&mut inner, connection_id, id, method, frame)
            }
            ProfileAction::Response { id } => {
                self.handle_peer_response_inner(&mut inner, connection_id, &id, frame)
            }
            ProfileAction::Notification { method } => {
                self.notifications.observe(connection_id, &method, None);
                PeerTextOutcome::Ok
            }
            ProfileAction::PlatformError { kind } => {
                let kind = PlatformErrorKind::from(kind);
                if kind == PlatformErrorKind::InvalidParams {
                    if let Some(id) = extract_peer_id(frame) {
                        return self.predispatch_error(&mut inner, connection_id, id, kind);
                    }
                }
                self.handle_platform_error(&mut inner, connection_id, kind)
            }
            ProfileAction::Close { code } => {
                let reason = if code == 1009 {
                    "JSON-RPC text frame exceeds profile limits"
                } else {
                    "invalid JSON-RPC response"
                };
                PeerTextOutcome::Close(WebSocketLifecycleClose::new(code, reason).unwrap_or_else(
                    |_| WebSocketLifecycleClose {
                        code,
                        reason: "JSON-RPC protocol error".to_string(),
                    },
                ))
            }
        }
    }

    /// Binary peer frames are not supported (C-ws §4.5 close 1003).
    pub fn handle_peer_binary(&self, connection_id: &str) -> PeerTextOutcome {
        if self.lock().generations.contains_key(connection_id) {
            PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: CLOSE_BINARY_FRAME.0,
                reason: CLOSE_BINARY_FRAME.1.to_string(),
            })
        } else {
            PeerTextOutcome::Ok
        }
    }

    /// Peer close: detach all pending without writing a close frame
    /// (C-ws §4.5). The lane routes the returned outcome into the finalizer.
    pub fn handle_peer_disconnect(&self, connection_id: &str) -> PeerTextOutcome {
        let now = self.clock.now_ms();
        let mut inner = self.lock();
        inner.outbound_tombstones.sweep(now);
        inner.inbound_tombstones.sweep(now);
        if !inner.generations.contains_key(connection_id) {
            return PeerTextOutcome::Ok;
        }
        self.close_generation_inner(&mut inner, connection_id, false);
        PeerTextOutcome::Ok
    }

    fn handle_peer_response_inner(
        &self,
        inner: &mut BrokerInner,
        connection_id: &str,
        peer_id: &str,
        frame: &[u8],
    ) -> PeerTextOutcome {
        let Some(generation_uid) = inner.generations.get(connection_id).map(|state| state.uid)
        else {
            return PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "unknown JSON-RPC response id".to_string(),
            });
        };
        let peer_key = format!("{generation_uid}:s:{peer_id}");
        let Some(entry) = inner.outbound_by_peer.get(&peer_key).cloned() else {
            let now = self.clock.now_ms();
            if inner.outbound_tombstones.contains(&peer_key, now) {
                return PeerTextOutcome::Ok;
            }
            return PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "unknown JSON-RPC response id".to_string(),
            });
        };
        if Some(entry.generation_uid) != inner.generations.get(connection_id).map(|state| state.uid)
        {
            return PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "unknown JSON-RPC response id".to_string(),
            });
        }
        match self.handle_outbound_response(inner, &entry, frame, peer_id) {
            Ok(()) => PeerTextOutcome::Ok,
            Err(_) => PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "invalid JSON-RPC response payload".to_string(),
            }),
        }
    }

    fn handle_outbound_response(
        &self,
        inner: &mut BrokerInner,
        entry: &OutboundEntry,
        frame: &[u8],
        peer_id: &str,
    ) -> Result<(), String> {
        let terminal = self.profile.peer_response_terminal(frame, peer_id)?;
        let response = match terminal {
            super::profile::PeerResponseTerminal::Success { result } => BrokerRuntimeResponse {
                request_id: entry.request_id.clone(),
                outcome: skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::Success,
                remote: None,
                payload: result,
            },
            super::profile::PeerResponseTerminal::RemoteError { code, message, data } => {
                BrokerRuntimeResponse {
                    request_id: entry.request_id.clone(),
                    outcome: skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::Remote,
                    remote: Some(
                        skiff_runtime_transport::connection_protocol::ConnectionRemoteErrorFrameHeader {
                            code,
                            message,
                            data_present: data.is_some(),
                        },
                    ),
                    payload: data.unwrap_or_default(),
                }
            }
        };
        self.settle_outbound(inner, entry, response);
        Ok(())
    }

    fn handle_inbound_request(
        &self,
        inner: &mut BrokerInner,
        connection_id: &str,
        peer_id: OpaquePeerId,
        method: String,
        frame: &[u8],
    ) -> PeerTextOutcome {
        let now = self.clock.now_ms();
        let Some(state) = inner.generations.get(connection_id) else {
            return PeerTextOutcome::Ok;
        };
        let generation_uid = state.uid;
        let peer_key = generation_peer_key(generation_uid, &peer_id);
        if inner.inbound_by_peer.contains_key(&peer_key)
            || inner.inbound_tombstones.contains(&peer_key, now)
        {
            return PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "duplicate JSON-RPC request id".to_string(),
            });
        }
        if !state.open {
            return PeerTextOutcome::Ok;
        }
        if !self.methods.accepts(&method) {
            return self.predispatch_error(
                inner,
                connection_id,
                peer_id,
                PlatformErrorKind::MethodNotFound,
            );
        }
        if inner.inbound_by_peer.len() >= self.options.inbound_global_capacity
            || state.inbound_active >= self.options.inbound_per_generation_capacity
        {
            return self.predispatch_error(
                inner,
                connection_id,
                peer_id,
                PlatformErrorKind::ServerBusy,
            );
        }
        let params = match extract_request_params(frame) {
            Some(params) => params,
            None => {
                return self.predispatch_error(
                    inner,
                    connection_id,
                    peer_id,
                    PlatformErrorKind::InvalidParams,
                );
            }
        };
        let generation_uid = state.uid;
        let socket_generation = state.generation;
        let execution_sequence = state.execution_sequence;
        let state = inner
            .generations
            .get_mut(connection_id)
            .expect("generation");
        state.execution_sequence += 1;
        let execution_token = InboundExecutionToken {
            connection_id: connection_id.to_string(),
            socket_generation,
            sequence: execution_sequence,
        };
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        inner.inbound_by_peer.insert(
            peer_key.clone(),
            InboundEntry {
                peer_key: peer_key.clone(),
                peer_id: peer_id.clone(),
                execution_token: execution_token.clone(),
                cancel_tx,
                deadline_at_ms: now.saturating_add(self.options.inbound_timeout_ms),
                generation_uid,
            },
        );
        inner
            .inbound_by_token
            .insert(execution_token.clone(), peer_key.clone());
        let state = inner
            .generations
            .get_mut(connection_id)
            .expect("generation");
        state.inbound_active += 1;
        let action = InboundDispatchAction {
            profile: state.handle.profile,
            connection_id: connection_id.to_string(),
            socket_generation,
            peer_id: peer_id.clone(),
            method,
            params,
            execution_token: execution_token.clone(),
            cancel: cancel_rx,
        };
        if self.dispatch.dispatch(action).is_err() {
            return match self.complete_inbound_inner(
                inner,
                &execution_token,
                InboundDispatchResult::InternalError,
            ) {
                InboundCompletionOutcome::Completed | InboundCompletionOutcome::IgnoredLate => {
                    PeerTextOutcome::Ok
                }
                InboundCompletionOutcome::Close(close) => PeerTextOutcome::Close(close),
            };
        }
        PeerTextOutcome::Ok
    }

    fn predispatch_error(
        &self,
        inner: &mut BrokerInner,
        connection_id: &str,
        peer_id: OpaquePeerId,
        kind: PlatformErrorKind,
    ) -> PeerTextOutcome {
        let now = self.clock.now_ms();
        let Some(state) = inner.generations.get(connection_id) else {
            return PeerTextOutcome::Ok;
        };
        let generation_uid = state.uid;
        let peer_key = generation_peer_key(generation_uid, &peer_id);
        if inner.inbound_by_peer.contains_key(&peer_key)
            || inner.inbound_tombstones.contains(&peer_key, now)
        {
            return PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: 1002,
                reason: "duplicate JSON-RPC request id".to_string(),
            });
        }
        inner.inbound_tombstones.add(peer_key, generation_uid, now);
        let frame = self.profile.encode_platform_error(Some(&peer_id), kind);
        match frame.and_then(|frame| {
            inner
                .generations
                .get(connection_id)
                .expect("generation")
                .writer
                .write_text(frame)
        }) {
            Ok(()) => PeerTextOutcome::Ok,
            Err(_) => PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: CLOSE_TRANSPORT_ERROR.0,
                reason: CLOSE_TRANSPORT_ERROR.1.to_string(),
            }),
        }
    }

    fn handle_platform_error(
        &self,
        inner: &mut BrokerInner,
        connection_id: &str,
        kind: PlatformErrorKind,
    ) -> PeerTextOutcome {
        let frame = self.profile.encode_platform_error(None, kind);
        match frame.and_then(|frame| {
            inner
                .generations
                .get(connection_id)
                .expect("generation")
                .writer
                .write_text(frame)
        }) {
            Ok(()) => PeerTextOutcome::Ok,
            Err(_) => PeerTextOutcome::Close(WebSocketLifecycleClose {
                code: CLOSE_TRANSPORT_ERROR.0,
                reason: CLOSE_TRANSPORT_ERROR.1.to_string(),
            }),
        }
    }

    /// Dispatcher terminal (C-ws §4.4). Late completions are ignored
    /// idempotently (the execution token fence).
    pub fn complete_inbound(
        &self,
        token: &InboundExecutionToken,
        result: InboundDispatchResult,
    ) -> InboundCompletionOutcome {
        self.lock_complete_inbound(token, result)
    }

    fn lock_complete_inbound(
        &self,
        token: &InboundExecutionToken,
        result: InboundDispatchResult,
    ) -> InboundCompletionOutcome {
        let mut inner = self.lock();
        self.complete_inbound_inner(&mut inner, token, result)
    }

    fn complete_inbound_inner(
        &self,
        inner: &mut BrokerInner,
        token: &InboundExecutionToken,
        result: InboundDispatchResult,
    ) -> InboundCompletionOutcome {
        let Some(peer_key) = inner.inbound_by_token.get(token).cloned() else {
            return InboundCompletionOutcome::IgnoredLate;
        };
        let entry = inner
            .inbound_by_peer
            .get(&peer_key)
            .cloned()
            .expect("inbound entry");
        let (terminal, abort) = match result {
            InboundDispatchResult::Success { result } => {
                (self.profile.encode_result(&entry.peer_id, &result), false)
            }
            InboundDispatchResult::InvalidParams => (
                self.profile
                    .encode_platform_error(Some(&entry.peer_id), PlatformErrorKind::InvalidParams),
                false,
            ),
            InboundDispatchResult::InternalError | InboundDispatchResult::RuntimeUnavailable => (
                self.profile
                    .encode_platform_error(Some(&entry.peer_id), PlatformErrorKind::Internal),
                false,
            ),
            InboundDispatchResult::DeadlineExceeded => (
                self.profile
                    .encode_platform_error(Some(&entry.peer_id), PlatformErrorKind::Timeout),
                true,
            ),
        };
        self.detach_inbound(inner, &entry);
        if abort {
            let _ = entry.cancel_tx.send(true);
        }
        let frame = match terminal {
            Ok(frame) => frame,
            Err(_) => self
                .profile
                .encode_platform_error(Some(&entry.peer_id), PlatformErrorKind::Internal)
                .unwrap_or_default(),
        };
        let write_result = inner
            .generations
            .get(&entry.execution_token.connection_id)
            .map(|state| state.writer.write_text(frame));
        match write_result {
            Some(Ok(())) => InboundCompletionOutcome::Completed,
            _ => InboundCompletionOutcome::Close(WebSocketLifecycleClose {
                code: CLOSE_TRANSPORT_ERROR.0,
                reason: CLOSE_TRANSPORT_ERROR.1.to_string(),
            }),
        }
    }

    /// Outbound deadline (C-ws §4.2(7)); settles exactly once.
    pub fn fire_deadline(&self, connection_id: &str, request_id: &str) -> bool {
        let mut inner = self.lock();
        let Some(peer_key) = inner
            .outbound_by_generation_request
            .get(&(connection_id.to_string(), request_id.to_string()))
            .cloned()
        else {
            return false;
        };
        let entry = inner
            .outbound_by_peer
            .get(&peer_key)
            .cloned()
            .expect("outbound entry");
        let response = BrokerRuntimeResponse {
            request_id: entry.request_id.clone(),
            outcome: skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::DeadlineExceeded,
            remote: None,
            payload: Vec::new(),
        };
        self.settle_outbound(&mut inner, &entry, response);
        true
    }

    /// Inbound deadline (C-ws §4.4): timeout terminal + abort.
    pub fn fire_inbound_deadline(&self, token: &InboundExecutionToken) -> bool {
        let mut inner = self.lock();
        if !inner.inbound_by_token.contains_key(token) {
            return false;
        }
        self.complete_inbound_inner(&mut inner, token, InboundDispatchResult::DeadlineExceeded);
        true
    }

    /// Finalizer step: detach the exact generation, install bounded
    /// tombstones, abort inbound and respond outbound (C-ws §4.6). The writer
    /// close/drain is owned by the connection finalizer.
    pub fn close_generation(
        &self,
        connection_id: &str,
        _generation: &ClientSocketGeneration,
        protocol_outcome: bool,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        if !inner.generations.contains_key(connection_id) {
            return Ok(());
        }
        self.close_generation_inner(&mut inner, connection_id, protocol_outcome);
        Ok(())
    }

    fn close_generation_inner(
        &self,
        inner: &mut BrokerInner,
        connection_id: &str,
        protocol_outcome: bool,
    ) {
        let Some(state) = inner.generations.get_mut(connection_id) else {
            return;
        };
        if !state.open {
            return;
        }
        state.open = false;
        let generation_uid = state.uid;
        let outbound = inner
            .outbound_by_peer
            .values()
            .filter(|entry| entry.generation_uid == generation_uid)
            .cloned()
            .collect::<Vec<_>>();
        let inbound = inner
            .inbound_by_peer
            .values()
            .filter(|entry| entry.generation_uid == generation_uid)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &outbound {
            self.detach_outbound(inner, entry);
        }
        for entry in &inbound {
            self.detach_inbound(inner, entry);
        }
        inner.generations.remove(connection_id);
        inner.outbound_tombstones.remove_generation(generation_uid);
        inner.inbound_tombstones.remove_generation(generation_uid);
        for entry in inbound {
            let _ = entry.cancel_tx.send(true);
        }
        let outcome = if protocol_outcome {
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::ProtocolError
        } else {
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::TransportUnavailable
        };
        for entry in outbound {
            self.respond(
                &entry.source,
                &BrokerRuntimeResponse {
                    request_id: entry.request_id.clone(),
                    outcome,
                    remote: None,
                    payload: Vec::new(),
                },
            );
        }
    }

    fn detach_outbound(&self, inner: &mut BrokerInner, entry: &OutboundEntry) {
        if inner.outbound_by_peer.remove(&entry.peer_key).is_none() {
            return;
        }
        inner.outbound_by_runtime.remove(&entry.runtime_key);
        inner
            .outbound_by_generation_request
            .retain(|_, value| value != &entry.peer_key);
        if let Some(state) = inner.generations.get_mut(&entry.connection_id) {
            state.outbound_active = state.outbound_active.saturating_sub(1);
        }
        let now = self.clock.now_ms();
        inner
            .outbound_tombstones
            .add(entry.peer_key.clone(), entry.generation_uid, now);
    }

    fn detach_inbound(&self, inner: &mut BrokerInner, entry: &InboundEntry) {
        if inner.inbound_by_peer.remove(&entry.peer_key).is_none() {
            return;
        }
        inner.inbound_by_token.remove(&entry.execution_token);
        if let Some(state) = inner
            .generations
            .get_mut(&entry.execution_token.connection_id)
        {
            state.inbound_active = state.inbound_active.saturating_sub(1);
        }
        let now = self.clock.now_ms();
        inner
            .inbound_tombstones
            .add(entry.peer_key.clone(), entry.generation_uid, now);
    }

    fn settle_outbound(
        &self,
        inner: &mut BrokerInner,
        entry: &OutboundEntry,
        response: BrokerRuntimeResponse,
    ) {
        if !inner.outbound_by_peer.contains_key(&entry.peer_key) {
            return;
        }
        self.detach_outbound(inner, entry);
        self.respond(&entry.source, &response);
    }

    fn respond(&self, source: &BrokerRuntimeSource, response: &BrokerRuntimeResponse) {
        let _ = source.respond.respond(response);
    }

    pub fn snapshot(&self) -> BrokerHealthSnapshot {
        let now = self.clock.now_ms();
        let inner = self.lock();
        BrokerHealthSnapshot {
            generation_count: inner.generations.len(),
            outbound_pending: inner.outbound_by_peer.len(),
            inbound_pending: inner.inbound_by_peer.len(),
            outbound_tombstones: inner.outbound_tombstones.live_len(now),
            inbound_tombstones: inner.inbound_tombstones.live_len(now),
            timer_count: inner
                .outbound_by_peer
                .values()
                .filter(|entry| entry.deadline_at_ms.is_some())
                .count()
                + inner
                    .inbound_by_peer
                    .values()
                    .filter(|entry| entry.deadline_at_ms > 0)
                    .count(),
            protocol_violations: inner.protocol_violations,
            runtime_disconnect_detached: inner.runtime_disconnect_detached,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BrokerInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn extract_request_params(frame: &[u8]) -> Option<Vec<u8>> {
    let source = std::str::from_utf8(frame).ok()?;
    let members = top_level_members(source)?;
    let span = members
        .iter()
        .find(|(key, _)| key == "params")
        .map(|(_, span)| span.clone())?;
    Some(source[span].as_bytes().to_vec())
}

/// TS parity `peerKey` (webSocketRequestBroker.ts): peer correlation keys are
/// scoped by the socket generation uid, so identical JSON-RPC ids on
/// different socket generations never collide in the active maps or
/// tombstones.
fn generation_peer_key(generation_uid: u64, peer_id: &OpaquePeerId) -> String {
    format!("{}:{}", generation_uid, peer_id.canonical_key())
}

fn extract_peer_id(frame: &[u8]) -> Option<OpaquePeerId> {
    let source = std::str::from_utf8(frame).ok()?;
    let members = top_level_members(source)?;
    let span = members
        .iter()
        .find(|(key, _)| key == "id")
        .map(|(_, span)| span.clone())?;
    let raw = source[span].trim();
    parse_peer_id_bytes(raw.as_bytes())
}

fn parse_peer_id_bytes(raw: &[u8]) -> Option<OpaquePeerId> {
    if raw.first() == Some(&b'"') {
        let value = decode_json_string(std::str::from_utf8(raw).ok()?)?;
        if value.is_empty() || value.len() > 1024 {
            return None;
        }
        return Some(OpaquePeerId::String(value));
    }
    let lexeme = std::str::from_utf8(raw).ok()?;
    let value = super::profile::parse_safe_integer_i64(lexeme)?;
    Some(OpaquePeerId::SafeInteger(value as i128))
}

fn top_level_members(source: &str) -> Option<Vec<(String, std::ops::Range<usize>)>> {
    let bytes = source.as_bytes();
    let mut index = 0;
    skip_ws_at(bytes, &mut index);
    if bytes.get(index) != Some(&b'{') {
        return None;
    }
    index += 1;
    skip_ws_at(bytes, &mut index);
    if bytes.get(index) == Some(&b'}') {
        return Some(Vec::new());
    }
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        skip_ws_at(bytes, &mut index);
        let key_start = index;
        if !scan_string_span(bytes, &mut index) {
            return None;
        }
        let key = std::str::from_utf8(&bytes[key_start..index])
            .ok()
            .and_then(decode_json_string)?;
        if !seen.insert(key.clone()) {
            return None;
        }
        skip_ws_at(bytes, &mut index);
        if bytes.get(index) != Some(&b':') {
            return None;
        }
        index += 1;
        skip_ws_at(bytes, &mut index);
        let value_start = index;
        if !scan_value_span(bytes, &mut index) {
            return None;
        }
        members.push((key, value_start..index));
        skip_ws_at(bytes, &mut index);
        match bytes.get(index) {
            Some(b',') => {
                index += 1;
            }
            Some(b'}') => return Some(members),
            _ => return None,
        }
    }
}

fn skip_ws_at(bytes: &[u8], index: &mut usize) {
    while matches!(bytes.get(*index), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *index += 1;
    }
}

fn scan_string_span(bytes: &[u8], index: &mut usize) -> bool {
    if bytes.get(*index) != Some(&b'"') {
        return false;
    }
    *index += 1;
    loop {
        let Some(byte) = bytes.get(*index).copied() else {
            return false;
        };
        *index += 1;
        match byte {
            b'"' => return true,
            b'\\' => {
                if *index >= bytes.len() {
                    return false;
                }
                *index += 1;
            }
            byte if byte < 0x20 => return false,
            _ => {}
        }
    }
}

fn scan_value_span(bytes: &[u8], index: &mut usize) -> bool {
    skip_ws_at(bytes, index);
    let Some(byte) = bytes.get(*index).copied() else {
        return false;
    };
    match byte {
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut in_string = false;
            loop {
                let Some(current) = bytes.get(*index).copied() else {
                    return false;
                };
                *index += 1;
                if in_string {
                    match current {
                        b'\\' => {
                            if *index >= bytes.len() {
                                return false;
                            }
                            *index += 1;
                        }
                        b'"' => in_string = false,
                        _ => {}
                    }
                    continue;
                }
                match current {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        let Some(next) = depth.checked_sub(1) else {
                            return false;
                        };
                        depth = next;
                        if depth == 0 {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        b'"' => scan_string_span(bytes, index),
        b't' => consume_at(bytes, index, "true"),
        b'f' => consume_at(bytes, index, "false"),
        b'n' => consume_at(bytes, index, "null"),
        b'-' | b'0'..=b'9' => {
            let start = *index;
            while matches!(
                bytes.get(*index),
                Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
            ) {
                *index += 1;
            }
            *index > start
        }
        _ => false,
    }
}

fn consume_at(bytes: &[u8], index: &mut usize, literal: &str) -> bool {
    if bytes
        .get(*index..)
        .is_some_and(|rest| rest.starts_with(literal.as_bytes()))
    {
        *index += literal.len();
        true
    } else {
        false
    }
}

fn decode_json_string(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut index = 1;
    while index + 1 < bytes.len() {
        let byte = bytes[index];
        index += 1;
        match byte {
            b'\\' => {
                let escaped = *bytes.get(index)?;
                index += 1;
                match escaped {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = bytes.get(index..index + 4)?;
                        index += 4;
                        let codepoint =
                            u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        out.push(char::from_u32(codepoint as u32)?);
                    }
                    _ => return None,
                }
            }
            byte if byte < 0x20 => return None,
            _ => {
                let rest = &raw[index - 1..];
                let ch = rest.chars().next()?;
                out.push(ch);
                index += ch.len_utf8() - 1;
            }
        }
    }
    Some(out)
}

fn connection_unavailable(
    _source: &BrokerRuntimeSource,
    request: &RuntimeRequest,
) -> BrokerRuntimeResponse {
    BrokerRuntimeResponse {
        request_id: request.request_id.clone(),
        outcome: skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::ConnectionUnavailable,
        remote: None,
        payload: Vec::new(),
    }
}

fn protocol_error(
    _source: &BrokerRuntimeSource,
    request: &RuntimeRequest,
) -> BrokerRuntimeResponse {
    BrokerRuntimeResponse {
        request_id: request.request_id.clone(),
        outcome:
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::ProtocolError,
        remote: None,
        payload: Vec::new(),
    }
}

fn resource_limit(
    _source: &BrokerRuntimeSource,
    request: &RuntimeRequest,
) -> BrokerRuntimeResponse {
    BrokerRuntimeResponse {
        request_id: request.request_id.clone(),
        outcome:
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::ResourceLimit,
        remote: None,
        payload: Vec::new(),
    }
}

fn transport_unavailable(_source: &BrokerRuntimeSource, request_id: &str) -> BrokerRuntimeResponse {
    BrokerRuntimeResponse {
        request_id: request_id.to_string(),
        outcome: skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::TransportUnavailable,
        remote: None,
        payload: Vec::new(),
    }
}

impl SessionConsumer for WebSocketRequestBroker {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::WebSocketRequestBroker
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        self.runtime_disconnected_sender(session);
        Ok(())
    }
}
