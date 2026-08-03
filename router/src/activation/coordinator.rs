//! `ActivationCoordinator`: durable activation transaction lifecycle and
//! live/recovery participant binding (authority design §4.1/§4.2,
//! C-activation-coordinator).
//!
//! Owner invariants:
//! 1. One live transaction per environment (the durable pending slot is the
//!    single slot; CAS enforces it).
//! 2. Once the durable commit CAS is issued the outcome is durable
//!    authoritative: disconnect/timeout reconcile through durable state and
//!    never assume abort.
//! 3. The active epoch changes only through the publish port's verified,
//!    infallible whole-pointer swap; the coordinator keeps no eligibility
//!    cache and no pending publication token.
//! 4. Post-decision enqueue failures abort the exact session; committed
//!    durable state is never rolled back.
//! 5. The coordinator awaits only its own persistence/loader ports and never
//!    holds another owner's state across `.await`; session enqueues are
//!    synchronous and non-blocking.
//!
//! The coordinator is an actor with a bounded mailbox: external events
//! (start/ACK/disconnect/replacement/register/timeout/shutdown/hard-abort)
//! are delivered with `try_send`, and internal continuation events are
//! queued through the same mailbox so that external events observed between
//! the durable commit decision and the commit/abort enqueues are processed
//! first (decision-after semantics stay exact).

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRequest, AssemblyActivationServiceDb,
};
use tokio::sync::{mpsc, watch};
use tokio::time::{sleep_until, Instant};

use crate::artifact::ActorRoutingProjectionRef;
use crate::bootstrap::{
    ActiveRoutingEpochStore, BlockingLoader, BlockingLoaderError, BootstrapStrictLoader,
    RoutingEpoch,
};
use crate::dispatch::CandidateViewSource;
use crate::routing::{CandidateQuery, DispatchMode, RegisteredSessionLease, RuntimeCandidateQuery};
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::observer::RegistrationObserver;
use crate::session::{ConsumerKind, SessionConsumer};

use super::recovery::{project_recovery, CandidateEpochRefs};
use super::repository::{AbortInput, ActivationStateRepository, CommitInput, PrepareInput};

/// Participant binding frozen at candidate-query time (plan §3.4,
/// C-model-activation §3). The session epoch is coordinator-internal and
/// never appears on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActivationParticipantBinding {
    pub replica_id: String,
    pub session_epoch: RuntimeSessionEpoch,
}

/// Non-blocking writer enqueue outcome (C-model-activation §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    Ok,
    QueueFull,
}

/// Revalidation outcome used by the coordinator at plan §4.1 steps 4/5/7:
/// any session-epoch/registration-revision/tuple/cancellation change is
/// stale. The richer per-rule outcomes belong to dispatch admission; the
/// coordinator only needs the binary decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActivationRevalidateOutcome {
    #[default]
    Ok,
    Stale,
}

/// Fail-closed candidate projection errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivationCandidateError {
    #[error("no active routing epoch published for environment {environment}")]
    NoEpoch { environment: String },
    #[error(
        "active routing epoch environment {actual} does not match activation environment {expected}"
    )]
    EnvironmentMismatch { expected: String, actual: String },
    #[error("candidate query projection failed: {0}")]
    Query(String),
}

/// Fail-closed blocking-loader outcomes for candidate epochs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateLoadError {
    #[error("candidate epoch is missing")]
    Missing,
    #[error("candidate epoch is malformed or failed strict load: {0}")]
    Malformed(String),
    #[error("candidate loader pool is saturated")]
    Saturated,
    #[error("candidate loader deadline elapsed")]
    Deadline,
    #[error("candidate loader is shut down")]
    Shutdown,
}

/// Public coordinator errors (synchronous delivery failures).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoordinatorError {
    #[error("activation coordinator mailbox is full")]
    MailboxFull,
    #[error("activation coordinator is shut down")]
    Shutdown,
    #[error("a live or recovery transaction is already in progress")]
    TransactionInProgress,
    #[error("invalid activation request: {0}")]
    InvalidRequest(String),
}

/// Transaction phase vocabulary (C-activation-coordinator §7 health).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivationPhase {
    #[default]
    Idle,
    /// Live/recovery startup: durable read, candidate load, freeze,
    /// revalidation, durable prepare and prepare enqueues.
    Freezing,
    /// All expected participants staged with prepare; awaiting ACKs.
    Prepared,
    /// Cold recovery installed; waiting for expected replicas to rebind.
    WaitingRecovery,
    /// Durable commit CAS in flight.
    Committing,
    Committed,
    Aborted,
    Failed,
    Shutdown,
    Exited,
}

impl ActivationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Freezing => "freezing",
            Self::Prepared => "prepared",
            Self::WaitingRecovery => "waitingRecovery",
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
            Self::Failed => "failed",
            Self::Shutdown => "shutdown",
            Self::Exited => "exited",
        }
    }
}

/// Durable decision state (C-activation-coordinator §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionState {
    #[default]
    Idle,
    Preparing,
    Committing,
    Committed,
    Aborted,
}

impl DecisionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Preparing => "preparing",
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
        }
    }
}

/// Coordinator health snapshot (C-activation-coordinator §7). Read-only to
/// consumers; never contains Mongo URLs, secrets or business payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationCoordinatorHealth {
    pub phase: ActivationPhase,
    pub environment: Option<String>,
    pub activation_id: Option<String>,
    pub expected_generation: Option<u64>,
    pub candidate_generation: Option<u64>,
    pub participant_bindings: usize,
    pub prepared_acks: usize,
    pub reject_acks: usize,
    pub stale_acks: u64,
    pub session_aborts: u64,
    pub decision: DecisionState,
    pub recovery_active: bool,
    pub rebound_participants: usize,
    pub waiting_replicas: Vec<String>,
    pub readiness: bool,
    pub mailbox_occupancy: usize,
    pub mailbox_capacity: usize,
    pub mailbox_saturation: u64,
    pub shutdown: bool,
    pub last_failure: Option<String>,
}

impl Default for ActivationCoordinatorHealth {
    fn default() -> Self {
        Self {
            phase: ActivationPhase::Idle,
            environment: None,
            activation_id: None,
            expected_generation: None,
            candidate_generation: None,
            participant_bindings: 0,
            prepared_acks: 0,
            reject_acks: 0,
            stale_acks: 0,
            session_aborts: 0,
            decision: DecisionState::Idle,
            recovery_active: false,
            rebound_participants: 0,
            waiting_replicas: Vec::new(),
            readiness: false,
            mailbox_occupancy: 0,
            mailbox_capacity: 0,
            mailbox_saturation: 0,
            shutdown: false,
            last_failure: None,
        }
    }
}

