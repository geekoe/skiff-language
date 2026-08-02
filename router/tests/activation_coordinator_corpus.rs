//! Executable `ActivationCoordinator` corpus: the shared
//! `activation-transaction-cases.json` fixture (live 16 + coldRecovery 6)
//! driven against the real coordinator implementation.
//!
//! The corpus steps are split into two planes:
//! - script steps (readState / captureActiveEpoch / loadCandidate /
//!   queryCandidates / revalidate / durablePrepare / enqueuePrepare /
//!   durableCommit / publishEpoch / enqueueCommit / enqueueAbort) configure
//!   the test-double ports; durable CAS semantics run through the real
//!   in-memory repository reducer;
//! - event steps (ack / disconnect / replacement / timeout / register /
//!   shutdown / processExit) are delivered to the coordinator mailbox in
//!   corpus order.
//!
//! Deterministic interleaving for `live-disconnect-after-commit-reconciles`
//! and `cold-recovery-exit-after-commit-before-swap` uses a commit gate on
//! the fake repository: the driver waits for the commit to start, queues the
//! external event, then releases the gate, so the queued event is processed
//! before the coordinator's publish/commit-enqueue continuation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use serde::Deserialize;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRejectReason, AssemblyActivationRequest,
    AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly, RuntimeAssemblyRef,
    RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::activation_state::{
    EnvironmentActivationState, ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CommittedActivation, PendingActivation};
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, repository::AbortInput, repository::CommitInput,
    repository::PrepareInput, ActivationCandidateError, ActivationCoordinator,
    ActivationCoordinatorOptions, ActivationCoordinatorPorts, ActivationParticipantBinding,
    ActivationPhase, ActivationRevalidateOutcome, ActivationStateRepository, BlockingLoaderPort,
    CandidateEpochRefs, CandidateLoadError, EnqueueResult, EpochStorePublishPort, NoopHealthSink,
    PublishCommittedEpochPort, RepositoryError, RuntimeCandidateQueryPort, SessionEnqueuePort,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use skiff_router::routing::{DispatchCapabilities, RegisteredSessionLease, SessionCancellation};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};
use tokio::sync::{watch, RwLock};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    schema_version: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    name: String,
    contract: String,
    tx: Option<TxFixture>,
    steps: Option<Vec<Step>>,
    runs: Option<Vec<Run>>,
    expected: Option<Expected>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Run {
    tx: Option<TxFixture>,
    steps: Vec<Step>,
    expected: Expected,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TxFixture {
    environment: String,
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingFixture {
    activation_id: String,
    expected_generation: u64,
    candidate_generation: u64,
    participant_replica_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LeaseFixture {
    replica_id: String,
    session_epoch: u64,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expected {
    terminal: String,
    durable_state: DurableStateFixture,
    published: bool,
    #[serde(default)]
    listener_open: Option<bool>,
    #[serde(default)]
    readiness: Option<bool>,
    #[serde(default)]
    session_aborts: Vec<String>,
    #[serde(default)]
    enqueues: Vec<Vec<String>>,
    #[serde(default)]
    stale_acks: u64,
    #[serde(default)]
    recovery: Option<bool>,
    #[serde(default)]
    active_epoch: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableStateFixture {
    committed_generation: u64,
    pending: Option<PendingFixture>,
}

// The frozen corpus schema includes steps that no current case uses
// (DurableAbort / SessionAbort / PublishCommitted); they stay parseable so
// the fixture remains forward-compatible.
#[derive(Deserialize, Clone)]
#[allow(dead_code)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Step {
    ReadState {
        committed_generation: u64,
        pending: Option<PendingFixture>,
    },
    CaptureActiveEpoch {
        generation: u64,
    },
    LoadCandidate {
        result: String,
    },
    QueryCandidates {
        leases: Vec<LeaseFixture>,
    },
    Revalidate {
        result: String,
    },
    DurablePrepare {
        expected: String,
    },
    EnqueuePrepare {
        replica_id: String,
        result: String,
    },
    Ack {
        kind: String,
        replica_id: String,
        session_epoch: u64,
        expected: String,
    },
    DurableCommit {
        expected: String,
        #[serde(default)]
        durable_outcome: Option<String>,
    },
    PublishEpoch {
        expected: String,
    },
    PublishCommitted,
    EnqueueCommit {
        replica_id: String,
        result: String,
    },
    EnqueueAbort {
        replica_id: String,
        result: String,
    },
    DurableAbort {
        expected: String,
        #[serde(default)]
        queue_full_for: Vec<String>,
    },
    SessionAbort {
        replica_id: String,
    },
    Disconnect {
        replica_id: String,
    },
    Replacement {
        replica_id: String,
        session_epoch: u64,
    },
    Timeout {
        after: String,
    },
    Register {
        replica_id: String,
        session_epoch: u64,
    },
    Shutdown,
    ProcessExit,
}

fn assembly(byte: u8) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            char::from(b'a' + byte).to_string().repeat(64)
        )),
    }
}

