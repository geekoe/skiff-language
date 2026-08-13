//! `SessionLayer`: the W-session assembly owner.
//!
//! Invariant: this layer uniquely owns the `RuntimeRegistrationDirectory`,
//! the pre-auth pool, the live session-task handles, the consumer manifest
//! with its reserved terminal mailboxes, and the process fail-stop flag. It
//! does not own routing eligibility, admission permits, request pending or
//! any other business mutable state. A replacement request only cancels the
//! old session's task; the old close barrier runs inside that task and can
//! never delete a replacement.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use skiff_runtime_transport::protocol::{
    encode_binary_frame, RuntimeRegisteredFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::sync::{oneshot, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::timeout;

use crate::config::RouterConfig;
use crate::routing::DispatchCapabilities;

use super::bootstrap::RuntimeBootstrapProvider;
use super::budget::SessionBudgets;
use super::consumer::{
    ConsumerKind, ConsumerMailbox, ConsumerManifest, FailStop, SessionConsumer,
    TerminalDeliveryError,
};
use super::demux::{InboundSinkSet, RegistrationFrameSink, RuntimeFrameDemux};
use super::directory::{RegistrationFacts, RuntimeRegistrationDirectory};
use super::health::RuntimeHealthLedger;
use super::identity::{RuntimeConnectionEpoch, RuntimeSessionEpoch};
use super::observer::RegistrationObserver;
use super::pre_auth::PreAuthPool;
use super::task::{run_session_task, RuntimeSocket};

/// Handshake and close deadlines. Defaults are process-level constants
/// (C-session §4/§5.3, C-process-lifecycle S6); tests inject smaller values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTiming {
    pub bootstrap: Duration,
    pub capabilities: Duration,
    pub ack_write: Duration,
    pub close_barrier: Duration,
    pub shutdown_total: Duration,
}

impl Default for SessionTiming {
    fn default() -> Self {
        Self {
            // Cold-start parity (TS had no per-phase handshake deadlines):
            // a fresh Runtime may spend 10-21s provisioning the
            // whole-assembly service DB indexes before it can send
            // `runtime.capabilities`; the deadline must cover that window.
            // Registered sessions never consult these deadlines again.
            bootstrap: Duration::from_secs(30),
            capabilities: Duration::from_secs(30),
            ack_write: Duration::from_secs(5),
            close_barrier: Duration::from_secs(20),
            shutdown_total: Duration::from_secs(20),
        }
    }
}

/// Injectable layer options (tests use fake consumer manifests).
#[derive(Debug)]
pub struct SessionLayerOptions {
    pub manifest: ConsumerManifest,
    pub consumers: Vec<Arc<dyn SessionConsumer>>,
    pub timing: SessionTiming,
    pub budgets: SessionBudgets,
    /// Test seam: delay before every outbound frame write (defaults None).
    pub writer_delay: Option<Duration>,
}

impl Default for SessionLayerOptions {
    fn default() -> Self {
        Self {
            manifest: ConsumerManifest::default_installed(),
            consumers: vec![Arc::new(RuntimeHealthLedger::new())],
            timing: SessionTiming::default(),
            budgets: SessionBudgets::default(),
            writer_delay: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCloseReason {
    Shutdown,
    Replaced,
    Disconnect,
}

/// Bounded non-blocking writer for one exact Runtime session (composition
/// seam; C-session §5.3).
///
/// The `OutboundQueue` stays owned by the per-connection session task; this
/// trait exposes only the bounded enqueue surface so installed lane ports
/// (`RuntimePeer`, activation enqueue, WS lifecycle/responder, actor
/// control) can write frames to the exact session. `Err` means the queue is
/// full or the session has no registered writer; the caller fails closed per
/// its own contract and must never wait.
pub trait SessionFrameWriter: Send + Sync + fmt::Debug {
    fn enqueue(&self, bytes: Vec<u8>) -> Result<(), String>;
}

#[derive(Debug)]
pub enum SessionLayerError {
    Config(String),
    FailStop(FailStop),
    Timeout(String),
}

impl fmt::Display for SessionLayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(formatter, "session layer config error: {message}"),
            Self::FailStop(reason) => write!(formatter, "session layer fail-stop: {reason}"),
            Self::Timeout(message) => write!(formatter, "session layer timeout: {message}"),
        }
    }
}