/// Blocking loader port (C-activation-coordinator §8): loads and strictly
/// validates a candidate `RoutingEpoch`. The production adapter runs the
/// strict loader through the shared bounded `BlockingLoader` pool.
#[async_trait]
pub trait BlockingLoaderPort: Send + Sync + fmt::Debug {
    async fn load_candidate(
        &self,
        refs: &CandidateEpochRefs,
    ) -> Result<Arc<RoutingEpoch>, CandidateLoadError>;
}

/// Candidate query port (C-activation-coordinator §8): freezes exact leases
/// for the current epoch and revalidates the frozen participant set.
pub trait RuntimeCandidateQueryPort: Send + Sync + fmt::Debug {
    /// Captures the current whole epoch and freezes the exact matching
    /// `RegisteredSessionLease` set. Empty results are the fail-closed signal.
    fn freeze(
        &self,
        environment: &str,
    ) -> Result<Vec<RegisteredSessionLease>, ActivationCandidateError>;

    /// Re-checks session epoch, registration revision, exact tuple and
    /// cancellation for the frozen participant set.
    fn revalidate(
        &self,
        activation_id: &str,
        frozen: &[ActivationParticipantBinding],
    ) -> ActivationRevalidateOutcome;
}

/// Non-blocking activation writer port per exact session
/// (C-model-activation §7 transaction sink shape). `abort_session` is the
/// exact-session fence used when an enqueue is refused.
pub trait SessionEnqueuePort: Send + Sync + fmt::Debug {
    fn enqueue_prepare(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult;
    fn enqueue_commit(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult;
    fn enqueue_abort(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult;
    fn abort_session(&self, session: &RuntimeSessionEpoch);
}

/// Publish port: verified, infallible atomic `Arc` swap
/// (`ActiveRoutingEpochStore::publish`).
pub trait PublishCommittedEpochPort: Send + Sync + fmt::Debug {
    fn publish(&self, epoch: Arc<RoutingEpoch>);
}

/// Owner-published health sink (aggregation/telemetry seam).
pub trait HealthSinkPort: Send + Sync + fmt::Debug {
    fn publish(&self, health: &ActivationCoordinatorHealth);
}

/// No-op health sink for unit/corpus harnesses.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopHealthSink;

impl HealthSinkPort for NoopHealthSink {
    fn publish(&self, _health: &ActivationCoordinatorHealth) {}
}

/// Production adapter for the publish port over the single-authority epoch
/// store.
#[derive(Debug, Clone)]
pub struct EpochStorePublishPort {
    store: Arc<ActiveRoutingEpochStore>,
}

impl EpochStorePublishPort {
    pub fn new(store: Arc<ActiveRoutingEpochStore>) -> Self {
        Self { store }
    }
}

impl PublishCommittedEpochPort for EpochStorePublishPort {
    fn publish(&self, epoch: Arc<RoutingEpoch>) {
        self.store.publish(epoch);
    }
}

/// Production adapter for the loader port: strict candidate epoch load
/// through the shared bounded blocking pool.
#[derive(Debug, Clone)]
pub struct BlockingLoaderCandidatePort {
    loader: Arc<BlockingLoader>,
    strict_loader: Arc<BootstrapStrictLoader>,
    actor_projection: ActorRoutingProjectionRef,
}

impl BlockingLoaderCandidatePort {
    pub fn new(
        loader: Arc<BlockingLoader>,
        strict_loader: Arc<BootstrapStrictLoader>,
        actor_projection: ActorRoutingProjectionRef,
    ) -> Self {
        Self {
            loader,
            strict_loader,
            actor_projection,
        }
    }
}

#[async_trait]
impl BlockingLoaderPort for BlockingLoaderCandidatePort {
    async fn load_candidate(
        &self,
        refs: &CandidateEpochRefs,
    ) -> Result<Arc<RoutingEpoch>, CandidateLoadError> {
        let strict_loader = Arc::clone(&self.strict_loader);
        let actor_projection = self.actor_projection.clone();
        let refs = refs.clone();
        let environment = refs.environment.clone();
        self.loader
            .run(move || {
                strict_loader.load_epoch(
                    &environment,
                    refs.generation,
                    &refs.assembly,
                    &refs.config_snapshot,
                    &actor_projection,
                )
            })
            .await
            .map_err(|error| match error {
                BlockingLoaderError::Saturated => CandidateLoadError::Saturated,
                BlockingLoaderError::Deadline => CandidateLoadError::Deadline,
                BlockingLoaderError::Shutdown => CandidateLoadError::Shutdown,
                BlockingLoaderError::Join(message) => CandidateLoadError::Malformed(message),
                BlockingLoaderError::Operation(error) => {
                    CandidateLoadError::Malformed(error.to_string())
                }
            })
    }
}

/// Production adapter for the candidate query port.
///
/// Captures the current epoch from `ActiveRoutingEpochStore`, snapshots the
/// coherent directory view through the caller-supplied
/// [`CandidateViewSource`] (E-activation wires the W-session directory lock
/// and the capabilities binding), and freezes the exact leases. Activation
/// participants are all exact sessions: the frozen projection is the union
/// over both dispatch modes, de-duplicated by session epoch. The revalidate
/// step compares the current exact `(replica_id, connection_generation)`
/// set against the frozen bindings.
#[derive(Debug, Clone)]
pub struct RoutingCandidateQueryPortAdapter {
    epoch_store: Arc<ActiveRoutingEpochStore>,
    view_source: Arc<dyn CandidateViewSource>,
}

impl RoutingCandidateQueryPortAdapter {
    pub fn new(
        epoch_store: Arc<ActiveRoutingEpochStore>,
        view_source: Arc<dyn CandidateViewSource>,
    ) -> Self {
        Self {
            epoch_store,
            view_source,
        }
    }

    fn freeze_with_epoch(
        &self,
        epoch: &Arc<RoutingEpoch>,
        environment: &str,
    ) -> Result<Vec<RegisteredSessionLease>, ActivationCandidateError> {
        if epoch.environment() != environment {
            return Err(ActivationCandidateError::EnvironmentMismatch {
                expected: environment.to_string(),
                actual: epoch.environment().to_string(),
            });
        }
        let view = self.view_source.view();
        let query = RuntimeCandidateQuery;
        let mut leases = Vec::new();
        let mut seen = HashSet::new();
        for deployment in epoch.deployment_projection() {
            for mode in [DispatchMode::Unary, DispatchMode::ServerStream] {
                let candidates = query
                    .query(
                        epoch,
                        &view,
                        &CandidateQuery {
                            mode,
                            deployment: deployment.clone(),
                        },
                    )
                    .map_err(|error| ActivationCandidateError::Query(error.to_string()))?;
                for lease in candidates {
                    if seen.insert(lease.session_epoch.clone()) {
                        leases.push(lease);
                    }
                }
            }
        }
        Ok(leases)
    }
}

impl RuntimeCandidateQueryPort for RoutingCandidateQueryPortAdapter {
    fn freeze(
        &self,
        environment: &str,
    ) -> Result<Vec<RegisteredSessionLease>, ActivationCandidateError> {
        let epoch =
            self.epoch_store
                .capture()
                .ok_or_else(|| ActivationCandidateError::NoEpoch {
                    environment: environment.to_string(),
                })?;
        self.freeze_with_epoch(&epoch, environment)
    }

    fn revalidate(
        &self,
        _activation_id: &str,
        frozen: &[ActivationParticipantBinding],
    ) -> ActivationRevalidateOutcome {
        let current = match self.epoch_store.capture() {
            Some(epoch) => self
                .freeze_with_epoch(&epoch, epoch.environment())
                .map(|leases| {
                    leases
                        .into_iter()
                        .map(|lease| lease.session_epoch)
                        .collect::<Vec<_>>()
                }),
            None => Err(ActivationCandidateError::NoEpoch {
                environment: "<unknown>".to_string(),
            }),
        };
        let current = match current {
            Ok(current) => current,
            Err(_) => return ActivationRevalidateOutcome::Stale,
        };
        let frozen_set = frozen
            .iter()
            .map(|binding| {
                (
                    binding.replica_id.clone(),
                    binding.session_epoch.connection_generation,
                )
            })
            .collect::<BTreeSet<_>>();
        let current_set = current
            .iter()
            .map(|session| (session.replica_id.clone(), session.connection_generation))
            .collect::<BTreeSet<_>>();
        if frozen_set == current_set {
            ActivationRevalidateOutcome::Ok
        } else {
            ActivationRevalidateOutcome::Stale
        }
    }
}

/// Coordinator port bundle (C-activation-coordinator §8 fake seam).
pub struct ActivationCoordinatorPorts {
    pub repository: Arc<dyn ActivationStateRepository>,
    pub loader: Arc<dyn BlockingLoaderPort>,
    pub candidates: Arc<dyn RuntimeCandidateQueryPort>,
    pub sessions: Arc<dyn SessionEnqueuePort>,
    pub publish: Arc<dyn PublishCommittedEpochPort>,
    pub health: Arc<dyn HealthSinkPort>,
}

impl fmt::Debug for ActivationCoordinatorPorts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationCoordinatorPorts")
            .field("repository", &"ActivationStateRepository")
            .field("loader", &"BlockingLoaderPort")
            .field("candidates", &"RuntimeCandidateQueryPort")
            .field("sessions", &"SessionEnqueuePort")
            .field("publish", &"PublishCommittedEpochPort")
            .field("health", &"HealthSinkPort")
            .finish()
    }
}

/// Coordinator configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationCoordinatorOptions {
    pub mailbox_capacity: usize,
    pub ack_deadline: Duration,
    /// Optional `serviceDb` carried on Prepare/Commit wire controls
    /// (C-model-activation §2; defaults to `None` for tests).
    pub service_db_mongo_url: Option<String>,
}