fn config(byte: u8) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            char::from(b'a' + byte).to_string().repeat(32)
        ))
        .expect("config snapshot id"),
    }
}

fn session(replica_id: &str, connection_generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica_id.to_string(),
        connection_generation,
    }
}

fn epoch(
    environment: &str,
    generation: u64,
    assembly_ref: RuntimeAssemblyRef,
    config_snapshot_ref: RuntimeConfigSnapshotRef,
) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_ref.assembly_identity.clone(),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let snapshot = RuntimeConfigSnapshot::new(environment, config_snapshot_ref, Vec::new())
        .expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            environment,
            generation,
            Arc::new(assembly),
            Arc::new(snapshot),
            catalog,
        )
        .expect("epoch fixture"),
    )
}

fn state_with(
    environment: &str,
    committed_generation: u64,
    pending: Option<PendingFixture>,
) -> EnvironmentActivationState {
    EnvironmentActivationState {
        schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
        environment: environment.to_string(),
        committed: CommittedActivation {
            generation: committed_generation,
            assembly: assembly(0),
            config_snapshot: config(0),
        },
        pending: pending.map(|pending| PendingActivation {
            activation_id: pending.activation_id,
            expected_generation: pending.expected_generation,
            candidate_generation: pending.candidate_generation,
            assembly: assembly(1),
            config_snapshot: config(1),
            participant_replica_ids: pending.participant_replica_ids,
        }),
    }
}

fn request(tx: &TxFixture) -> AssemblyActivationRequest {
    AssemblyActivationRequest {
        schema_version: ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION.to_string(),
        environment: tx.environment.clone(),
        activation_id: tx.activation_id.clone(),
        expected_generation: tx.expected_generation,
        assembly: assembly(1),
        config_snapshot: config(1),
    }
}

fn ack_control(
    kind: &str,
    request: &AssemblyActivationRequest,
    replica_id: &str,
) -> AssemblyActivationControl {
    match kind {
        "prepared" => AssemblyActivationControl::Prepared {
            environment: request.environment.clone(),
            activation_id: request.activation_id.clone(),
            expected_generation: request.expected_generation,
            candidate_generation: request.expected_generation + 1,
            assembly: request.assembly.clone(),
            config_snapshot: request.config_snapshot.clone(),
            replica_id: replica_id.to_string(),
        },
        "reject" => AssemblyActivationControl::Reject {
            environment: request.environment.clone(),
            activation_id: request.activation_id.clone(),
            expected_generation: request.expected_generation,
            candidate_generation: request.expected_generation + 1,
            assembly: request.assembly.clone(),
            config_snapshot: request.config_snapshot.clone(),
            replica_id: replica_id.to_string(),
            reason: AssemblyActivationRejectReason::Admission,
        },
        other => panic!("unknown ack kind {other}"),
    }
}

fn expected_phase(terminal: &str) -> ActivationPhase {
    match terminal {
        "committed" => ActivationPhase::Committed,
        "aborted" => ActivationPhase::Aborted,
        "failed" => ActivationPhase::Failed,
        "waitingRecovery" => ActivationPhase::WaitingRecovery,
        "exited" => ActivationPhase::Exited,
        "shutdown" => ActivationPhase::Shutdown,
        other => panic!("unknown terminal {other}"),
    }
}

/// Fake blocking loader: builds a real immutable epoch for any refs and
/// scripts missing/malformed results for the candidate refs.
#[derive(Debug, Default)]
struct FakeCandidateLoader {
    candidate_failure: StdMutex<Option<CandidateLoadError>>,
    committed_failure: StdMutex<Option<CandidateLoadError>>,
}

impl FakeCandidateLoader {
    fn new() -> Self {
        Self::default()
    }

    fn set_candidate_result(&self, result: &str) {
        let failure = if result == "ok" {
            None
        } else {
            Some(load_failure_for(result))
        };
        *self.candidate_failure.lock().expect("loader lock") = failure;
    }
}

