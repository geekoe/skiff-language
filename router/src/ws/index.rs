//! `ClientConnectionIndex`: logical client connection, business identity
//! replacement, `ClientSocketGeneration` generations and the independent
//! finalization barrier (C-client-lifecycle §3/§4, authority design §3.7).
//!
//! Every external terminal (peer close, business replacement, slow-client
//! overflow, Runtime disconnect, shutdown, protocol close, release timeout)
//! enters the same finalizer: mark closing + deindex → broker detach +
//! tombstones → ledger release → writer close/drain → barrier removes the old
//! generation record. The old finalizer never touches a replacement record.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::connection_protocol::{ClientSocketGeneration, WebSocketRpcProfile};
use tokio::task::JoinHandle;

use crate::session::identity::RuntimeSessionEpoch;

use super::ledger::PendingReleaseHandle;
use super::ledger::{ReleaseOutcome, ReleaseResolution};
use super::types::{
    BrokerConnectionGeneration, BusinessKey, ClientTerminal, Clock, OwnerToken, PeerWriter,
    SystemClock, WebSocketLifecycleClose, CLOSE_HIGH_WATER_CAPACITY, CLOSE_POLICY_REJECTED,
    CLOSE_PROTOCOL_ERROR, CLOSE_RELEASE_TIMEOUT, CLOSE_RUNTIME_DISCONNECTED, CLOSE_SHUTDOWN,
    CLOSE_SLOW_CLIENT, CLOSE_SUPERSEDED, CLOSE_TRANSPORT_ERROR,
};

#[derive(Debug, Clone)]
pub struct ClientConnectionIndexOptions {
    pub connection_limit: usize,
    pub slow_client_budget_bytes: u64,
    pub high_water_capacity: usize,
}