impl std::error::Error for SessionLayerError {}

#[derive(Debug)]
struct SessionTaskHandle {
    cancel_tx: watch::Sender<Option<SessionCloseReason>>,
    abort_handle: AbortHandle,
    join: JoinHandle<()>,
}

/// Per-session registration facts refreshed from the `runtime.capabilities`
/// frame (integration-contract-v2 §1/§3): dispatch modes plus the loaded
/// build-id set and the lazy-load advertisement. The session task records
/// them on every capabilities bound/refresh; the layer keeps the coherent
/// snapshot for the candidate query and writes them through to the directory
/// record when it exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionRegistrationFacts {
    pub dispatch: DispatchCapabilities,
    pub registration: RegistrationFacts,
}

/// W-session assembly owner (see module doc).
#[derive(Debug)]
pub struct SessionLayer {
    bootstrap_provider: RuntimeBootstrapProvider,
    manifest: ConsumerManifest,
    directory: Mutex<RuntimeRegistrationDirectory>,
    pre_auth: Mutex<PreAuthPool>,
    health: Arc<RuntimeHealthLedger>,
    mailboxes: BTreeMap<ConsumerKind, ConsumerMailbox>,
    demux: RuntimeFrameDemux,
    registration_sink: RegistrationFrameSink,
    pub(crate) timing: SessionTiming,
    pub(crate) budgets: SessionBudgets,
    pub(crate) writer_delay: Option<Duration>,
    router_artifact_root: Option<String>,
    next_connection_id: AtomicU64,
    next_generation: AtomicU64,
    shutdown_tx: watch::Sender<()>,
    fail_stop: Mutex<Option<String>>,
    fail_stop_tx: watch::Sender<Option<String>>,
    handles: Mutex<HashMap<String, SessionTaskHandle>>,
    epoch_index: Mutex<HashMap<RuntimeSessionEpoch, String>>,
    frame_writers: Mutex<HashMap<RuntimeSessionEpoch, Arc<dyn SessionFrameWriter>>>,
    inbound_sinks: Mutex<Arc<InboundSinkSet>>,
    registration_facts: Mutex<HashMap<RuntimeSessionEpoch, SessionRegistrationFacts>>,
    registration_observer: Mutex<Option<Arc<dyn RegistrationObserver>>>,
}

impl SessionLayer {
    /// Production assembly: default timing/budgets and the health-ledger
    /// consumer.
    pub fn new(config: RouterConfig) -> Self {
        Self::with_options(config, SessionLayerOptions::default())
            .expect("default session layer options are valid")
    }

    pub fn with_options(
        config: RouterConfig,
        options: SessionLayerOptions,
    ) -> Result<Self, SessionLayerError> {
        // Static manifest checker: every installed session-keyed component is
        // in the manifest, and every manifest kind has exactly one consumer.
        // The health ledger is the single owner of retained observations:
        // the layer records into it and registers it as the HealthLedger
        // session consumer so the close barrier removes exact observations
        // (batch 12 health leaf; without this, disconnected sessions leave
        // stale observations forever).
        let health = Arc::new(RuntimeHealthLedger::new());
        let mut consumer_kinds = std::collections::BTreeSet::new();
        for consumer in &options.consumers {
            if !consumer_kinds.insert(consumer.kind()) {
                return Err(SessionLayerError::Config(format!(
                    "duplicate session consumer {:?}",
                    consumer.kind()
                )));
            }
        }
        let manifest_kinds = options
            .manifest
            .kinds()
            .collect::<std::collections::BTreeSet<_>>();
        if consumer_kinds != manifest_kinds {
            return Err(SessionLayerError::Config(format!(
                "consumer manifest mismatch: manifest={manifest_kinds:?} consumers={consumer_kinds:?}"
            )));
        }

        let pre_auth_limit = usize::try_from(config.runtime_max_concurrency).unwrap_or(usize::MAX);
        let mut mailboxes = BTreeMap::new();
        for consumer in options.consumers.iter().map(|consumer| {
            if consumer.kind() == ConsumerKind::HealthLedger {
                Arc::clone(&health) as Arc<dyn SessionConsumer>
            } else {
                Arc::clone(consumer)
            }
        }) {
            let mailbox = ConsumerMailbox::spawn(Arc::clone(&consumer), pre_auth_limit);
            mailboxes.insert(consumer.kind(), mailbox);
        }

        let directory = RuntimeRegistrationDirectory::new(&options.manifest);
        let pre_auth = PreAuthPool::new(pre_auth_limit);
        let (shutdown_tx, _) = watch::channel(());
        let (fail_stop_tx, _) = watch::channel(None);
        let bootstrap_provider = RuntimeBootstrapProvider::new(&config);

        Ok(Self {
            bootstrap_provider,
            manifest: options.manifest,
            directory: Mutex::new(directory),
            pre_auth: Mutex::new(pre_auth),
            health,
            mailboxes,
            demux: RuntimeFrameDemux,
            registration_sink: RegistrationFrameSink,
            timing: options.timing,
            budgets: options.budgets,
            writer_delay: options.writer_delay,
            router_artifact_root: Some(crate::config::canonicalize_artifact_root(
                &config.artifacts_path,
            )),
            next_connection_id: AtomicU64::new(0),
            next_generation: AtomicU64::new(0),
            shutdown_tx,
            fail_stop: Mutex::new(None),
            fail_stop_tx,
            handles: Mutex::new(HashMap::new()),
            epoch_index: Mutex::new(HashMap::new()),
            frame_writers: Mutex::new(HashMap::new()),
            inbound_sinks: Mutex::new(Arc::new(InboundSinkSet::default())),
            registration_facts: Mutex::new(HashMap::new()),
            registration_observer: Mutex::new(None),
        })
    }