fn load_failure_for(result: &str) -> CandidateLoadError {
    match result {
        "missing" => CandidateLoadError::Missing,
        "malformed" => CandidateLoadError::Malformed("scripted malformed candidate".to_string()),
        other => panic!("unknown loadCandidate result {other}"),
    }
}

#[async_trait]
impl BlockingLoaderPort for FakeCandidateLoader {
    async fn load_candidate(
        &self,
        refs: &CandidateEpochRefs,
    ) -> Result<Arc<RoutingEpoch>, CandidateLoadError> {
        let failure = if refs.assembly == assembly(0) && refs.config_snapshot == config(0) {
            // Committed refs are identified by the scripted committed tuple
            self.committed_failure.lock().expect("loader lock").clone()
        } else {
            self.candidate_failure.lock().expect("loader lock").clone()
        };
        if let Some(failure) = failure {
            return Err(failure);
        }
        Ok(epoch(
            &refs.environment,
            refs.generation,
            refs.assembly.clone(),
            refs.config_snapshot.clone(),
        ))
    }
}

/// Fake candidate query port: scripted leases plus a first-call revalidate
/// outcome (subsequent calls stay ok, matching the corpus step placement).
#[derive(Debug, Default)]
struct FakeCandidatePort {
    leases: StdMutex<Vec<LeaseFixture>>,
    current_tuple: StdMutex<Option<RegisteredAssemblyTuple>>,
    first_revalidate: StdMutex<ActivationRevalidateOutcome>,
    revalidate_calls: AtomicUsize,
}

impl FakeCandidatePort {
    fn new() -> Self {
        Self {
            first_revalidate: StdMutex::new(ActivationRevalidateOutcome::Ok),
            ..Self::default()
        }
    }

    fn set_leases(&self, leases: Vec<LeaseFixture>) {
        *self.leases.lock().expect("leases lock") = leases;
    }

    fn set_current_tuple(&self, tuple: RegisteredAssemblyTuple) {
        *self.current_tuple.lock().expect("tuple lock") = Some(tuple);
    }

    fn set_first_revalidate(&self, outcome: ActivationRevalidateOutcome) {
        *self.first_revalidate.lock().expect("revalidate lock") = outcome;
    }

    fn leases(&self) -> Vec<LeaseFixture> {
        self.leases.lock().expect("leases lock").clone()
    }
}

impl RuntimeCandidateQueryPort for FakeCandidatePort {
    fn freeze(
        &self,
        _environment: &str,
    ) -> Result<Vec<RegisteredSessionLease>, ActivationCandidateError> {
        let tuple = self
            .current_tuple
            .lock()
            .expect("tuple lock")
            .clone()
            .expect("current tuple configured");
        Ok(self
            .leases()
            .into_iter()
            .map(|lease| RegisteredSessionLease {
                session_epoch: session(&lease.replica_id, lease.session_epoch),
                registration_revision: 1,
                exact_registered_tuple: tuple.clone(),
                cancellation: SessionCancellation { cancelled: false },
                capabilities: DispatchCapabilities {
                    unary: true,
                    server_stream: true,
                },
            })
            .collect())
    }

    fn revalidate(
        &self,
        _activation_id: &str,
        _frozen: &[ActivationParticipantBinding],
    ) -> ActivationRevalidateOutcome {
        let call = self.revalidate_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            *self.first_revalidate.lock().expect("revalidate lock")
        } else {
            ActivationRevalidateOutcome::Ok
        }
    }
}

fn assert_router_to_runtime_wire(control: &AssemblyActivationControl) {
    let bytes = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        control,
    )
    .expect("router to runtime encode");
    assert_eq!(
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RouterToRuntime, &bytes)
            .expect("router to runtime decode"),
        *control
    );
    assert!(
        encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            control
        )
        .is_err(),
        "reverse direction must fail"
    );
}

/// Fake session writer port: records enqueues/aborts in call order and
/// round-trips every control through the real transport codec.
#[derive(Debug, Default)]
struct FakeSessionPort {
    prepare_results: StdMutex<HashMap<String, EnqueueResult>>,
    commit_results: StdMutex<HashMap<String, EnqueueResult>>,
    abort_results: StdMutex<HashMap<String, EnqueueResult>>,
    enqueues: StdMutex<Vec<(String, String)>>,
    session_aborts: StdMutex<Vec<String>>,
}