impl Default for ClientConnectionIndexOptions {
    fn default() -> Self {
        Self {
            connection_limit: super::types::CONNECTION_LIMIT_DEFAULT,
            slow_client_budget_bytes: super::types::SLOW_CLIENT_BUDGET_BYTES_DEFAULT,
            high_water_capacity: super::types::CONNECTION_LIMIT_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    RejectNew,
    CloseOldest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionOutcome {
    Accepted,
    Rejected { close: WebSocketLifecycleClose },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachMeta {
    pub service_id: String,
    pub websocket_entry_id: String,
    pub profile: WebSocketRpcProfile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHealthSnapshot {
    pub connection_count: usize,
    pub open_connections: Vec<String>,
    pub finalizer_pending: usize,
    pub finalizer_count: u64,
    pub finalizer_failures: Vec<String>,
    pub terminals_by_id: HashMap<String, ClientTerminal>,
    pub slow_client_count: u64,
    pub observed_write_bytes: HashMap<String, u64>,
}

/// Broker generation lifecycle port (attach + finalizer detach).
pub trait BrokerGenerationPort: Send + Sync + fmt::Debug {
    fn attach_generation(
        &self,
        handle: BrokerConnectionGeneration,
        writer: Arc<dyn PeerWriter>,
        generation: u64,
    ) -> Result<OwnerToken, String>;

    fn close_generation(
        &self,
        connection_id: &str,
        generation: &ClientSocketGeneration,
        protocol_outcome: bool,
    ) -> Result<(), String>;
}

/// Ledger release port (finalizer step 4).
pub trait LedgerReleasePort: Send + Sync + fmt::Debug {
    fn release_connection(
        &self,
        connection_id: &str,
        socket_open: bool,
    ) -> Result<ReleaseOutcome, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Reserved,
    Admitted,
    Attached,
}

#[derive(Debug)]
struct ConnectionRecord {
    business_key: Option<BusinessKey>,
    rank: Option<u64>,
    state: ConnectionState,
    generation: Option<ClientSocketGeneration>,
    runtime: Option<RuntimeSessionEpoch>,
    writer: Option<Arc<dyn PeerWriter>>,
    observed_write_bytes: u64,
    observed_writes: u64,
}

#[derive(Debug, Clone)]
struct FinalizerRecord {
    connection_id: String,
    generation: Option<ClientSocketGeneration>,
    terminal: ClientTerminal,
    close: Option<WebSocketLifecycleClose>,
    writer: Option<Arc<dyn PeerWriter>>,
}

/// Finalizer started synchronously: broker detach + ledger release initiation
/// are inline steps; only the release wait, writer close/drain and record
/// removal run in the barrier task.
struct StartedFinalizer {
    record: FinalizerRecord,
    release: Option<PendingReleaseHandle>,
    immediate_failures: Vec<String>,
}

#[derive(Debug, Default)]
struct IndexInner {
    connections_by_id: HashMap<String, ConnectionRecord>,
    admission_order: Vec<String>,
    business_order: HashMap<String, Vec<String>>,
    high_water: HashMap<String, u64>,
    by_runtime: HashMap<RuntimeSessionEpoch, BTreeSet<String>>,
    finalizing: HashMap<String, FinalizerRecord>,
    finalizer_count: u64,
    finalizer_failures: Vec<String>,
    terminals_by_id: HashMap<String, ClientTerminal>,
    slow_client_count: u64,
    shutting_down: bool,
}

/// Unique owner of logical client connections and per-generation finalizer
/// barriers (C-client-lifecycle §6.1).
#[derive(Debug)]
pub struct ClientConnectionIndex {
    inner: Mutex<IndexInner>,
    broker: Arc<dyn BrokerGenerationPort>,
    ledger: Arc<dyn LedgerReleasePort>,
    options: ClientConnectionIndexOptions,
}

impl ClientConnectionIndex {
    pub fn new(
        broker: Arc<dyn BrokerGenerationPort>,
        ledger: Arc<dyn LedgerReleasePort>,
        options: ClientConnectionIndexOptions,
    ) -> Arc<Self> {
        Self::with_clock(broker, ledger, options, Arc::new(SystemClock))
    }

    pub fn with_clock(
        broker: Arc<dyn BrokerGenerationPort>,
        ledger: Arc<dyn LedgerReleasePort>,
        options: ClientConnectionIndexOptions,
        _clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(IndexInner::default()),
            broker,
            ledger,
            options,
        })
    }

    /// Reserve a connection id (C-client-lifecycle §3.1). Connection-limit
    /// overflow and shutdown reject without entering the handshake.
    pub fn reserve(&self, id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.shutting_down {
            return Err("gateway is shutting down".to_string());
        }
        if inner.connections_by_id.len() >= self.options.connection_limit {
            return Err("connection limit reached".to_string());
        }
        if inner.connections_by_id.contains_key(id) {
            return Err(format!("connection {id} already exists"));
        }
        inner.connections_by_id.insert(
            id.to_string(),
            ConnectionRecord {
                business_key: None,
                rank: None,
                state: ConnectionState::Reserved,
                generation: None,
                runtime: None,
                writer: None,
                observed_write_bytes: 0,
                observed_writes: 0,
            },
        );
        inner.admission_order.push(id.to_string());
        Ok(())
    }

    /// Admit with business replacement (C-client-lifecycle §3.2). Old
    /// generations are closed before the new record is indexed; their
    /// finalizers are spawned after the admission lock is released.
    pub fn admit(
        self: &Arc<Self>,
        id: &str,
        business_key: Option<BusinessKey>,
        rank: Option<u64>,
        max_connections: usize,
        overflow: OverflowPolicy,
    ) -> AdmissionOutcome {
        if !(1..=(1u64 << 32) - 1).contains(&(max_connections as u64)) {
            return AdmissionOutcome::Rejected {
                close: WebSocketLifecycleClose {
                    code: CLOSE_POLICY_REJECTED.0,
                    reason: "maxConnections out of range".to_string(),
                },
            };
        }
        let mut to_finalize = Vec::new();
        let outcome = {
            let mut inner = self.lock();
            if inner.shutting_down {
                return AdmissionOutcome::Rejected {
                    close: WebSocketLifecycleClose {
                        code: CLOSE_SHUTDOWN.0,
                        reason: CLOSE_SHUTDOWN.1.to_string(),
                    },
                };
            }
            let Some(record) = inner.connections_by_id.get_mut(id) else {
                return AdmissionOutcome::Rejected {
                    close: WebSocketLifecycleClose {
                        code: CLOSE_POLICY_REJECTED.0,
                        reason: "unknown connection id".to_string(),
                    },
                };
            };
            if record.state != ConnectionState::Reserved {
                return AdmissionOutcome::Rejected {
                    close: WebSocketLifecycleClose {
                        code: CLOSE_POLICY_REJECTED.0,
                        reason: "connection is not reserved".to_string(),
                    },
                };
            }
            let mut rejected = None;
            if let Some(key) = &business_key {
                let existing = self.existing_for(&inner, key);
                let fence = inner.high_water.get(key.as_str()).copied();
                if fence.is_some_and(|fence| rank.is_none_or(|rank| rank <= fence)) {
                    rejected = Some(WebSocketLifecycleClose {
                        code: CLOSE_POLICY_REJECTED.0,
                        reason: CLOSE_POLICY_REJECTED.1.to_string(),
                    });
                } else if rank.is_some() {
                    if !inner.high_water.contains_key(key.as_str())
                        && inner.high_water.len() >= self.options.high_water_capacity
                    {
                        rejected = Some(WebSocketLifecycleClose {
                            code: CLOSE_HIGH_WATER_CAPACITY.0,
                            reason: CLOSE_HIGH_WATER_CAPACITY.1.to_string(),
                        });
                    } else {
                        inner
                            .high_water
                            .insert(key.as_str().to_string(), rank.unwrap());
                        for old_id in existing {
                            if let Some(record) = self.finish_locked(
                                &mut inner,
                                &old_id,
                                ClientTerminal::Replacement,
                                Some(WebSocketLifecycleClose {
                                    code: CLOSE_SUPERSEDED.0,
                                    reason: CLOSE_SUPERSEDED.1.to_string(),
                                }),
                            ) {
                                to_finalize.push(record);
                            }
                        }
                    }
                } else if overflow == OverflowPolicy::RejectNew && existing.len() >= max_connections
                {
                    rejected = Some(WebSocketLifecycleClose {
                        code: CLOSE_POLICY_REJECTED.0,
                        reason: CLOSE_POLICY_REJECTED.1.to_string(),
                    });
                } else if overflow == OverflowPolicy::CloseOldest
                    && existing.len() + 1 > max_connections
                {
                    let overflow_count = existing.len() + 1 - max_connections;
                    for old_id in existing.into_iter().take(overflow_count) {
                        if let Some(record) = self.finish_locked(
                            &mut inner,
                            &old_id,
                            ClientTerminal::Replacement,
                            Some(WebSocketLifecycleClose {
                                code: CLOSE_POLICY_REJECTED.0,
                                reason: CLOSE_POLICY_REJECTED.1.to_string(),
                            }),
                        ) {
                            to_finalize.push(record);
                        }
                    }
                }
            }
            if let Some(close) = rejected {
                self.finish_locked(&mut inner, id, ClientTerminal::PolicyRejected, None);
                return AdmissionOutcome::Rejected { close };
            }
            let record = inner.connections_by_id.get_mut(id).expect("record");
            record.business_key = business_key.clone();
            record.rank = rank;
            record.state = ConnectionState::Admitted;
            if let Some(key) = &business_key {
                inner
                    .business_order
                    .entry(key.as_str().to_string())
                    .or_default()
                    .push(id.to_string());
            }
            AdmissionOutcome::Accepted
        };
        for record in to_finalize {
            self.spawn_finalizer(self.start_finalizer(record));
        }
        outcome
    }

    /// Attach the physical socket generation (C-client-lifecycle §3.1) and
    /// register the broker generation with the captured writer. Returns the
    /// fenced writer handle.
    pub fn attach(
        self: &Arc<Self>,
        id: &str,
        generation: u64,
        display: String,
        runtime: RuntimeSessionEpoch,
        writer: Arc<dyn PeerWriter>,
        meta: AttachMeta,
    ) -> Result<Arc<dyn PeerWriter>, String> {
        let typed = ClientSocketGeneration::new(id.to_string(), generation)?;
        let captured: Arc<dyn PeerWriter> = Arc::new(CapturedPeerWriter {
            index: self.clone(),
            connection_id: id.to_string(),
            generation: typed.clone(),
            transport: writer.clone(),
        });
        let handle = BrokerConnectionGeneration {
            connection_id: id.to_string(),
            socket_generation: display,
            service_id: meta.service_id,
            websocket_entry_id: meta.websocket_entry_id,
            profile: meta.profile,
        };
        self.broker
            .attach_generation(handle, captured.clone(), generation)?;
        {
            let mut inner = self.lock();
            let Some(record) = inner.connections_by_id.get_mut(id) else {
                let _ = self.broker.close_generation(id, &typed, false);
                return Err(format!("unknown connection {id}"));
            };
            if record.state != ConnectionState::Admitted {
                return Err(format!("connection {id} is not admitted"));
            }
            record.generation = Some(typed.clone());
            record.runtime = Some(runtime.clone());
            record.writer = Some(captured.clone());
            record.state = ConnectionState::Attached;
            inner
                .by_runtime
                .entry(runtime.clone())
                .or_default()
                .insert(id.to_string());
        }
        Ok(captured)
    }

    /// Finalizer entry point: every external terminal enters here exactly
    /// once (idempotent for later events). Returns the spawned barrier handle
    /// for shutdown/flush aggregation.
    pub fn finish(
        self: &Arc<Self>,
        id: &str,
        terminal: ClientTerminal,
        close: Option<WebSocketLifecycleClose>,
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        let record = {
            let mut inner = self.lock();
            self.finish_locked(&mut inner, id, terminal, close)
        };
        record
            .map(|record| self.start_finalizer(record))
            .map(|started| self.spawn_finalizer(started))
    }

    /// Runtime disconnect: finish every attached connection of the exact
    /// Runtime session with 1011 (C-client-lifecycle §6.5).
    pub fn runtime_disconnected(
        self: &Arc<Self>,
        runtime: &RuntimeSessionEpoch,
    ) -> Vec<JoinHandle<Result<(), Vec<String>>>> {
        let records = {
            let mut inner = self.lock();
            let affected = inner
                .by_runtime
                .get(runtime)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            affected
                .into_iter()
                .filter_map(|id| {
                    self.finish_locked(&mut inner, &id, ClientTerminal::RuntimeDisconnect, None)
                })
                .collect::<Vec<_>>()
        };
        records
            .into_iter()
            .map(|record| self.spawn_finalizer(self.start_finalizer(record)))
            .collect()
    }

    /// Shutdown: stop admission and finish every open connection with 1001
    /// (C-process-lifecycle S3/S5).
    pub fn shutdown(self: &Arc<Self>) -> Vec<JoinHandle<Result<(), Vec<String>>>> {
        let records = {
            let mut inner = self.lock();
            inner.shutting_down = true;
            let open = inner.connections_by_id.keys().cloned().collect::<Vec<_>>();
            open.into_iter()
                .filter_map(|id| {
                    self.finish_locked(&mut inner, &id, ClientTerminal::Shutdown, None)
                })
                .collect::<Vec<_>>()
        };
        records
            .into_iter()
            .map(|record| self.spawn_finalizer(self.start_finalizer(record)))
            .collect()
    }

    /// Slow-client write over the byte budget (C-client-lifecycle §3.4).
    pub fn slow_client_write(&self, id: &str, bytes: u64) -> WriteBudget {
        let inner = self.lock();
        let Some(record) = inner.connections_by_id.get(id) else {
            return WriteBudget::Stale;
        };
        if record.state != ConnectionState::Attached {
            return WriteBudget::Stale;
        }
        let buffered = record
            .writer
            .as_ref()
            .map(|writer| writer.buffered_bytes())
            .unwrap_or(0);
        if buffered
            .saturating_add(record.observed_write_bytes)
            .saturating_add(bytes)
            > self.options.slow_client_budget_bytes
        {
            WriteBudget::OverBudget
        } else {
            WriteBudget::Accepted
        }
    }

    /// Captured writer fence + budget reservation (single-writer invariant).
    pub(crate) fn reserve_write(
        &self,
        id: &str,
        generation: &ClientSocketGeneration,
        bytes: u64,
    ) -> WriteBudget {
        let mut inner = self.lock();
        let Some(record) = inner.connections_by_id.get(id) else {
            return WriteBudget::Stale;
        };
        if record.state != ConnectionState::Attached
            || record.generation.as_ref() != Some(generation)
        {
            return WriteBudget::Stale;
        }
        let buffered = record
            .writer
            .as_ref()
            .map(|writer| writer.buffered_bytes())
            .unwrap_or(0);
        if buffered
            .saturating_add(record.observed_write_bytes)
            .saturating_add(bytes)
            > self.options.slow_client_budget_bytes
        {
            return WriteBudget::OverBudget;
        }
        let record = inner.connections_by_id.get_mut(id).expect("record");
        record.observed_write_bytes = record.observed_write_bytes.saturating_add(bytes);
        record.observed_writes += 1;
        WriteBudget::Accepted
    }

    pub(crate) fn complete_write(&self, id: &str, generation: &ClientSocketGeneration, bytes: u64) {
        let mut inner = self.lock();
        if let Some(record) = inner.connections_by_id.get_mut(id) {
            if record.generation.as_ref() == Some(generation) {
                record.observed_write_bytes = record.observed_write_bytes.saturating_sub(bytes);
            }
        }
    }

    fn finish_locked(
        &self,
        inner: &mut IndexInner,
        id: &str,
        terminal: ClientTerminal,
        close: Option<WebSocketLifecycleClose>,
    ) -> Option<FinalizerRecord> {
        let mut record = inner.connections_by_id.remove(id)?;
        if let Some(key) = &record.business_key {
            if let Some(order) = inner.business_order.get_mut(key.as_str()) {
                order.retain(|existing| existing != id);
                if order.is_empty() {
                    inner.business_order.remove(key.as_str());
                    // Fence reclamation: only when no active connection remains.
                    inner.high_water.remove(key.as_str());
                }
            }
        }
        if let Some(runtime) = &record.runtime {
            if let Some(set) = inner.by_runtime.get_mut(runtime) {
                set.remove(id);
            }
        }
        inner.finalizer_count += 1;
        inner.terminals_by_id.insert(id.to_string(), terminal);
        if terminal == ClientTerminal::SlowClient {
            inner.slow_client_count += 1;
        }
        let finalizer = FinalizerRecord {
            connection_id: id.to_string(),
            generation: record.generation.take(),
            terminal,
            close,
            writer: record.writer.take(),
        };
        if finalizer.writer.is_some() || finalizer.generation.is_some() {
            inner.finalizing.insert(id.to_string(), finalizer.clone());
        }
        Some(finalizer)
    }

    fn start_finalizer(&self, record: FinalizerRecord) -> StartedFinalizer {
        let mut immediate_failures = Vec::new();
        let release = if let Some(generation) = &record.generation {
            if let Err(error) = self.broker.close_generation(
                &record.connection_id,
                generation,
                record.terminal == ClientTerminal::ProtocolClose,
            ) {
                immediate_failures.push(error);
            }
            let socket_open = record.terminal != ClientTerminal::RuntimeDisconnect;
            match self
                .ledger
                .release_connection(&record.connection_id, socket_open)
            {
                Ok(ReleaseOutcome::Pending(handle)) => Some(handle),
                Ok(ReleaseOutcome::Resolved) => None,
                Err(error) => {
                    immediate_failures.push(error);
                    None
                }
            }
        } else {
            None
        };
        StartedFinalizer {
            record,
            release,
            immediate_failures,
        }
    }

    fn spawn_finalizer(
        self: &Arc<Self>,
        started: StartedFinalizer,
    ) -> JoinHandle<Result<(), Vec<String>>> {
        let index = self.clone();
        tokio::spawn(async move {
            let mut failures = started.immediate_failures;
            if let Some(handle) = started.release {
                if let ReleaseResolution::Failed { reason } = handle.wait().await {
                    failures.push(reason);
                }
            }
            if let Some(writer) = &started.record.writer {
                close_writer(
                    writer,
                    &started.record.terminal,
                    started.record.close.as_ref(),
                );
            }
            index.remove_finalized(&started.record.connection_id);
            index.record_finalizer_failures(&failures);
            if failures.is_empty() {
                Ok(())
            } else {
                Err(failures)
            }
        })
    }

    fn remove_finalized(&self, id: &str) {
        let mut inner = self.lock();
        inner.finalizing.remove(id);
    }

    fn record_finalizer_failures(&self, failures: &[String]) {
        if failures.is_empty() {
            return;
        }
        let mut inner = self.lock();
        inner.finalizer_failures.extend(failures.iter().cloned());
    }

    fn existing_for(&self, inner: &IndexInner, key: &BusinessKey) -> Vec<String> {
        inner
            .business_order
            .get(key.as_str())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|id| inner.connections_by_id.contains_key(id))
            .collect()
    }

    pub fn connection_terminal(&self, id: &str) -> Option<String> {
        let inner = self.lock();
        if inner.connections_by_id.contains_key(id) {
            return Some("None".to_string());
        }
        if inner.finalizing.contains_key(id) {
            return inner
                .finalizing
                .get(id)
                .map(|record| record.terminal.as_str().to_string());
        }
        inner
            .terminals_by_id
            .get(id)
            .map(|terminal| terminal.as_str().to_string())
    }

    /// Live connection ids for one business identity (Runtime
    /// `connection.send` targeting `businessIdentity`, TS parity). Only
    /// admitted connections with a current record are returned.
    pub fn connections_for_business_identity(
        &self,
        service_id: &str,
        websocket_entry_id: &str,
        business_identity: &str,
    ) -> Vec<String> {
        let key = BusinessKey::from_parts(service_id, websocket_entry_id, business_identity);
        let inner = self.lock();
        self.existing_for(&inner, &key)
    }

    pub fn finalizer_terminal(&self, id: &str) -> Option<ClientTerminal> {
        self.lock().finalizing.get(id).map(|record| record.terminal)
    }

    pub fn snapshot(&self) -> IndexHealthSnapshot {
        let inner = self.lock();
        IndexHealthSnapshot {
            connection_count: inner.connections_by_id.len(),
            open_connections: inner
                .admission_order
                .iter()
                .filter(|id| inner.connections_by_id.contains_key(*id))
                .cloned()
                .collect(),
            finalizer_pending: inner.finalizing.len(),
            finalizer_count: inner.finalizer_count,
            finalizer_failures: inner.finalizer_failures.clone(),
            terminals_by_id: inner.terminals_by_id.clone(),
            slow_client_count: inner.slow_client_count,
            observed_write_bytes: inner
                .connections_by_id
                .iter()
                .filter(|(_, record)| record.state == ConnectionState::Attached)
                .map(|(id, record)| (id.clone(), record.observed_write_bytes))
                .collect(),
        }
    }

    pub fn connections_for_runtime(&self, runtime: &RuntimeSessionEpoch) -> Vec<String> {
        self.lock()
            .by_runtime
            .get(runtime)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, IndexInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBudget {
    Accepted,
    Stale,
    OverBudget,
}

/// Single captured writer with generation fence + slow-client budget
/// (C-client-lifecycle §3.3/§3.4).
#[derive(Debug)]
struct CapturedPeerWriter {
    index: Arc<ClientConnectionIndex>,
    connection_id: String,
    generation: ClientSocketGeneration,
    transport: Arc<dyn PeerWriter>,
}

impl PeerWriter for CapturedPeerWriter {
    fn write_text(&self, frame: String) -> Result<(), String> {
        let bytes = frame.len() as u64;
        match self
            .index
            .reserve_write(&self.connection_id, &self.generation, bytes)
        {
            WriteBudget::Accepted => {}
            WriteBudget::Stale => return Err("captured writer is stale".to_string()),
            WriteBudget::OverBudget => {
                let _ = self.index.clone().finish(
                    &self.connection_id,
                    ClientTerminal::SlowClient,
                    None,
                );
                return Err("websocket client is too slow".to_string());
            }
        }
        let result = self.transport.write_text(frame);
        if result.is_err() {
            self.index
                .complete_write(&self.connection_id, &self.generation, bytes);
        }
        result
    }

    fn write_binary(&self, payload: Vec<u8>) -> Result<(), String> {
        let bytes = payload.len() as u64;
        match self
            .index
            .reserve_write(&self.connection_id, &self.generation, bytes)
        {
            WriteBudget::Accepted => {}
            WriteBudget::Stale => return Err("captured writer is stale".to_string()),
            WriteBudget::OverBudget => {
                let _ = self.index.clone().finish(
                    &self.connection_id,
                    ClientTerminal::SlowClient,
                    None,
                );
                return Err("websocket client is too slow".to_string());
            }
        }
        let result = self.transport.write_binary(payload);
        if result.is_err() {
            self.index
                .complete_write(&self.connection_id, &self.generation, bytes);
        }
        result
    }

    fn buffered_bytes(&self) -> u64 {
        self.transport.buffered_bytes()
    }

    fn close(&self, code: u16, reason: &str) -> Result<(), String> {
        self.transport.close(code, reason)
    }

    fn terminate(&self) {
        self.transport.terminate();
    }
}

fn close_writer(
    writer: &Arc<dyn PeerWriter>,
    terminal: &ClientTerminal,
    explicit: Option<&WebSocketLifecycleClose>,
) {
    match terminal {
        // Peer close writes no close frame (C-client-lifecycle §4).
        ClientTerminal::PeerClose => writer.terminate(),
        _ => {
            let close = explicit.cloned().or_else(|| close_for_terminal(*terminal));
            if let Some(close) = close {
                if writer.close(close.code, &close.reason).is_err() {
                    writer.terminate();
                }
            } else {
                writer.terminate();
            }
        }
    }
}

fn close_for_terminal(terminal: ClientTerminal) -> Option<WebSocketLifecycleClose> {
    let (code, reason) = match terminal {
        ClientTerminal::PeerClose => return None,
        ClientTerminal::Replacement => CLOSE_SUPERSEDED,
        ClientTerminal::PolicyRejected => CLOSE_POLICY_REJECTED,
        ClientTerminal::RuntimeDisconnect => CLOSE_RUNTIME_DISCONNECTED,
        ClientTerminal::Shutdown => CLOSE_SHUTDOWN,
        ClientTerminal::SlowClient => CLOSE_SLOW_CLIENT,
        ClientTerminal::ProtocolClose => CLOSE_PROTOCOL_ERROR,
        ClientTerminal::ReleaseTimeout => CLOSE_RELEASE_TIMEOUT,
        ClientTerminal::TransportError => CLOSE_TRANSPORT_ERROR,
    };
    Some(WebSocketLifecycleClose {
        code,
        reason: reason.to_string(),
    })
}