    /// Installs the registration observer. Additive: an absent observer is a
    /// no-op and the session state machine never depends on the callback.
    pub fn set_registration_observer(&self, observer: Arc<dyn RegistrationObserver>) {
        *self
            .registration_observer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(observer);
    }

    /// Notifies the installed observer that a session became routable. Used
    /// by the session task right after `mark_registered`; failures inside the
    /// observer are non-blocking by contract and never fail-stop the session.
    pub(crate) fn notify_session_registered(&self, session: &RuntimeSessionEpoch) {
        if let Some(observer) = self
            .registration_observer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            observer.on_session_registered(session);
        }
    }

    /// The byte-exact `router.bootstrap` frame built from the frozen Router
    /// config (profile + artifact root + service DB; M4: no epoch tuple).
    pub fn bootstrap_bytes(&self) -> Option<Vec<u8>> {
        self.bootstrap_provider.build().ok()
    }

    pub fn manifest_kinds(&self) -> Vec<ConsumerKind> {
        self.manifest.kinds().collect()
    }

    pub fn registered_ack_bytes(&self, replica_id: &str) -> Option<Vec<u8>> {
        let header = RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: replica_id.to_string(),
        };
        encode_binary_frame(&header, &[]).ok()
    }

    pub fn directory_lock(&self) -> MutexGuard<'_, RuntimeRegistrationDirectory> {
        self.directory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn health(&self) -> &Arc<RuntimeHealthLedger> {
        &self.health
    }

    pub fn demux(&self) -> RuntimeFrameDemux {
        self.demux
    }

    pub fn registration_sink(&self) -> RegistrationFrameSink {
        self.registration_sink
    }

    pub fn timing(&self) -> SessionTiming {
        self.timing
    }

    /// Accept one upgraded `/runtime` WebSocket. Refused connections (pre-auth
    /// cap) are closed immediately without entering the handshake.
    pub fn accept(self: &Arc<Self>, socket: RuntimeSocket) {
        let connection_id = format!(
            "conn-{}",
            self.next_connection_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        if !self
            .pre_auth
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_acquire(&connection_id)
        {
            return;
        }
        let connection_epoch = RuntimeConnectionEpoch {
            opaque_connection_id: connection_id.clone(),
            generation,
        };
        let (cancel_tx, cancel_rx) = watch::channel(None);
        let shutdown_rx = self.shutdown_tx.subscribe();
        let join = tokio::spawn(run_session_task(
            Arc::clone(self),
            connection_epoch,
            socket,
            shutdown_rx,
            cancel_rx,
        ));
        let handle = SessionTaskHandle {
            cancel_tx,
            abort_handle: join.abort_handle(),
            join,
        };
        self.handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection_id, handle);
    }

    pub fn bind_session(&self, connection_id: &str, session: RuntimeSessionEpoch) {
        self.epoch_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session, connection_id.to_string());
    }

    /// Registers the bounded frame writer for one bound session. The session
    /// task calls this once after capabilities bind; replacement sessions
    /// register their own writer under their own session epoch.
    pub fn register_frame_writer(
        &self,
        session: &RuntimeSessionEpoch,
        writer: Arc<dyn SessionFrameWriter>,
    ) {
        self.frame_writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session.clone(), writer);
    }

    /// Unregisters the exact-session writer before the close barrier. The
    /// public write path returns `Err` afterwards; the writer task itself is
    /// drained by the session task.
    pub fn unregister_frame_writer(&self, session: &RuntimeSessionEpoch) {
        self.frame_writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session);
    }

    /// Bounded non-blocking enqueue of one arbitrary frame to the exact
    /// session (composition seam; C-session §5.3). `Err` when the session has
    /// no registered writer or the frame/byte budget rejects the frame.
    pub fn write_session_frame(
        &self,
        session: &RuntimeSessionEpoch,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        let writer = self
            .frame_writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "no frame writer registered for session {:?}",
                    session.replica_id
                )
            })?;
        writer.enqueue(bytes)
    }

    pub fn has_frame_writer(&self, session: &RuntimeSessionEpoch) -> bool {
        self.frame_writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(session)
    }

    /// Records the validated `runtime.capabilities` registration facts for
    /// one bound session (integration-contract-v2 §3): every refresh
    /// overwrites the previous facts. The facts are retained in the layer
    /// snapshot and written through to the directory record when the session
    /// record already exists (a bound-but-not-yet-registered session keeps
    /// the cached facts; `sync_registration_facts` applies them once the
    /// record is published).
    pub fn record_registration_facts(
        &self,
        session: &RuntimeSessionEpoch,
        facts: SessionRegistrationFacts,
    ) {
        self.registration_facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session.clone(), facts.clone());
        let mut directory = self.directory_lock();
        if let Some(record) = directory.record_mut(session) {
            record.update_registration_facts(facts.registration);
        }
    }

    /// Applies the cached capabilities facts to the exact directory record
    /// (called right after the register publish created the record; the
    /// capabilities frame arrives before `runtime.register`).
    pub fn sync_registration_facts(&self, session: &RuntimeSessionEpoch) {
        let facts = self
            .registration_facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .cloned();
        if let Some(facts) = facts {
            let mut directory = self.directory_lock();
            if let Some(record) = directory.record_mut(session) {
                record.update_registration_facts(facts.registration);
            }
        }
    }

    pub fn remove_registration_facts(&self, session: &RuntimeSessionEpoch) {
        self.registration_facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session);
    }

    /// Coherent snapshot of the per-session registration facts (the
    /// C-routing-query seam; `RuntimeCandidateQuery::snapshot_directory_view`
    /// consumes it under the directory lock).
    pub fn registration_facts_snapshot(
        &self,
    ) -> HashMap<RuntimeSessionEpoch, SessionRegistrationFacts> {
        self.registration_facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Coherent snapshot of the per-session dispatch capability bindings
    /// (health projection seam; dispatch modes are projected out of the
    /// registration facts).
    pub fn dispatch_capabilities_snapshot(
        &self,
    ) -> HashMap<RuntimeSessionEpoch, DispatchCapabilities> {
        self.registration_facts_snapshot()
            .into_iter()
            .map(|(session, facts)| (session, facts.dispatch))
            .collect()
    }

    /// Current exact session for one replica (composition seam for
    /// actor/activation outbound ports that carry only `replica_id`).
    pub fn current_session_by_replica(&self, replica_id: &str) -> Option<RuntimeSessionEpoch> {
        self.directory_lock()
            .current_by_replica()
            .get(replica_id)
            .cloned()
    }

    /// Installs the static lane sink bundle (plan §5.5). Called by the
    /// composition before any listener starts; never extended at runtime.
    /// Existing session behavior is unchanged while the bundle is empty.
    pub fn install_inbound_sinks(&self, sinks: Arc<InboundSinkSet>) {
        *self
            .inbound_sinks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = sinks;
    }

    pub fn inbound_sinks(&self) -> Arc<InboundSinkSet> {
        self.inbound_sinks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Cancel the exact old session task (replacement or shutdown). The close
    /// barrier runs in the old task; this never deletes the new record.
    pub fn request_close(&self, session: &RuntimeSessionEpoch, reason: SessionCloseReason) {
        let connection_id = self
            .epoch_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .cloned();
        if let Some(connection_id) = connection_id {
            if let Some(handle) = self
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&connection_id)
            {
                let _ = handle.cancel_tx.send(Some(reason));
            }
        }
    }

    pub fn deliver_terminal(
        &self,
        consumer: ConsumerKind,
        session: &RuntimeSessionEpoch,
    ) -> Result<oneshot::Receiver<Result<(), String>>, TerminalDeliveryError> {
        let mailbox = self
            .mailboxes
            .get(&consumer)
            .ok_or_else(|| TerminalDeliveryError {
                consumer,
                session: session.clone(),
            })?;
        mailbox.try_deliver_terminal(session)
    }

    pub fn release_pre_auth(&self, connection_id: &str) {
        self.pre_auth
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .release(connection_id);
    }

    pub fn fail_stop(&self, reason: String) {
        let mut slot = self
            .fail_stop
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_none() {
            *slot = Some(reason.clone());
            let _ = self.fail_stop_tx.send(Some(reason));
        }
    }

    pub fn fail_stop_reason(&self) -> Option<String> {
        self.fail_stop
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn fail_stop_subscribe(&self) -> watch::Receiver<Option<String>> {
        self.fail_stop_tx.subscribe()
    }

    pub fn task_finished(&self, connection_id: &str) {
        self.handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(connection_id);
        self.epoch_index
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, conn| conn != connection_id);
    }

    /// Close all sessions via the close barrier (C-process-lifecycle S6) with
    /// a total deadline; fail-stop on timeout or barrier failure.
    pub async fn shutdown(&self) -> Result<(), SessionLayerError> {
        let _ = self.shutdown_tx.send(());
        let (joins, aborts): (Vec<JoinHandle<()>>, Vec<AbortHandle>) = {
            let mut handles = self
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            handles
                .drain()
                .map(|(_, handle)| (handle.join, handle.abort_handle))
                .unzip()
        };
        let joined = timeout(
            self.timing.shutdown_total,
            futures_util::future::join_all(joins),
        )
        .await;
        for abort in aborts {
            abort.abort();
        }
        for mailbox in self.mailboxes.values() {
            mailbox.abort();
        }
        match joined {
            Err(_) => Err(SessionLayerError::Timeout(
                "shutdown session barrier deadline exceeded".to_string(),
            )),
            Ok(_) => {
                if let Some(reason) = self.fail_stop_reason() {
                    Err(SessionLayerError::FailStop(FailStop { reason }))
                } else {
                    Ok(())
                }
            }
        }
    }

    pub fn health_snapshot(&self) -> SessionHealthSnapshot {
        let directory = self.directory_lock();
        let pre_auth = self
            .pre_auth
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        SessionHealthSnapshot {
            pre_auth_connections: pre_auth.occupied(),
            pre_auth_refused: pre_auth.refused(),
            registered_sessions: directory.routable_count(),
            pending_sessions: directory.pending_count(),
            cancelled_sessions: directory.cancelled_count(),
            barrier_pending: directory.barrier_pending_count(),
            consumer_permits_held: directory.permits_held(),
            observed_health: self.health.observed_total(),
            fail_stop: self.fail_stop_reason(),
            live_session_tasks: self
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
        }
    }

    pub fn candidates_by_build_id(&self, build_id: &str) -> Vec<RuntimeSessionEpoch> {
        self.directory_lock()
            .candidates_by_build_id(build_id, self.router_artifact_root.as_deref())
    }
}

/// Read-only health projection fields (C-session §7.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHealthSnapshot {
    pub pre_auth_connections: usize,
    pub pre_auth_refused: u64,
    pub registered_sessions: usize,
    pub pending_sessions: usize,
    pub cancelled_sessions: usize,
    pub barrier_pending: usize,
    pub consumer_permits_held: usize,
    pub observed_health: u64,
    pub fail_stop: Option<String>,
    pub live_session_tasks: usize,
}