impl FakeSessionPort {
    fn new() -> Self {
        Self::default()
    }

    fn set_prepare_result(&self, replica_id: &str, result: &str) {
        self.prepare_results
            .lock()
            .expect("prepare results")
            .insert(replica_id.to_string(), enqueue_result_for(result));
    }

    fn set_commit_result(&self, replica_id: &str, result: &str) {
        self.commit_results
            .lock()
            .expect("commit results")
            .insert(replica_id.to_string(), enqueue_result_for(result));
    }

    fn set_abort_result(&self, replica_id: &str, result: &str) {
        self.abort_results
            .lock()
            .expect("abort results")
            .insert(replica_id.to_string(), enqueue_result_for(result));
    }

    fn enqueues(&self) -> Vec<(String, String)> {
        self.enqueues.lock().expect("enqueues lock").clone()
    }

    fn session_aborts(&self) -> Vec<String> {
        self.session_aborts.lock().expect("aborts lock").clone()
    }

    fn record(&self, kind: &str, replica_id: &str) {
        self.enqueues
            .lock()
            .expect("enqueues lock")
            .push((kind.to_string(), replica_id.to_string()));
    }
}

fn enqueue_result_for(result: &str) -> EnqueueResult {
    match result {
        "ok" => EnqueueResult::Ok,
        "queueFull" => EnqueueResult::QueueFull,
        other => panic!("unknown enqueue result {other}"),
    }
}

impl SessionEnqueuePort for FakeSessionPort {
    fn enqueue_prepare(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        assert_router_to_runtime_wire(control);
        let result = self
            .prepare_results
            .lock()
            .expect("prepare results")
            .get(&binding.replica_id)
            .copied()
            .unwrap_or(EnqueueResult::Ok);
        if result == EnqueueResult::Ok {
            self.record("prepare", &binding.replica_id);
        }
        result
    }

    fn enqueue_commit(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        assert_router_to_runtime_wire(control);
        let result = self
            .commit_results
            .lock()
            .expect("commit results")
            .get(&binding.replica_id)
            .copied()
            .unwrap_or(EnqueueResult::Ok);
        if result == EnqueueResult::Ok {
            self.record("commit", &binding.replica_id);
        }
        result
    }

    fn enqueue_abort(
        &self,
        binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        assert_router_to_runtime_wire(control);
        let result = self
            .abort_results
            .lock()
            .expect("abort results")
            .get(&binding.replica_id)
            .copied()
            .unwrap_or(EnqueueResult::Ok);
        if result == EnqueueResult::Ok {
            self.record("abort", &binding.replica_id);
        }
        result
    }

    fn abort_session(&self, session: &RuntimeSessionEpoch) {
        self.session_aborts
            .lock()
            .expect("aborts lock")
            .push(session.replica_id.clone());
    }
}

#[derive(Debug, Clone)]
struct CommitGate {
    started_tx: watch::Sender<bool>,
    release_rx: watch::Receiver<bool>,
}

/// Real-reducer repository fake: delegates to `MemoryActivationStateRepository`
/// and adds a scripted commit gate plus a scripted commit CAS mismatch with a
/// durable-outcome read override.
#[derive(Debug)]
struct FakeRepository {
    inner: MemoryActivationStateRepository,
    read_override: RwLock<Option<EnvironmentActivationState>>,
    commit_failure: StdMutex<Option<EnvironmentActivationState>>,
    commit_gate: StdMutex<Option<CommitGate>>,
}

impl FakeRepository {
    fn new() -> Self {
        Self {
            inner: MemoryActivationStateRepository::new(),
            read_override: RwLock::new(None),
            commit_failure: StdMutex::new(None),
            commit_gate: StdMutex::new(None),
        }
    }

    async fn install_commit_gate(&self) -> (watch::Receiver<bool>, watch::Sender<bool>) {
        let (started_tx, started_rx) = watch::channel(false);
        let (release_tx, release_rx) = watch::channel(false);
        *self.commit_gate.lock().expect("gate lock") = Some(CommitGate {
            started_tx,
            release_rx,
        });
        (started_rx, release_tx)
    }

