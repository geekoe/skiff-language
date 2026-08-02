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

use crate::bootstrap::ActiveRoutingEpochStore;
use crate::config::RouterConfig;

use super::bootstrap::RuntimeBootstrapProvider;
use super::budget::SessionBudgets;
use super::consumer::{
    ConsumerKind, ConsumerMailbox, ConsumerManifest, FailStop, SessionConsumer,
    TerminalDeliveryError,
};
use super::demux::{RegistrationFrameSink, RuntimeFrameDemux};
use super::directory::RuntimeRegistrationDirectory;
use super::handshake::EpochContext;
use super::health::RuntimeHealthLedger;
use super::identity::{RegisteredAssemblyTuple, RuntimeConnectionEpoch, RuntimeSessionEpoch};
use super::pre_auth::PreAuthPool;
use super::task::{run_session_task, RuntimeSocket};

/// Handshake and close deadlines. Defaults are process-level constants
/// (C-session §4/§5.3, C-process-lifecycle S6); tests inject smaller values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTiming {
    pub bootstrap: Duration,
    pub capabilities: Duration,
    pub register: Duration,
    pub ack_write: Duration,
    pub close_barrier: Duration,
    pub shutdown_total: Duration,
}

impl Default for SessionTiming {
    fn default() -> Self {
        Self {
            bootstrap: Duration::from_secs(10),
            capabilities: Duration::from_secs(10),
            register: Duration::from_secs(30),
            ack_write: Duration::from_secs(5),
            close_barrier: Duration::from_secs(20),
            shutdown_total: Duration::from_secs(20),
        }
    }
}

/// Injectable layer options (tests use the corpus committed epoch and fake
/// consumer manifests).
#[derive(Debug)]
pub struct SessionLayerOptions {
    pub committed_epoch: Option<RegisteredAssemblyTuple>,
    pub pending_epoch: Option<RegisteredAssemblyTuple>,
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
            committed_epoch: None,
            pending_epoch: None,
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

/// W-session assembly owner (see module doc).
#[derive(Debug)]
pub struct SessionLayer {
    bootstrap_provider: RuntimeBootstrapProvider,
    committed_epoch: Option<RegisteredAssemblyTuple>,
    epoch_store: Mutex<Option<Arc<ActiveRoutingEpochStore>>>,
    pending_epoch: Option<RegisteredAssemblyTuple>,
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
    next_connection_id: AtomicU64,
    next_generation: AtomicU64,
    shutdown_tx: watch::Sender<()>,
    fail_stop: Mutex<Option<String>>,
    fail_stop_tx: watch::Sender<Option<String>>,
    handles: Mutex<HashMap<String, SessionTaskHandle>>,
    epoch_index: Mutex<HashMap<RuntimeSessionEpoch, String>>,
}

impl SessionLayer {
    /// Production assembly: no committed epoch yet (fail-closed at the
    /// bootstrap deadline until the bootstrap lane supplies the epoch
    /// source), default timing/budgets and the health-ledger consumer.
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
        for consumer in &options.consumers {
            let mailbox = ConsumerMailbox::spawn(Arc::clone(consumer), pre_auth_limit);
            mailboxes.insert(consumer.kind(), mailbox);
        }

        let directory = RuntimeRegistrationDirectory::new(&options.manifest);
        let pre_auth = PreAuthPool::new(pre_auth_limit);
        let (shutdown_tx, _) = watch::channel(());
        let (fail_stop_tx, _) = watch::channel(None);
        let bootstrap_provider = RuntimeBootstrapProvider::new(&config);

        Ok(Self {
            bootstrap_provider,
            committed_epoch: options.committed_epoch,
            epoch_store: Mutex::new(None),
            pending_epoch: options.pending_epoch,
            manifest: options.manifest,
            directory: Mutex::new(directory),
            pre_auth: Mutex::new(pre_auth),
            health: Arc::new(RuntimeHealthLedger::new()),
            mailboxes,
            demux: RuntimeFrameDemux,
            registration_sink: RegistrationFrameSink,
            timing: options.timing,
            budgets: options.budgets,
            writer_delay: options.writer_delay,
            next_connection_id: AtomicU64::new(0),
            next_generation: AtomicU64::new(0),
            shutdown_tx,
            fail_stop: Mutex::new(None),
            fail_stop_tx,
            handles: Mutex::new(HashMap::new()),
            epoch_index: Mutex::new(HashMap::new()),
        })
    }

    /// The current committed tuple: from the epoch store when wired, else the
    /// static test seam. This is the W-bootstrap seam; session state-machine
    /// logic is unchanged.
    fn current_tuple(&self) -> Option<RegisteredAssemblyTuple> {
        if let Some(store) = self
            .epoch_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return store.capture().map(|epoch| epoch.registered_tuple());
        }
        self.committed_epoch.clone()
    }

    /// W-bootstrap seam: attach the single-authority epoch store. Subsequent
    /// bootstrap bytes and register-validation contexts capture the current
    /// epoch from the store; no session state-machine logic is touched.
    pub fn attach_epoch_store(&self, store: Arc<ActiveRoutingEpochStore>) {
        *self
            .epoch_store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(store);
    }

    pub fn bootstrap_bytes(&self) -> Option<Vec<u8>> {
        let epoch = self.current_tuple()?;
        self.bootstrap_provider.build(&epoch).ok()
    }

    pub fn epoch_context(&self) -> EpochContext {
        EpochContext {
            current: self.current_tuple(),
            pending: self.pending_epoch.clone(),
        }
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
            health_before_ack: self.health.dropped_before_ack_total(),
            fail_stop: self.fail_stop_reason(),
            live_session_tasks: self
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
        }
    }

    pub fn candidates(&self, tuple: &RegisteredAssemblyTuple) -> Vec<RuntimeSessionEpoch> {
        self.directory_lock().candidates(tuple)
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
    pub health_before_ack: u64,
    pub fail_stop: Option<String>,
    pub live_session_tasks: usize,
}