impl Default for ActivationCoordinatorOptions {
    fn default() -> Self {
        Self {
            mailbox_capacity: 64,
            ack_deadline: Duration::from_secs(60),
            service_db_mongo_url: None,
        }
    }
}

/// Shared handle state: phase/health watch + mailbox occupancy counters.
struct CoordinatorShared {
    phase_tx: watch::Sender<ActivationPhase>,
    phase_rx: watch::Receiver<ActivationPhase>,
    health_tx: watch::Sender<ActivationCoordinatorHealth>,
    health_rx: watch::Receiver<ActivationCoordinatorHealth>,
    health: Mutex<ActivationCoordinatorHealth>,
    queued: AtomicUsize,
    saturation: AtomicU64,
    shutdown: AtomicBool,
    handles: AtomicUsize,
}

impl fmt::Debug for CoordinatorShared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoordinatorShared")
            .field("phase", &*self.phase_rx.borrow())
            .field("shutdown", &self.shutdown.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Commit,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AckKind {
    Prepared,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedSession {
    replica_id: String,
    session_epoch: RuntimeSessionEpoch,
}

/// Authoritative per-transaction state (owned by the actor task only).
struct TransactionState {
    environment: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    assembly: skiff_artifact_model::RuntimeAssemblyRef,
    config_snapshot: skiff_artifact_model::RuntimeConfigSnapshotRef,
    recovery: bool,
    candidate_epoch: Option<Arc<RoutingEpoch>>,
    participants: BTreeSet<String>,
    bindings: Vec<ActivationParticipantBinding>,
    staged: Vec<StagedSession>,
    prepared: BTreeSet<String>,
    rejected: BTreeSet<String>,
    waiting: BTreeSet<String>,
    rebound: usize,
    decision: Option<Decision>,
}

impl fmt::Debug for TransactionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransactionState")
            .field("environment", &self.environment)
            .field("activation_id", &self.activation_id)
            .field("expected_generation", &self.expected_generation)
            .field("candidate_generation", &self.candidate_generation)
            .field("recovery", &self.recovery)
            .field("participants", &self.participants)
            .field("bindings", &self.bindings)
            .field("staged", &self.staged)
            .field("prepared", &self.prepared)
            .field("rejected", &self.rejected)
            .field("waiting", &self.waiting)
            .field("rebound", &self.rebound)
            .field("decision", &self.decision)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
struct Counters {
    stale_acks: u64,
    session_aborts: u64,
}

enum Event {
    StartLive(AssemblyActivationRequest),
    StartRecovery(String),
    Ack {
        source: RuntimeSessionEpoch,
        control: AssemblyActivationControl,
    },
    Disconnect(RuntimeSessionEpoch),
    Replacement {
        replica_id: String,
        new_session: RuntimeSessionEpoch,
    },
    Register(ActivationParticipantBinding),
    ForceTimeout,
    Shutdown,
    HardAbort,
    Internal(InternalEvent),
}

enum InternalEvent {
    PublishAndCommitEnqueue,
    AbortEnqueue,
}

enum AckVerdict {
    Stale,
    Accept(AckKind, String),
}

struct CoordinatorActor {
    shared: Arc<CoordinatorShared>,
    ports: ActivationCoordinatorPorts,
    options: ActivationCoordinatorOptions,
    events_tx: mpsc::Sender<Event>,
    events_rx: mpsc::Receiver<Event>,
    tx: Option<TransactionState>,
    counters: Counters,
    last_failure: Option<String>,
}

/// Cloneable coordinator handle. Events are delivered non-blocking; durable
/// outcomes and phase changes are observable through health/phase watches.
pub struct ActivationCoordinatorHandle {
    events: mpsc::Sender<Event>,
    shared: Arc<CoordinatorShared>,
}

impl fmt::Debug for ActivationCoordinatorHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationCoordinatorHandle")
            .field("phase", &self.phase())
            .field("shutdown", &self.shared.shutdown.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl Clone for ActivationCoordinatorHandle {
    fn clone(&self) -> Self {
        self.shared.handles.fetch_add(1, Ordering::SeqCst);
        Self {
            events: self.events.clone(),
            shared: Arc::clone(&self.shared),
        }
    }
}

impl Drop for ActivationCoordinatorHandle {
    fn drop(&mut self) {
        // The last handle outlives the actor: stop it with process-exit
        // semantics so the spawned task never leaks.
        if self.shared.handles.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _ = self.events.try_send(Event::HardAbort);
        }
    }
}

/// Spawn point (one coordinator per environment; the durable pending slot is
/// the single slot).
pub struct ActivationCoordinator;

impl ActivationCoordinator {
    pub fn spawn(
        ports: ActivationCoordinatorPorts,
        options: ActivationCoordinatorOptions,
    ) -> ActivationCoordinatorHandle {
        let capacity = options.mailbox_capacity.max(1);
        let (events_tx, events_rx) = mpsc::channel(capacity);
        let (phase_tx, phase_rx) = watch::channel(ActivationPhase::Idle);
        let (health_tx, health_rx) = watch::channel(ActivationCoordinatorHealth::default());
        let shared = Arc::new(CoordinatorShared {
            phase_tx,
            phase_rx,
            health_tx,
            health_rx,
            health: Mutex::new(ActivationCoordinatorHealth::default()),
            queued: AtomicUsize::new(0),
            saturation: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            handles: AtomicUsize::new(1),
        });
        let actor = CoordinatorActor {
            shared: Arc::clone(&shared),
            ports,
            options,
            events_tx: events_tx.clone(),
            events_rx,
            tx: None,
            counters: Counters::default(),
            last_failure: None,
        };
        tokio::spawn(actor.run());
        ActivationCoordinatorHandle {
            events: events_tx,
            shared,
        }
    }
}

impl ActivationCoordinatorHandle {
    /// Begins a live activation transaction (plan §4.1). Returns after the
    /// request is accepted into the coordinator mailbox; durable outcomes
    /// are observable through `wait_for_phase`/health.
    pub fn start_live(&self, request: AssemblyActivationRequest) -> Result<(), CoordinatorError> {
        request
            .validate()
            .map_err(CoordinatorError::InvalidRequest)?;
        self.check_can_begin()?;
        self.send(Event::StartLive(request))
    }

    /// Begins cold recovery for one environment (plan §4.2): committed epoch
    /// is published first, then a durable pending (if any) is installed as a
    /// recovery transaction.
    pub fn start_recovery(&self, environment: impl Into<String>) -> Result<(), CoordinatorError> {
        self.check_can_begin()?;
        self.send(Event::StartRecovery(environment.into()))
    }

    /// Delivers a `Prepared`/`Reject` ACK from a Runtime session. Stale/new
    /// session ACKs are rejected (counter + health) with no durable effect.
    pub fn deliver_ack(
        &self,
        source: &RuntimeSessionEpoch,
        control: AssemblyActivationControl,
    ) -> Result<(), CoordinatorError> {
        self.send(Event::Ack {
            source: source.clone(),
            control,
        })
    }

    /// Session close notification (the `SessionConsumer` fence). A
    /// pre-decision disconnect triggers the durable abort path; after the
    /// decision the durable outcome is authoritative.
    pub fn notify_session_closed(
        &self,
        session: &RuntimeSessionEpoch,
    ) -> Result<(), CoordinatorError> {
        self.send(Event::Disconnect(session.clone()))
    }

    /// Same-replica replacement notification: the replica now has a new
    /// session epoch (new generation). Pre-decision replacement aborts.
    pub fn notify_session_replaced(
        &self,
        replica_id: &str,
        new_session: RuntimeSessionEpoch,
    ) -> Result<(), CoordinatorError> {
        self.send(Event::Replacement {
            replica_id: replica_id.to_string(),
            new_session,
        })
    }

    /// Expected replica registration during cold recovery: binds a new exact
    /// session (epoch may change) and sends prepare.
    pub fn register_recovery_session(
        &self,
        binding: ActivationParticipantBinding,
    ) -> Result<(), CoordinatorError> {
        self.send(Event::Register(binding))
    }

    /// Test/control seam: forces the pre-decision ACK timeout terminal.
    pub fn force_ack_timeout(&self) -> Result<(), CoordinatorError> {
        self.send(Event::ForceTimeout)
    }

    /// Graceful shutdown: no new transactions; an in-flight pre-decision
    /// pending is durably aborted, a decided commit is published/enqueued.
    pub fn shutdown(&self) -> Result<(), CoordinatorError> {
        self.send(Event::Shutdown)
    }

    /// Hard stop with process-exit semantics: abandons in-flight enqueues
    /// and marks the coordinator `Exited` (durable reconcile happens at the
    /// next startup).
    pub fn hard_abort(&self) -> Result<(), CoordinatorError> {
        self.send(Event::HardAbort)
    }

    pub fn health(&self) -> ActivationCoordinatorHealth {
        self.shared
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn phase(&self) -> ActivationPhase {
        *self.shared.phase_rx.borrow()
    }

    pub async fn wait_for_phase(
        &self,
        predicate: impl Fn(ActivationPhase) -> bool,
    ) -> ActivationPhase {
        let mut rx = self.shared.phase_rx.clone();
        loop {
            let phase = *rx.borrow();
            if predicate(phase) {
                return phase;
            }
            if rx.changed().await.is_err() {
                return *rx.borrow();
            }
        }
    }

    pub async fn wait_until_health(
        &self,
        predicate: impl Fn(&ActivationCoordinatorHealth) -> bool,
    ) -> ActivationCoordinatorHealth {
        let mut rx = self.shared.health_rx.clone();
        loop {
            let health = self.health();
            if predicate(&health) {
                return health;
            }
            if rx.changed().await.is_err() {
                return self.health();
            }
        }
    }

    fn check_can_begin(&self) -> Result<(), CoordinatorError> {
        if self.shared.shutdown.load(Ordering::SeqCst) {
            return Err(CoordinatorError::Shutdown);
        }
        match self.phase() {
            ActivationPhase::Idle
            | ActivationPhase::Committed
            | ActivationPhase::Aborted
            | ActivationPhase::Failed => Ok(()),
            ActivationPhase::Shutdown | ActivationPhase::Exited => Err(CoordinatorError::Shutdown),
            ActivationPhase::Freezing
            | ActivationPhase::Prepared
            | ActivationPhase::WaitingRecovery
            | ActivationPhase::Committing => Err(CoordinatorError::TransactionInProgress),
        }
    }

    fn send(&self, event: Event) -> Result<(), CoordinatorError> {
        self.shared.queued.fetch_add(1, Ordering::AcqRel);
        if self.events.try_send(event).is_ok() {
            Ok(())
        } else {
            self.shared.queued.fetch_sub(1, Ordering::AcqRel);
            self.shared.saturation.fetch_add(1, Ordering::Relaxed);
            Err(CoordinatorError::MailboxFull)
        }
    }
}

impl SessionConsumer for ActivationCoordinatorHandle {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::ActivationCoordinator
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        self.notify_session_closed(session)
            .map_err(|error| error.to_string())
    }
}

/// Cold recovery rebind seam (plan §4.2): the session layer notifies the
/// coordinator when a Runtime registration becomes routable. The coordinator
/// binds expected replicas into the recovery transaction (non-expected
/// replicas are ignored by `on_register`); the callback is non-blocking and
/// mailbox saturation only affects recovery health, never the session.
impl RegistrationObserver for ActivationCoordinatorHandle {
    fn on_session_registered(&self, session: &RuntimeSessionEpoch) {
        let binding = ActivationParticipantBinding {
            replica_id: session.replica_id.clone(),
            session_epoch: session.clone(),
        };
        let _ = self.register_recovery_session(binding);
    }
}

impl CoordinatorActor {
    async fn run(mut self) {
        let mut ack_deadline: Option<Instant> = None;
        loop {
            let event = if let Some(deadline) = ack_deadline {
                tokio::select! {
                    event = self.events_rx.recv() => match event {
                        Some(event) => event,
                        None => return,
                    },
                    _ = sleep_until(deadline) => Event::ForceTimeout,
                }
            } else {
                match self.events_rx.recv().await {
                    Some(event) => event,
                    None => return,
                }
            };
            self.shared.queued.fetch_sub(1, Ordering::AcqRel);
            self.process(event).await;
            ack_deadline = self.next_ack_deadline();
            self.publish_health();
            if self.should_stop() {
                return;
            }
        }
    }

    fn should_stop(&self) -> bool {
        match self.phase() {
            ActivationPhase::Shutdown | ActivationPhase::Exited => true,
            ActivationPhase::Committed | ActivationPhase::Aborted | ActivationPhase::Failed
                if self.shared.shutdown.load(Ordering::SeqCst) =>
            {
                true
            }
            _ => false,
        }
    }

    fn next_ack_deadline(&self) -> Option<Instant> {
        let awaiting_acks = self.phase() == ActivationPhase::Prepared
            && self.tx.as_ref().is_some_and(|tx| tx.decision.is_none());
        awaiting_acks.then(|| Instant::now() + self.options.ack_deadline)
    }

    async fn process(&mut self, event: Event) {
        match event {
            Event::StartLive(request) => self.on_start_live(request).await,
            Event::StartRecovery(environment) => self.on_start_recovery(environment).await,
            Event::Ack { source, control } => self.on_ack(&source, &control).await,
            Event::Disconnect(session) => self.on_disconnect(&session).await,
            Event::Replacement {
                replica_id,
                new_session,
            } => self.on_replacement(&replica_id, &new_session).await,
            Event::Register(binding) => self.on_register(&binding).await,
            Event::ForceTimeout => self.on_timeout().await,
            Event::Shutdown => self.on_shutdown().await,
            Event::HardAbort => self.on_hard_abort(),
            Event::Internal(internal) => self.on_internal(internal).await,
        }
    }

    async fn on_start_live(&mut self, request: AssemblyActivationRequest) {
        if !self.can_begin() {
            return;
        }
        self.reset_counters();
        self.set_phase(ActivationPhase::Freezing);
        let state = match self.ports.repository.read(&request.environment).await {
            Ok(state) => state,
            Err(error) => {
                self.fail(format!("durable activation state read failed: {error}"));
                return;
            }
        };
        if state.committed.generation != request.expected_generation {
            self.fail(format!(
                "committed generation {} does not match request expected generation {}",
                state.committed.generation, request.expected_generation
            ));
            return;
        }
        let candidate_generation = match request.expected_generation.checked_add(1) {
            Some(generation) => generation,
            None => {
                self.fail("candidate generation overflow".to_string());
                return;
            }
        };
        let refs = CandidateEpochRefs {
            environment: request.environment.clone(),
            generation: candidate_generation,
            assembly: request.assembly.clone(),
            config_snapshot: request.config_snapshot.clone(),
        };
        let candidate_epoch = match self.ports.loader.load_candidate(&refs).await {
            Ok(epoch) => epoch,
            Err(error) => {
                self.fail(format!("candidate epoch load failed: {error}"));
                return;
            }
        };
        let leases = match self.ports.candidates.freeze(&request.environment) {
            Ok(leases) => leases,
            Err(error) => {
                self.fail(format!("candidate freeze failed: {error}"));
                return;
            }
        };
        if leases.is_empty() {
            self.fail("no exact candidate sessions for the current epoch".to_string());
            return;
        }
        for lease in &leases {
            let tuple = &lease.exact_registered_tuple;
            if tuple.environment != request.environment
                || tuple.generation != state.committed.generation
            {
                self.fail(
                    "candidate lease tuple does not match the durable committed epoch".to_string(),
                );
                return;
            }
        }
        let bindings = leases
            .into_iter()
            .map(|lease| ActivationParticipantBinding {
                replica_id: lease.session_epoch.replica_id.clone(),
                session_epoch: lease.session_epoch,
            })
            .collect::<Vec<_>>();
        if bindings.is_empty() {
            self.fail("no exact participant bindings".to_string());
            return;
        }
        if self
            .ports
            .candidates
            .revalidate(&request.activation_id, &bindings)
            != ActivationRevalidateOutcome::Ok
        {
            self.fail("candidate revalidation failed before durable prepare".to_string());
            return;
        }
        let participants = bindings
            .iter()
            .map(|binding| binding.replica_id.clone())
            .collect::<BTreeSet<_>>();
        let prepare_input = PrepareInput {
            environment: request.environment.clone(),
            activation_id: request.activation_id.clone(),
            expected_generation: request.expected_generation,
            candidate_generation,
            assembly: request.assembly.clone(),
            config_snapshot: request.config_snapshot.clone(),
            participant_replica_ids: participants.iter().cloned().collect(),
        };
        if let Err(error) = self.ports.repository.prepare(prepare_input).await {
            self.fail(format!("durable prepare CAS failed: {error}"));
            return;
        }
        if self
            .ports
            .candidates
            .revalidate(&request.activation_id, &bindings)
            != ActivationRevalidateOutcome::Ok
        {
            // Pending was already written; step-5 revalidation failure is the
            // decision-before abort path (C-activation-coordinator §3 step 5).
            self.durable_abort_and_enqueue().await;
            return;
        }
        self.tx = Some(TransactionState {
            environment: request.environment.clone(),
            activation_id: request.activation_id.clone(),
            expected_generation: request.expected_generation,
            candidate_generation,
            assembly: request.assembly,
            config_snapshot: request.config_snapshot,
            recovery: false,
            candidate_epoch: Some(candidate_epoch),
            participants,
            bindings,
            staged: Vec::new(),
            prepared: BTreeSet::new(),
            rejected: BTreeSet::new(),
            waiting: BTreeSet::new(),
            rebound: 0,
            decision: None,
        });
        if self.enqueue_prepare_all().await.is_err() {
            return;
        }
        self.set_phase(ActivationPhase::Prepared);
    }

    async fn on_start_recovery(&mut self, environment: String) {
        if !self.can_begin() {
            return;
        }
        self.reset_counters();
        self.set_phase(ActivationPhase::Freezing);
        let state = match self.ports.repository.read(&environment).await {
            Ok(state) => state,
            Err(error) => {
                self.fail(format!("durable activation state read failed: {error}"));
                return;
            }
        };
        let committed_refs = CandidateEpochRefs::committed(&state);
        let committed_epoch = match self.ports.loader.load_candidate(&committed_refs).await {
            Ok(epoch) => epoch,
            Err(error) => {
                self.fail(format!("committed epoch load failed: {error}"));
                return;
            }
        };
        // §4.2(1): committed epoch is constructed and published before any
        // pending recovery work; the public listener may open afterwards.
        self.ports.publish.publish(Arc::clone(&committed_epoch));
        let Some(recovery) = project_recovery(&state) else {
            self.set_phase(ActivationPhase::Committed);
            return;
        };
        let candidate_epoch = match self
            .ports
            .loader
            .load_candidate(&recovery.candidate_refs())
            .await
        {
            Ok(epoch) => epoch,
            Err(error) => {
                // §4.2(4): candidate load failure is the reducer durable
                // abort; the committed epoch stays published.
                self.fail(format!("recovery candidate load failed: {error}"));
                let input = AbortInput {
                    environment: environment.clone(),
                    activation_id: recovery.activation_id.clone(),
                    expected_generation: recovery.expected_generation,
                };
                match self.ports.repository.abort(input).await {
                    Ok(_) => self.set_phase(ActivationPhase::Aborted),
                    Err(abort_error) => {
                        self.fail(format!(
                            "recovery candidate load failed and durable abort failed: {abort_error}"
                        ));
                    }
                }
                return;
            }
        };
        let expected_replica_ids = recovery.expected_replica_ids.clone();
        self.tx = Some(TransactionState {
            environment: recovery.environment.clone(),
            activation_id: recovery.activation_id.clone(),
            expected_generation: recovery.expected_generation,
            candidate_generation: recovery.candidate_generation,
            assembly: recovery.assembly.clone(),
            config_snapshot: recovery.config_snapshot.clone(),
            recovery: true,
            candidate_epoch: Some(candidate_epoch),
            participants: expected_replica_ids.iter().cloned().collect(),
            bindings: Vec::new(),
            staged: Vec::new(),
            prepared: BTreeSet::new(),
            rejected: BTreeSet::new(),
            waiting: expected_replica_ids.into_iter().collect(),
            rebound: 0,
            decision: None,
        });
        self.set_phase(ActivationPhase::WaitingRecovery);
    }

    async fn on_ack(&mut self, source: &RuntimeSessionEpoch, control: &AssemblyActivationControl) {
        let verdict = self.evaluate_ack(source, control);
        let (kind, replica_id) = match verdict {
            AckVerdict::Stale => {
                self.counters.stale_acks += 1;
                return;
            }
            AckVerdict::Accept(kind, replica_id) => (kind, replica_id),
        };
        match kind {
            AckKind::Prepared => {
                self.tx
                    .as_mut()
                    .expect("accepted ACK requires a transaction")
                    .prepared
                    .insert(replica_id);
            }
            AckKind::Reject => {
                self.tx
                    .as_mut()
                    .expect("accepted ACK requires a transaction")
                    .rejected
                    .insert(replica_id);
                self.durable_abort_and_enqueue().await;
                return;
            }
        }
        let all_prepared = self
            .tx
            .as_ref()
            .is_some_and(|tx| tx.prepared == tx.participants && tx.rejected.is_empty());
        if !all_prepared {
            return;
        }
        let revalidate_ok = {
            let tx = self.tx.as_ref().expect("transaction exists");
            self.ports
                .candidates
                .revalidate(&tx.activation_id, &tx.bindings)
                == ActivationRevalidateOutcome::Ok
        };
        if !revalidate_ok {
            self.durable_abort_and_enqueue().await;
            return;
        }
        self.set_phase(ActivationPhase::Committing);
        let input = {
            let tx = self.tx.as_ref().expect("transaction exists");
            commit_input(tx)
        };
        match self.ports.repository.commit(input).await {
            Ok(_) => {
                self.tx.as_mut().expect("transaction exists").decision = Some(Decision::Commit);
                self.queue_internal(InternalEvent::PublishAndCommitEnqueue);
            }
            Err(_) => self.reconcile_after_durable_failure().await,
        }
    }

    fn evaluate_ack(
        &self,
        source: &RuntimeSessionEpoch,
        control: &AssemblyActivationControl,
    ) -> AckVerdict {
        let Some(tx) = self.tx.as_ref() else {
            return AckVerdict::Stale;
        };
        if tx.decision.is_some() {
            return AckVerdict::Stale;
        }
        let (kind, replica_id) = match control {
            AssemblyActivationControl::Prepared { replica_id, .. } => {
                (AckKind::Prepared, replica_id)
            }
            AssemblyActivationControl::Reject { replica_id, .. } => (AckKind::Reject, replica_id),
            _ => return AckVerdict::Stale,
        };
        if !control_matches_tx(control, tx) {
            return AckVerdict::Stale;
        }
        if !tx.participants.contains(replica_id)
            || !tx.bindings.iter().any(|binding| {
                binding.replica_id == *replica_id && &binding.session_epoch == source
            })
            || !tx
                .staged
                .iter()
                .any(|staged| staged.replica_id == *replica_id && &staged.session_epoch == source)
            || tx.prepared.contains(replica_id)
            || tx.rejected.contains(replica_id)
        {
            return AckVerdict::Stale;
        }
        AckVerdict::Accept(kind, replica_id.clone())
    }

    async fn on_disconnect(&mut self, session: &RuntimeSessionEpoch) {
        let affected = {
            let Some(tx) = self.tx.as_mut() else {
                return;
            };
            let before = tx.bindings.len();
            tx.bindings
                .retain(|binding| &binding.session_epoch != session);
            tx.bindings.len() != before
        };
        if affected && self.tx.as_ref().is_some_and(|tx| tx.decision.is_none()) {
            self.durable_abort_and_enqueue().await;
        }
    }

    async fn on_replacement(&mut self, replica_id: &str, new_session: &RuntimeSessionEpoch) {
        let affected = {
            let Some(tx) = self.tx.as_mut() else {
                return;
            };
            match tx
                .bindings
                .iter_mut()
                .find(|binding| binding.replica_id == replica_id)
            {
                Some(binding) => {
                    binding.session_epoch = new_session.clone();
                    true
                }
                None => false,
            }
        };
        if affected && self.tx.as_ref().is_some_and(|tx| tx.decision.is_none()) {
            self.durable_abort_and_enqueue().await;
        }
    }

    async fn on_register(&mut self, binding: &ActivationParticipantBinding) {
        let expected = self.tx.as_ref().is_some_and(|tx| {
            tx.recovery && tx.decision.is_none() && tx.waiting.contains(&binding.replica_id)
        });
        if !expected {
            return;
        }
        let control = self.prepare_control(binding);
        match self.ports.sessions.enqueue_prepare(binding, &control) {
            EnqueueResult::Ok => {
                let tx = self.tx.as_mut().expect("transaction exists");
                tx.waiting.remove(&binding.replica_id);
                if let Some(existing) = tx
                    .bindings
                    .iter_mut()
                    .find(|existing| existing.replica_id == binding.replica_id)
                {
                    existing.session_epoch = binding.session_epoch.clone();
                } else {
                    tx.bindings.push(binding.clone());
                }
                tx.staged.push(StagedSession {
                    replica_id: binding.replica_id.clone(),
                    session_epoch: binding.session_epoch.clone(),
                });
                tx.rebound += 1;
                if tx.waiting.is_empty() {
                    self.set_phase(ActivationPhase::Prepared);
                }
            }
            EnqueueResult::QueueFull => {
                self.ports.sessions.abort_session(&binding.session_epoch);
                self.counters.session_aborts += 1;
                self.durable_abort_and_enqueue().await;
            }
        }
    }

    async fn on_timeout(&mut self) {
        if self.tx.as_ref().is_some_and(|tx| tx.decision.is_none()) {
            self.durable_abort_and_enqueue().await;
        }
    }

    async fn on_shutdown(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        let decision = self.tx.as_ref().and_then(|tx| tx.decision);
        match decision {
            None => {
                if self.tx.is_some() {
                    self.durable_abort_and_enqueue().await;
                } else {
                    self.set_phase(ActivationPhase::Shutdown);
                }
            }
            Some(Decision::Commit) => self.finish_commit().await,
            Some(Decision::Abort) => self.finish_abort().await,
        }
    }

    fn on_hard_abort(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        self.set_phase(ActivationPhase::Exited);
    }

    async fn on_internal(&mut self, internal: InternalEvent) {
        match internal {
            InternalEvent::PublishAndCommitEnqueue => self.finish_commit().await,
            InternalEvent::AbortEnqueue => self.finish_abort().await,
        }
    }

    /// Non-blocking prepare enqueue for every frozen binding. A queue-full
    /// is the exact-session fence: abort that session, durably abort the
    /// pending, and enqueue abort to every staged exact session.
    async fn enqueue_prepare_all(&mut self) -> Result<(), ()> {
        let bindings = self
            .tx
            .as_ref()
            .map(|tx| tx.bindings.clone())
            .unwrap_or_default();
        for binding in &bindings {
            let control = self.prepare_control(binding);
            match self.ports.sessions.enqueue_prepare(binding, &control) {
                EnqueueResult::Ok => {
                    self.tx
                        .as_mut()
                        .expect("transaction exists")
                        .staged
                        .push(StagedSession {
                            replica_id: binding.replica_id.clone(),
                            session_epoch: binding.session_epoch.clone(),
                        });
                }
                EnqueueResult::QueueFull => {
                    self.ports.sessions.abort_session(&binding.session_epoch);
                    self.counters.session_aborts += 1;
                    self.durable_abort_and_enqueue().await;
                    return Err(());
                }
            }
        }
        Ok(())
    }

    async fn durable_abort_and_enqueue(&mut self) {
        let input = {
            let Some(tx) = self.tx.as_ref() else {
                return;
            };
            abort_input(tx)
        };
        match self.ports.repository.abort(input).await {
            Ok(_) => {
                self.tx.as_mut().expect("transaction exists").decision = Some(Decision::Abort);
                self.queue_internal(InternalEvent::AbortEnqueue);
            }
            Err(_) => self.reconcile_after_durable_failure().await,
        }
    }

    async fn reconcile_after_durable_failure(&mut self) {
        let environment = match self.tx.as_ref() {
            Some(tx) => tx.environment.clone(),
            None => {
                self.fail("durable reconcile without a transaction".to_string());
                return;
            }
        };
        match self.ports.repository.read(&environment).await {
            Ok(state) => {
                let Some(tx) = self.tx.as_ref() else {
                    return;
                };
                if state.pending.is_none() && state.committed.generation == tx.candidate_generation
                {
                    self.tx.as_mut().expect("transaction exists").decision = Some(Decision::Commit);
                    self.queue_internal(InternalEvent::PublishAndCommitEnqueue);
                } else if state.pending.is_none() {
                    self.tx.as_mut().expect("transaction exists").decision = Some(Decision::Abort);
                    self.queue_internal(InternalEvent::AbortEnqueue);
                } else {
                    self.fail(
                        "durable reconcile found a pending activation that is not ours".to_string(),
                    );
                }
            }
            Err(error) => self.fail(format!("durable reconcile read failed: {error}")),
        }
    }

    async fn finish_commit(&mut self) {
        let (candidate, staged, bindings) = {
            let Some(tx) = self.tx.as_ref() else {
                return;
            };
            if tx.decision != Some(Decision::Commit) {
                return;
            }
            let Some(candidate) = tx.candidate_epoch.clone() else {
                self.fail("commit decided without a loaded candidate epoch".to_string());
                return;
            };
            (candidate, tx.staged.clone(), tx.bindings.clone())
        };
        self.ports.publish.publish(candidate);
        for staged in staged {
            if !bindings.iter().any(|binding| {
                binding.replica_id == staged.replica_id
                    && binding.session_epoch == staged.session_epoch
            }) {
                continue;
            }
            let binding = ActivationParticipantBinding {
                replica_id: staged.replica_id.clone(),
                session_epoch: staged.session_epoch.clone(),
            };
            let control = self.commit_control(&binding);
            match self.ports.sessions.enqueue_commit(&binding, &control) {
                EnqueueResult::Ok => {}
                EnqueueResult::QueueFull => {
                    self.ports.sessions.abort_session(&binding.session_epoch);
                    self.counters.session_aborts += 1;
                }
            }
        }
        self.set_phase(ActivationPhase::Committed);
    }

    async fn finish_abort(&mut self) {
        let (staged, bindings, rejected) = {
            let Some(tx) = self.tx.as_ref() else {
                return;
            };
            if tx.decision != Some(Decision::Abort) {
                return;
            }
            (tx.staged.clone(), tx.bindings.clone(), tx.rejected.clone())
        };
        for staged in staged {
            if rejected.contains(&staged.replica_id)
                || !bindings.iter().any(|binding| {
                    binding.replica_id == staged.replica_id
                        && binding.session_epoch == staged.session_epoch
                })
            {
                continue;
            }
            let binding = ActivationParticipantBinding {
                replica_id: staged.replica_id.clone(),
                session_epoch: staged.session_epoch.clone(),
            };
            let control = self.abort_control(&binding);
            match self.ports.sessions.enqueue_abort(&binding, &control) {
                EnqueueResult::Ok => {}
                EnqueueResult::QueueFull => {
                    self.ports.sessions.abort_session(&binding.session_epoch);
                    self.counters.session_aborts += 1;
                }
            }
        }
        self.set_phase(ActivationPhase::Aborted);
    }

    fn prepare_control(&self, binding: &ActivationParticipantBinding) -> AssemblyActivationControl {
        let tx = self.tx.as_ref().expect("transaction exists");
        AssemblyActivationControl::Prepare {
            environment: tx.environment.clone(),
            activation_id: tx.activation_id.clone(),
            expected_generation: tx.expected_generation,
            candidate_generation: tx.candidate_generation,
            assembly: tx.assembly.clone(),
            config_snapshot: tx.config_snapshot.clone(),
            replica_id: binding.replica_id.clone(),
            service_db: self.service_db(),
        }
    }

    fn commit_control(&self, binding: &ActivationParticipantBinding) -> AssemblyActivationControl {
        let tx = self.tx.as_ref().expect("transaction exists");
        AssemblyActivationControl::Commit {
            environment: tx.environment.clone(),
            activation_id: tx.activation_id.clone(),
            expected_generation: tx.expected_generation,
            candidate_generation: tx.candidate_generation,
            assembly: tx.assembly.clone(),
            config_snapshot: tx.config_snapshot.clone(),
            replica_id: binding.replica_id.clone(),
            service_db: self.service_db(),
        }
    }

    fn abort_control(&self, binding: &ActivationParticipantBinding) -> AssemblyActivationControl {
        let tx = self.tx.as_ref().expect("transaction exists");
        AssemblyActivationControl::Abort {
            environment: tx.environment.clone(),
            activation_id: tx.activation_id.clone(),
            expected_generation: tx.expected_generation,
            candidate_generation: tx.candidate_generation,
            assembly: tx.assembly.clone(),
            config_snapshot: tx.config_snapshot.clone(),
            replica_id: binding.replica_id.clone(),
        }
    }

    fn service_db(&self) -> Option<AssemblyActivationServiceDb> {
        self.options
            .service_db_mongo_url
            .clone()
            .map(|mongo_url| AssemblyActivationServiceDb { mongo_url })
    }

    fn queue_internal(&self, internal: InternalEvent) {
        self.shared.queued.fetch_add(1, Ordering::AcqRel);
        if self.events_tx.try_send(Event::Internal(internal)).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::AcqRel);
            self.shared.saturation.fetch_add(1, Ordering::Relaxed);
            self.set_phase(ActivationPhase::Failed);
        }
    }

    fn can_begin(&self) -> bool {
        matches!(
            self.phase(),
            ActivationPhase::Idle
                | ActivationPhase::Committed
                | ActivationPhase::Aborted
                | ActivationPhase::Failed
        )
    }

    fn reset_counters(&mut self) {
        self.counters = Counters::default();
        self.last_failure = None;
    }

    fn fail(&mut self, reason: String) {
        self.last_failure = Some(reason);
        self.set_phase(ActivationPhase::Failed);
    }

    fn set_phase(&self, phase: ActivationPhase) {
        if *self.shared.phase_rx.borrow() != phase {
            let _ = self.shared.phase_tx.send(phase);
        }
    }

    fn phase(&self) -> ActivationPhase {
        *self.shared.phase_rx.borrow()
    }

    fn publish_health(&self) {
        let mut health = ActivationCoordinatorHealth {
            phase: self.phase(),
            mailbox_occupancy: self.shared.queued.load(Ordering::Relaxed),
            mailbox_capacity: self.options.mailbox_capacity,
            mailbox_saturation: self.shared.saturation.load(Ordering::Relaxed),
            shutdown: self.shared.shutdown.load(Ordering::SeqCst),
            stale_acks: self.counters.stale_acks,
            session_aborts: self.counters.session_aborts,
            last_failure: self.last_failure.clone(),
            ..ActivationCoordinatorHealth::default()
        };
        if let Some(tx) = &self.tx {
            health.environment = Some(tx.environment.clone());
            health.activation_id = Some(tx.activation_id.clone());
            health.expected_generation = Some(tx.expected_generation);
            health.candidate_generation = Some(tx.candidate_generation);
            health.participant_bindings = tx.bindings.len();
            health.prepared_acks = tx.prepared.len();
            health.reject_acks = tx.rejected.len();
            health.decision = match tx.decision {
                None if health.phase == ActivationPhase::Committing => DecisionState::Committing,
                None => DecisionState::Preparing,
                Some(Decision::Commit) => DecisionState::Committed,
                Some(Decision::Abort) => DecisionState::Aborted,
            };
            // Recovery is active only while the durable pending transaction is
            // undecided; after commit/abort the recovery contract is complete.
            health.recovery_active = tx.recovery && tx.decision.is_none();
            health.rebound_participants = tx.rebound;
            health.waiting_replicas = tx.waiting.iter().cloned().collect();
            health.readiness = tx.waiting.is_empty();
        }
        {
            let mut slot = self
                .shared
                .health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = health.clone();
        }
        let _ = self.shared.health_tx.send(health.clone());
        self.ports.health.publish(&health);
    }
}

fn control_matches_tx(control: &AssemblyActivationControl, tx: &TransactionState) -> bool {
    let (
        environment,
        activation_id,
        expected_generation,
        candidate_generation,
        assembly,
        config_snapshot,
    ) = match control {
        AssemblyActivationControl::Prepared {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            ..
        }
        | AssemblyActivationControl::Reject {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            ..
        } => (
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
        ),
        _ => return false,
    };
    *environment == tx.environment
        && *activation_id == tx.activation_id
        && *expected_generation == tx.expected_generation
        && *candidate_generation == tx.candidate_generation
        && *assembly == tx.assembly
        && *config_snapshot == tx.config_snapshot
}

fn commit_input(tx: &TransactionState) -> CommitInput {
    CommitInput {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        assembly: tx.assembly.clone(),
        config_snapshot: tx.config_snapshot.clone(),
        connected_replica_ids: tx
            .bindings
            .iter()
            .map(|binding| binding.replica_id.clone())
            .collect(),
        prepared_replica_ids: tx.prepared.iter().cloned().collect(),
    }
}

fn abort_input(tx: &TransactionState) -> AbortInput {
    AbortInput {
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
    }
}