    fn set_commit_failure(
        &self,
        durable_outcome: &str,
        environment: &str,
        committed_generation: u64,
        candidate_generation: u64,
    ) {
        let state = match durable_outcome {
            "committed" => EnvironmentActivationState {
                schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
                environment: environment.to_string(),
                committed: CommittedActivation {
                    generation: candidate_generation,
                    assembly: assembly(1),
                    config_snapshot: config(1),
                },
                pending: None,
            },
            "aborted" => state_with(environment, committed_generation, None),
            other => panic!("unknown durable outcome {other}"),
        };
        *self.commit_failure.lock().expect("failure lock") = Some(state);
    }
}

#[async_trait]
impl ActivationStateRepository for FakeRepository {
    async fn read(&self, environment: &str) -> Result<EnvironmentActivationState, RepositoryError> {
        if let Some(state) = self.read_override.read().await.clone() {
            return Ok(state);
        }
        self.inner.read(environment).await
    }

    async fn initialize(
        &self,
        state: &EnvironmentActivationState,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        self.inner.initialize(state).await
    }

    async fn prepare(
        &self,
        input: PrepareInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        self.inner.prepare(input).await
    }

    async fn commit(
        &self,
        input: CommitInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        let gate = self.commit_gate.lock().expect("gate lock").clone();
        if let Some(gate) = gate {
            let _ = gate.started_tx.send(true);
            let mut release = gate.release_rx;
            while !*release.borrow() {
                if release.changed().await.is_err() {
                    break;
                }
            }
        }
        let failure = self.commit_failure.lock().expect("failure lock").clone();
        if let Some(failure) = failure {
            *self.read_override.write().await = Some(failure);
            return Err(RepositoryError::CasMismatch {
                environment: input.environment.clone(),
                message: "scripted durable commit CAS mismatch".to_string(),
            });
        }
        self.inner.commit(input).await
    }

    async fn abort(
        &self,
        input: AbortInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        self.inner.abort(input).await
    }

    async fn append_audit(
        &self,
        event: &skiff_deployment::activation_state::ActivationAuditEvent,
    ) -> Result<(), RepositoryError> {
        self.inner.append_audit(event).await
    }

    async fn ensure_indexes(&self) -> Result<(), RepositoryError> {
        self.inner.ensure_indexes().await
    }

    fn health(&self) -> skiff_router::activation::ActivationRepositoryHealth {
        self.inner.health()
    }

    async fn close(&self) -> Result<(), RepositoryError> {
        self.inner.close().await
    }
}

struct Harness {
    repo: Arc<FakeRepository>,
    epoch_store: Arc<ActiveRoutingEpochStore>,
    sessions: Arc<FakeSessionPort>,
    handle: skiff_router::activation::ActivationCoordinatorHandle,
}

/// Scripted harness inputs extracted from one corpus case (keeps
/// `spawn_harness` argument count bounded).
struct HarnessScript {
    committed_generation: u64,
    candidate_generation: u64,
    initial_pending: Option<PendingFixture>,
    leases: Vec<LeaseFixture>,
    load_result: String,
    revalidate_result: String,
    commit_outcome: Option<String>,
    gated_commit: bool,
}

async fn spawn_harness(
    environment: &str,
    script: HarnessScript,
    options: ActivationCoordinatorOptions,
) -> (
    Harness,
    Option<(watch::Receiver<bool>, watch::Sender<bool>)>,
) {
    let HarnessScript {
        committed_generation,
        candidate_generation,
        initial_pending,
        leases,
        load_result,
        revalidate_result,
        commit_outcome,
        gated_commit,
    } = script;
    let repo = Arc::new(FakeRepository::new());
    repo.initialize(&state_with(environment, committed_generation, None))
        .await
        .expect("initialize committed state");
    if let Some(pending) = initial_pending.clone() {
        let participants = pending.participant_replica_ids.clone();
        let prepared = repo
            .prepare(PrepareInput {
                environment: environment.to_string(),
                activation_id: pending.activation_id.clone(),
                expected_generation: pending.expected_generation,
                candidate_generation: pending.candidate_generation,
                assembly: assembly(1),
                config_snapshot: config(1),
                participant_replica_ids: participants,
            })
            .await
            .expect("seed pending");
        assert_eq!(
            prepared.pending.as_ref().expect("pending").activation_id,
            pending.activation_id
        );
    }

    let current_epoch = epoch(environment, committed_generation, assembly(0), config(0));
    let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
    epoch_store.publish(Arc::clone(&current_epoch));

    let loader = Arc::new(FakeCandidateLoader::new());
    loader.set_candidate_result(&load_result);

    let candidates = Arc::new(FakeCandidatePort::new());
    candidates.set_leases(leases.clone());
    candidates.set_current_tuple(current_epoch.registered_tuple());
    candidates.set_first_revalidate(if revalidate_result == "ok" {
        ActivationRevalidateOutcome::Ok
    } else {
        ActivationRevalidateOutcome::Stale
    });

    let sessions = Arc::new(FakeSessionPort::new());
    if let Some(outcome) = &commit_outcome {
        repo.set_commit_failure(
            outcome,
            environment,
            committed_generation,
            candidate_generation,
        );
    }
    let gate = if gated_commit {
        Some(repo.install_commit_gate().await)
    } else {
        None
    };

    let ports = ActivationCoordinatorPorts {
        repository: Arc::clone(&repo) as Arc<dyn ActivationStateRepository>,
        loader: Arc::clone(&loader) as Arc<dyn BlockingLoaderPort>,
        candidates: Arc::clone(&candidates) as Arc<dyn RuntimeCandidateQueryPort>,
        sessions: Arc::clone(&sessions) as Arc<dyn SessionEnqueuePort>,
        publish: Arc::new(EpochStorePublishPort::new(Arc::clone(&epoch_store)))
            as Arc<dyn PublishCommittedEpochPort>,
        health: Arc::new(NoopHealthSink),
    };
    let handle = ActivationCoordinator::spawn(ports, options);
    (
        Harness {
            repo,
            epoch_store,
            sessions,
            handle,
        },
        gate,
    )
}

async fn run_live_case(case: &Case, steps: &[Step], expected: &Expected) {
    let tx = case.tx.as_ref().expect("live tx");
    let environment = tx.environment.clone();
    let read_state = steps
        .iter()
        .find_map(|step| match step {
            Step::ReadState {
                committed_generation,
                pending,
            } => Some((*committed_generation, pending.clone())),
            _ => None,
        })
        .expect("live case readState");
    assert_eq!(
        read_state.0, tx.expected_generation,
        "{} committed",
        case.name
    );
    let leases = steps
        .iter()
        .find_map(|step| match step {
            Step::QueryCandidates { leases } => Some(leases.clone()),
            _ => None,
        })
        .expect("live case queryCandidates");
    let load_result = steps
        .iter()
        .find_map(|step| match step {
            Step::LoadCandidate { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "ok".to_string());
    let revalidate_result = steps
        .iter()
        .find_map(|step| match step {
            Step::Revalidate { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "ok".to_string());
    let commit_outcome = steps.iter().find_map(|step| match step {
        Step::DurableCommit {
            expected: _,
            durable_outcome,
        } => durable_outcome.as_deref(),
        _ => None,
    });

    let gated = steps
        .iter()
        .any(|step| matches!(step, Step::DurableCommit { .. }));
    let script = HarnessScript {
        committed_generation: tx.expected_generation,
        candidate_generation: tx.candidate_generation,
        initial_pending: read_state.1,
        leases: leases.clone(),
        load_result,
        revalidate_result,
        commit_outcome: commit_outcome.map(str::to_string),
        gated_commit: gated,
    };
    let (harness, gate) = spawn_harness(&environment, script, options()).await;
    let Harness {
        repo,
        epoch_store,
        sessions,
        handle,
    } = harness;
    for step in steps {
        match step {
            Step::EnqueuePrepare { replica_id, result } => {
                sessions.set_prepare_result(replica_id, result);
            }
            Step::EnqueueCommit { replica_id, result } => {
                sessions.set_commit_result(replica_id, result);
            }
            Step::EnqueueAbort { replica_id, result } => {
                sessions.set_abort_result(replica_id, result);
            }
            _ => {}
        }
    }

    handle
        .start_live(request(tx))
        .expect("start live transaction");
    handle
        .wait_for_phase(|phase| {
            matches!(
                phase,
                ActivationPhase::Prepared
                    | ActivationPhase::Failed
                    | ActivationPhase::Aborted
                    | ActivationPhase::Committed
            )
        })
        .await;

    for step in steps {
        match step {
            Step::Ack {
                kind,
                replica_id,
                session_epoch,
                expected: ack_expected,
            } => {
                let source = session(replica_id, *session_epoch);
                let control = ack_control(kind, &request(tx), replica_id);
                handle.deliver_ack(&source, control).expect("deliver ack");
                let _ = ack_expected;
            }
            Step::Disconnect { replica_id } => {
                let binding_epoch = leases
                    .iter()
                    .find(|lease| &lease.replica_id == replica_id)
                    .map(|lease| lease.session_epoch)
                    .expect("disconnect replica must be a frozen candidate");
                handle
                    .notify_session_closed(&session(replica_id, binding_epoch))
                    .expect("disconnect");
            }
            Step::Replacement {
                replica_id,
                session_epoch,
            } => {
                handle
                    .notify_session_replaced(replica_id, session(replica_id, *session_epoch))
                    .expect("replacement");
            }
            Step::Timeout { .. } => {
                handle.force_ack_timeout().expect("timeout");
            }
            Step::Shutdown => {
                handle.shutdown().expect("shutdown");
            }
            _ => {}
        }
    }
    if let Some((mut started, release)) = gate {
        started
            .wait_for(|started| *started)
            .await
            .expect("commit started");
        let _ = release.send(true);
    }

    let terminal = expected_phase(&expected.terminal);
    handle.wait_for_phase(|phase| phase == terminal).await;
    assert_expected(
        case.name.as_str(),
        expected,
        &handle,
        &repo,
        &epoch_store,
        &sessions,
    )
    .await;
}

async fn run_cold_case(case: &Case, steps: &[Step], expected: &Expected) {
    let tx = case.tx.as_ref().expect("cold tx");
    let environment = tx.environment.clone();
    let read_state = steps
        .iter()
        .find_map(|step| match step {
            Step::ReadState {
                committed_generation,
                pending,
            } => Some((*committed_generation, pending.clone())),
            _ => None,
        })
        .expect("cold case readState");
    let load_result = steps
        .iter()
        .find_map(|step| match step {
            Step::LoadCandidate { result } => Some(result.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "ok".to_string());
    let gated = steps
        .iter()
        .any(|step| matches!(step, Step::DurableCommit { .. }));
    let script = HarnessScript {
        committed_generation: read_state.0,
        candidate_generation: tx.candidate_generation,
        initial_pending: read_state.1,
        leases: Vec::new(),
        load_result,
        revalidate_result: "ok".to_string(),
        commit_outcome: None,
        gated_commit: gated,
    };
    let (harness, gate) = spawn_harness(&environment, script, options()).await;
    let Harness {
        repo,
        epoch_store,
        sessions,
        handle,
    } = harness;

    handle
        .start_recovery(environment.clone())
        .expect("start recovery");
    handle
        .wait_for_phase(|phase| {
            matches!(
                phase,
                ActivationPhase::WaitingRecovery
                    | ActivationPhase::Committed
                    | ActivationPhase::Aborted
                    | ActivationPhase::Prepared
            )
        })
        .await;

    for step in steps {
        match step {
            Step::Register {
                replica_id,
                session_epoch,
            } => {
                handle
                    .register_recovery_session(ActivationParticipantBinding {
                        replica_id: replica_id.clone(),
                        session_epoch: session(replica_id, *session_epoch),
                    })
                    .expect("recovery register");
            }
            Step::Ack {
                kind,
                replica_id,
                session_epoch,
                ..
            } => {
                let source = session(replica_id, *session_epoch);
                handle
                    .deliver_ack(&source, ack_control(kind, &request(tx), replica_id))
                    .expect("deliver ack");
            }
            Step::ProcessExit => {
                handle.hard_abort().expect("process exit");
            }
            _ => {}
        }
    }
    if let Some((mut started, release)) = gate {
        started
            .wait_for(|started| *started)
            .await
            .expect("commit started");
        let _ = release.send(true);
    }

    if expected.terminal == "waitingRecovery" {
        handle
            .wait_until_health(|health| health.rebound_participants >= 1)
            .await;
    }
    let terminal = expected_phase(&expected.terminal);
    handle.wait_for_phase(|phase| phase == terminal).await;
    assert_expected(
        case.name.as_str(),
        expected,
        &handle,
        &repo,
        &epoch_store,
        &sessions,
    )
    .await;
}

async fn assert_expected(
    case: &str,
    expected: &Expected,
    handle: &skiff_router::activation::ActivationCoordinatorHandle,
    repo: &FakeRepository,
    epoch_store: &ActiveRoutingEpochStore,
    sessions: &FakeSessionPort,
) {
    let health = handle.health();
    let durable = repo
        .read(
            &health
                .environment
                .clone()
                .unwrap_or_else(|| "test".to_string()),
        )
        .await
        .expect("durable read for assertions");
    assert_eq!(
        durable.committed.generation, expected.durable_state.committed_generation,
        "{case} committed generation"
    );
    match (&durable.pending, &expected.durable_state.pending) {
        (None, None) => {}
        (Some(actual), Some(expected_pending)) => {
            assert_eq!(
                actual.activation_id, expected_pending.activation_id,
                "{case} pending activation id"
            );
            assert_eq!(
                actual.expected_generation, expected_pending.expected_generation,
                "{case} pending expected generation"
            );
            assert_eq!(
                actual.candidate_generation, expected_pending.candidate_generation,
                "{case} pending candidate generation"
            );
            assert_eq!(
                actual.participant_replica_ids, expected_pending.participant_replica_ids,
                "{case} pending participants"
            );
        }
        (actual, expected_pending) => {
            panic!("{case} pending mismatch: {actual:?} != {expected_pending:?}")
        }
    }

    let captured = epoch_store.capture();
    if expected.published {
        let epoch = captured
            .as_ref()
            .unwrap_or_else(|| panic!("{case} must publish the epoch"));
        let expected_generation = expected
            .active_epoch
            .unwrap_or(expected.durable_state.committed_generation);
        assert_eq!(
            epoch.assembly_generation(),
            expected_generation,
            "{case} active epoch generation"
        );
    } else {
        let epoch = captured
            .as_ref()
            .unwrap_or_else(|| panic!("{case} must keep the current epoch published"));
        assert_eq!(
            epoch.assembly_generation(),
            expected.durable_state.committed_generation,
            "{case} must not swap the active epoch"
        );
    }
    if let Some(listener_open) = expected.listener_open {
        assert_eq!(
            listener_open, expected.published,
            "{case} listener opens exactly when the committed epoch is published"
        );
    }
    if let Some(readiness) = expected.readiness {
        assert_eq!(health.readiness, readiness, "{case} readiness");
    }
    if let Some(recovery) = expected.recovery {
        assert_eq!(health.recovery_active, recovery, "{case} recovery");
    }
    let mut aborts = sessions.session_aborts();
    aborts.sort();
    let mut expected_aborts = expected.session_aborts.clone();
    expected_aborts.sort();
    assert_eq!(aborts, expected_aborts, "{case} session aborts");
    let expected_enqueues: Vec<(String, String)> = expected
        .enqueues
        .iter()
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    assert_eq!(sessions.enqueues(), expected_enqueues, "{case} enqueues");
    assert_eq!(health.stale_acks, expected.stale_acks, "{case} stale acks");
}

fn options() -> ActivationCoordinatorOptions {
    ActivationCoordinatorOptions {
        mailbox_capacity: 128,
        ack_deadline: std::time::Duration::from_secs(3600),
        service_db_mongo_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn activation_coordinator_corpus_live_and_cold_recovery() {
        let corpus: Corpus = serde_json::from_str(include_str!(
        "../../cross-system-fixtures/package-service-ecosystem/activation-transaction-cases.json"
    ))
        .expect("activation transaction corpus must parse");
        assert_eq!(
            corpus.schema_version,
            "skiff-activation-transaction-corpus-v1"
        );
        let mut live_cases = 0usize;
        let mut cold_cases = 0usize;
        for case in &corpus.cases {
            match (&case.runs, &case.steps, &case.expected) {
                (Some(runs), None, None) => {
                    assert_eq!(case.contract, "coldRecovery", "{}", case.name);
                    for run in runs {
                        run_cold_case(
                            &Case {
                                name: case.name.clone(),
                                contract: case.contract.clone(),
                                tx: run.tx.clone(),
                                steps: None,
                                runs: None,
                                expected: None,
                            },
                            &run.steps,
                            &run.expected,
                        )
                        .await;
                    }
                    cold_cases += 1;
                }
                (None, Some(steps), Some(expected)) => {
                    match case.contract.as_str() {
                        "live" => run_live_case(case, steps, expected).await,
                        "coldRecovery" => run_cold_case(case, steps, expected).await,
                        other => panic!("unknown contract {other}"),
                    }
                    if case.contract == "live" {
                        live_cases += 1;
                    } else {
                        cold_cases += 1;
                    }
                }
                _ => panic!("{} must have steps+expected or runs", case.name),
            }
        }
        assert_eq!(live_cases, 16, "live corpus must stay exhaustive");
        assert_eq!(cold_cases, 6, "cold recovery corpus must stay exhaustive");
    }
}
