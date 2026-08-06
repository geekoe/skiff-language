//! D2 focused router control-plane tests: submit success/rejection/idempotent
//! retry, status/cancel projection, the real admission seam's four decision
//! classes, settlement mapping (success/failure/timeout/uncertain), the
//! immediate-task wake fast path, and actor-method target rejection.

mod dispatch_harness;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, AssemblyIdentity,
    PackageCallableId, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_router::dispatch::{RequestDispatcher, RuntimeDispatcherOptions};
use skiff_router::session::demux::InboundFrameSink;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::supervisor::actor::{assemble_actor_components, ActorComponents};
use skiff_router::supervisor::actor_sink::ActorFrameSink;
use skiff_router::supervisor::session_ports::SessionHandle;
use skiff_router::supervisor::ws::WsSessionWriter;
use skiff_router::task::{
    DurableTaskControl, DurableTaskFrameSink, FirstAdmissionOutcome, NoopActorAttemptTerminalSink,
    NoopTaskSubmitParentResolver, RouterTaskAttemptAdmission, TaskActorOwnerPort,
    TaskControlCounters, TaskExecutionImageSource, TaskSubmitParentResolver,
};
use skiff_router::telemetry::{NoopTaskTelemetrySink, TaskTelemetrySink};
use skiff_router::ws::Clock;
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader, ActorOwnerFileFrameHeader,
    ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_task_cancel_error_frame, decode_task_cancel_response_frame,
    decode_task_status_error_frame, decode_task_status_response_frame,
    decode_task_submit_error_frame, decode_task_submit_response_frame,
    encode_task_cancel_request_frame, encode_task_status_request_frame,
    encode_task_submit_request_frame, ActivationIdentityFrameMetadata,
    TaskActorActivationSnapshotFrameMetadata, TaskActorMethodTargetFrameMetadata, TaskCallerKind,
    TaskCancelRequestFrameHeader, TaskCancelResultKindWire, TaskControlRejectionCode,
    TaskStatusKindWire, TaskSubmitRequestFrameHeaderV2, TaskSubmitTiming, TaskTargetKind,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_task_control::model::{
    DetachedCallTarget, DurableDuration, DurableUtcTimestamp, RecoverablePayload, ServiceOwner,
    TaskExecutionImageRef, TaskId, TaskOutcome, TaskRecord, TaskState, TaskStatusKind,
    TaskTerminal, TaskTestCaseAuthority, TaskTraceContext,
};
use skiff_task_control::scheduler::{
    AdmissionDecision, AttemptAdmission, RetryBackoffPolicy, Scheduler, SchedulerConfig,
};
use skiff_task_control::store::{ClaimInput, DueScanInput, SettleInput, StatusInput, TaskStore};
use skiff_task_control::MemoryTaskStore;
use skiff_task_control::TaskStoreError;
use tokio::sync::watch;

const SERVICE_ID: &str = "example.com/service-1";
const TASK_ID: &str = "task-control-1";

fn noop_telemetry() -> Arc<dyn TaskTelemetrySink> {
    Arc::new(NoopTaskTelemetrySink)
}

// ---------------------------------------------------------------------------
// Fakes
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct TestClock {
    now_ms: AtomicU64,
}

impl TestClock {
    fn advance(&self, millis: u64) {
        self.now_ms.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Default)]
struct FakeWriter {
    frames: Mutex<Vec<Vec<u8>>>,
}

impl WsSessionWriter for FakeWriter {
    fn write(&self, _runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.frames.lock().expect("writer frames").push(bytes);
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeTaskActorOwnerPort {
    candidates: Arc<Mutex<Vec<RuntimeSessionEpoch>>>,
    sessions: Arc<Mutex<HashMap<String, RuntimeSessionEpoch>>>,
    frames: Arc<Mutex<Vec<(RuntimeSessionEpoch, Vec<u8>)>>>,
}

impl TaskActorOwnerPort for FakeTaskActorOwnerPort {
    fn candidates_by_build_id(&self, _build_id: &str) -> Vec<RuntimeSessionEpoch> {
        self.candidates.lock().expect("candidates lock").clone()
    }

    fn current_session_by_replica(&self, replica_id: &str) -> Option<RuntimeSessionEpoch> {
        self.sessions
            .lock()
            .expect("sessions lock")
            .get(replica_id)
            .cloned()
    }

    fn write(&self, session: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.frames
            .lock()
            .expect("frames lock")
            .push((session.clone(), bytes));
        Ok(())
    }
}

/// Minimal real actor lane for the task admission seam (function-target tests
/// never exercise it; actor-method tests build their own richer rig).
fn actor_lane_stub() -> (
    Arc<ActorComponents>,
    Arc<FakeTaskActorOwnerPort>,
    Arc<Mutex<Option<Arc<ActorFrameSink>>>>,
) {
    let root = temp_artifact_root();
    let session = SessionHandle::new();
    let components = assemble_actor_components(
        &root,
        skiff_router::artifact::ActorRoutingProjectionRef::new(
            skiff_artifact_identity::ArtifactRelativePath::new(
                skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("actor projection path"),
        ),
        session.clone(),
    )
    .expect("actor components");
    let writer = Arc::new(FakeWriter::default());
    let sink = Arc::new(ActorFrameSink::new(
        Arc::clone(&components),
        session,
        skiff_deployment::storage::CanonicalArtifactStore::open(&root)
            .expect("artifact store open"),
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::new(TestClock::default()),
        Arc::new(NoopActorAttemptTerminalSink),
    ));
    let port = Arc::new(FakeTaskActorOwnerPort::default());
    let deferred = Arc::new(Mutex::new(Some(sink)));
    (components, port, deferred)
}

/// Temporary empty artifact root for the actor lane stub.
fn temp_artifact_root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "skiff-task-control-actor-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("create temp root");
    path
}

#[derive(Debug, Clone)]
struct FakeImageSource {
    image: TaskExecutionImageRef,
    services: Vec<String>,
}

impl FakeImageSource {
    fn new(image: TaskExecutionImageRef) -> Self {
        Self {
            image,
            services: vec![SERVICE_ID.to_string()],
        }
    }
}

impl TaskExecutionImageSource for FakeImageSource {
    fn resolve(&self, _header: &TaskSubmitRequestFrameHeaderV2) -> Option<TaskExecutionImageRef> {
        Some(self.image.clone())
    }

    fn contains_service(&self, service_id: &str) -> bool {
        self.services.iter().any(|known| known == service_id)
    }

    fn contains_deployment(
        &self,
        deployment: &skiff_artifact_model::ServiceDeploymentRef,
    ) -> bool {
        &self.image.deployment == deployment
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NoopAdmission;

#[async_trait]
impl AttemptAdmission for NoopAdmission {
    async fn admit(&self, _record: &TaskRecord) -> AdmissionDecision {
        AdmissionDecision::Accepted
    }
}

/// Scripted admission for the wake fast-path test (same shape as the
/// task-control scheduler test harness).
#[derive(Debug, Default)]
struct ScriptedAdmission {
    calls: AtomicUsize,
    notified: Mutex<Option<watch::Sender<u64>>>,
}

impl ScriptedAdmission {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            notified: Mutex::new(None),
        }
    }

    fn install_notifier(&self) -> watch::Receiver<u64> {
        let (sender, receiver) = watch::channel(0);
        *self.notified.lock().expect("notifier lock") = Some(sender);
        receiver
    }

    async fn wait_for_calls(&self, count: usize, mut receiver: watch::Receiver<u64>) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.calls.load(Ordering::SeqCst) < count {
                if receiver.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .expect("admission call did not arrive");
    }
}

#[async_trait]
impl AttemptAdmission for ScriptedAdmission {
    async fn admit(&self, _record: &TaskRecord) -> AdmissionDecision {
        let calls = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(sender) = &*self.notified.lock().expect("notifier lock") {
            let _ = sender.send(calls as u64);
        }
        AdmissionDecision::Accepted
    }
}

/// TaskStore wrapper whose next `create` reports a transient error after the
/// durable commit actually landed (ambiguous acceptance probe).
#[derive(Clone)]
struct AmbiguousCreateStore {
    inner: MemoryTaskStore,
    fail_next_create: Arc<AtomicBool>,
}

impl std::fmt::Debug for AmbiguousCreateStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AmbiguousCreateStore")
            .field("fail_next_create", &self.fail_next_create)
            .finish_non_exhaustive()
    }
}

impl AmbiguousCreateStore {
    fn new() -> Self {
        Self {
            inner: MemoryTaskStore::new(),
            fail_next_create: Arc::new(AtomicBool::new(false)),
        }
    }

    fn fail_next_create(&self) {
        self.fail_next_create.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl TaskStore for AmbiguousCreateStore {
    async fn now(&self) -> Result<DurableUtcTimestamp, TaskStoreError> {
        self.inner.now().await
    }

    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        if self.fail_next_create.swap(false, Ordering::SeqCst) {
            // The durable commit landed but the response was lost.
            let _committed = self.inner.create(record).await?;
            return Err(TaskStoreError::Transient {
                message: "ambiguous create response".to_string(),
            });
        }
        self.inner.create(record).await
    }

    async fn claim(
        &self,
        input: ClaimInput,
    ) -> Result<skiff_task_control::store::ClaimOutcome, TaskStoreError> {
        self.inner.claim(input).await
    }

    async fn renew(
        &self,
        input: skiff_task_control::store::RenewInput,
    ) -> Result<skiff_task_control::store::RenewOutcome, TaskStoreError> {
        self.inner.renew(input).await
    }

    async fn settle(
        &self,
        input: SettleInput,
    ) -> Result<skiff_task_control::store::SettleOutcome, TaskStoreError> {
        self.inner.settle(input).await
    }

    async fn cancel(
        &self,
        input: skiff_task_control::store::CancelInput,
    ) -> Result<skiff_task_control::model::TaskCancelResult, TaskStoreError> {
        self.inner.cancel(input).await
    }

    async fn recover_expired_lease(
        &self,
        input: skiff_task_control::store::LeaseRecoveryInput,
    ) -> Result<skiff_task_control::store::LeaseRecoveryOutcome, TaskStoreError> {
        self.inner.recover_expired_lease(input).await
    }

    async fn release(
        &self,
        input: skiff_task_control::store::ReleaseInput,
    ) -> Result<skiff_task_control::store::ReleaseOutcome, TaskStoreError> {
        self.inner.release(input).await
    }

    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_due(input).await
    }

    async fn scan_expired_leases(
        &self,
        input: skiff_task_control::store::ScanExpiredLeasesInput,
    ) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_expired_leases(input).await
    }

    async fn status(
        &self,
        input: StatusInput,
    ) -> Result<skiff_task_control::model::TaskStatus, TaskStoreError> {
        self.inner.status(input).await
    }

    async fn observe_backlog(
        &self,
    ) -> Result<skiff_task_control::store::BacklogObservation, TaskStoreError> {
        self.inner.observe_backlog().await
    }

    async fn ensure_indexes(&self) -> Result<(), TaskStoreError> {
        self.inner.ensure_indexes().await
    }

    async fn close(&self) -> Result<(), TaskStoreError> {
        self.inner.close().await
    }
}

/// Scripted status/cancel outcome for the E1 error-frame branches: each
/// control query can be forced to a transient store failure or a definite
/// store `NotFound`; everything else delegates to the in-memory store.
#[derive(Clone)]
struct ScriptedControlStore {
    inner: MemoryTaskStore,
    status_outcome: Arc<Mutex<ControlQueryOutcome>>,
    cancel_outcome: Arc<Mutex<ControlQueryOutcome>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ControlQueryOutcome {
    #[default]
    Passthrough,
    Transient,
    NotFound,
}

impl ScriptedControlStore {
    fn new() -> Self {
        Self {
            inner: MemoryTaskStore::new(),
            status_outcome: Arc::new(Mutex::new(ControlQueryOutcome::Passthrough)),
            cancel_outcome: Arc::new(Mutex::new(ControlQueryOutcome::Passthrough)),
        }
    }

    fn script_status(&self, outcome: ControlQueryOutcome) {
        *self.status_outcome.lock().expect("status script") = outcome;
    }

    fn script_cancel(&self, outcome: ControlQueryOutcome) {
        *self.cancel_outcome.lock().expect("cancel script") = outcome;
    }
}

#[async_trait]
impl TaskStore for ScriptedControlStore {
    async fn now(&self) -> Result<DurableUtcTimestamp, TaskStoreError> {
        self.inner.now().await
    }

    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        self.inner.create(record).await
    }

    async fn claim(
        &self,
        input: ClaimInput,
    ) -> Result<skiff_task_control::store::ClaimOutcome, TaskStoreError> {
        self.inner.claim(input).await
    }

    async fn renew(
        &self,
        input: skiff_task_control::store::RenewInput,
    ) -> Result<skiff_task_control::store::RenewOutcome, TaskStoreError> {
        self.inner.renew(input).await
    }

    async fn settle(
        &self,
        input: SettleInput,
    ) -> Result<skiff_task_control::store::SettleOutcome, TaskStoreError> {
        self.inner.settle(input).await
    }

    async fn cancel(
        &self,
        input: skiff_task_control::store::CancelInput,
    ) -> Result<skiff_task_control::model::TaskCancelResult, TaskStoreError> {
        let outcome = *self.cancel_outcome.lock().expect("cancel script");
        match outcome {
            ControlQueryOutcome::Passthrough => self.inner.cancel(input).await,
            ControlQueryOutcome::Transient => Err(TaskStoreError::Transient {
                message: "scripted cancel transient failure".to_string(),
            }),
            ControlQueryOutcome::NotFound => Err(TaskStoreError::NotFound {
                task_id: input.task_id,
            }),
        }
    }

    async fn recover_expired_lease(
        &self,
        input: skiff_task_control::store::LeaseRecoveryInput,
    ) -> Result<skiff_task_control::store::LeaseRecoveryOutcome, TaskStoreError> {
        self.inner.recover_expired_lease(input).await
    }

    async fn release(
        &self,
        input: skiff_task_control::store::ReleaseInput,
    ) -> Result<skiff_task_control::store::ReleaseOutcome, TaskStoreError> {
        self.inner.release(input).await
    }

    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_due(input).await
    }

    async fn scan_expired_leases(
        &self,
        input: skiff_task_control::store::ScanExpiredLeasesInput,
    ) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_expired_leases(input).await
    }

    async fn status(
        &self,
        input: StatusInput,
    ) -> Result<skiff_task_control::model::TaskStatus, TaskStoreError> {
        let outcome = *self.status_outcome.lock().expect("status script");
        match outcome {
            ControlQueryOutcome::Passthrough => self.inner.status(input).await,
            ControlQueryOutcome::Transient => Err(TaskStoreError::Transient {
                message: "scripted status transient failure".to_string(),
            }),
            ControlQueryOutcome::NotFound => Err(TaskStoreError::NotFound {
                task_id: input.task_id,
            }),
        }
    }

    async fn observe_backlog(
        &self,
    ) -> Result<skiff_task_control::store::BacklogObservation, TaskStoreError> {
        self.inner.observe_backlog().await
    }

    async fn ensure_indexes(&self) -> Result<(), TaskStoreError> {
        self.inner.ensure_indexes().await
    }

    async fn close(&self) -> Result<(), TaskStoreError> {
        self.inner.close().await
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn corpus_image() -> TaskExecutionImageRef {
    TaskExecutionImageRef {
        target_profile: dispatch_harness::CORPUS_PROFILE.to_string(),
        package_version: dispatch_harness::CORPUS_CONTRACT_VERSION.to_string(),
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(
                dispatch_harness::CORPUS_ASSEMBLY_IDENTITY.to_string(),
            ),
        },
        config_snapshot: RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                dispatch_harness::CORPUS_CONFIG_SNAPSHOT_ID.to_string(),
            )
            .expect("corpus config snapshot"),
        },
        deployment: dispatch_harness::corpus_deployment_ref(),
    }
}

fn record(
    task_id: &str,
    execution: TaskExecutionImageRef,
    due_at: DurableUtcTimestamp,
    state: TaskState,
) -> TaskRecord {
    TaskRecord {
        task_id: TaskId::new(task_id),
        owner: ServiceOwner::new(SERVICE_ID),
        execution,
        target: DetachedCallTarget::Function {
            callable: PackageCallableId::new("example.com/service-1:fn"),
        },
        payload: RecoverablePayload::new(vec![1, 2, 3]),
        due_at,
        state,
        attempt_generation: 0,
        active_lease: None,
        terminal: None,
        trace: TaskTraceContext {
            trace_id: "trace-1".to_string(),
            span_id: None,
        },
        created_at: due_at,
        retry_not_before: None,
        test_case: None,
    }
}

fn submit_header(
    task_id: Option<&str>,
    target_kind: TaskTargetKind,
    timing: Option<TaskSubmitTiming>,
) -> TaskSubmitRequestFrameHeaderV2 {
    TaskSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.submit.request".to_string(),
        rpc_id: "rpc:submit".to_string(),
        runtime_id: "runtime-a".to_string(),
        caller_kind: TaskCallerKind::Request,
        caller_request_id: "parent-request".to_string(),
        target_kind,
        service_id: SERVICE_ID.to_string(),
        service_version: dispatch_harness::CORPUS_CONTRACT_VERSION.to_string(),
        service_protocol_identity: "example.com/service-1:1.0.0".to_string(),
        target: "example.com/service-1:fn".to_string(),
        timing,
        task_id: task_id.map(str::to_string),
        build_id: None,
        activation_identity: ActivationIdentityFrameMetadata {
            assembly_identity: dispatch_harness::CORPUS_ASSEMBLY_IDENTITY.to_string(),
            generation: dispatch_harness::CORPUS_GENERATION,
            runtime_replica_id: "runtime-a".to_string(),
            deployment_revision: dispatch_harness::CORPUS_DEPLOYMENT_REVISION.to_string(),
        },
        trace_id: Some("trace-1".to_string()),
        caller_target: None,
        max_queue_wait_ms: None,
        actor_method: None,
    }
}

fn actor_method_submit_header(task_id: Option<&str>) -> TaskSubmitRequestFrameHeaderV2 {
    let mut header = submit_header(task_id, TaskTargetKind::ActorMethod, None);
    let key_payload = serde_json::json!({
        "serviceId": SERVICE_ID,
        "actorTypeIdentity": "skiff-actor-type-v1:sha256:aaa",
        "actorIdTypeIdentity": "skiff-actor-id-type-v1:sha256:aaa",
        "actorIdEncodingVersion": "v1",
        "canonicalActorIdKeyBytesBase64": "a2V5",
        "actorIdHash": "sha256:aaa",
    });
    header.actor_method = Some(TaskActorMethodTargetFrameMetadata {
        actor_ref: ActorLogicalRefFrameHeader {
            service_id: SERVICE_ID.to_string(),
            actor_type_identity: "skiff-actor-type-v1:sha256:aaa".to_string(),
            actor_id_type_identity: "skiff-actor-id-type-v1:sha256:aaa".to_string(),
            actor_id_encoding_version: "v1".to_string(),
            canonical_actor_id_key_bytes_base64: "a2V5".to_string(),
            actor_id_hash: "sha256:aaa".to_string(),
            epoch: 1,
        },
        declaration_owner: ActorDeclarationOwnerFrameHeader {
            unit: ActorOwnerUnitFrameHeader::Service,
            file: ActorOwnerFileFrameHeader::LoadedFileIndex(0),
            actor_symbol: "Actor".to_string(),
        },
        actor_abi_identity: ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:aaa"),
        actor_implementation_identity: ActorImplementationIdentity::new(
            "skiff-actor-implementation-v1:sha256:aaa",
        ),
        method_identity: ActorMethodIdentity::new("skiff-actor-method-v1:sha256:aaa"),
        activation: TaskActorActivationSnapshotFrameMetadata {
            key: base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&key_payload).expect("key payload json")),
            create_input: base64::engine::general_purpose::STANDARD.encode(b"[]"),
            expected_type_plan: serde_json::json!({
                "label": "record",
                "node": { "kind": "record", "fields": [] }
            }),
        },
    });
    header
}

fn status_request(
    task_ref: &str,
) -> skiff_runtime_transport::protocol::TaskStatusRequestFrameHeader {
    skiff_runtime_transport::protocol::TaskStatusRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.status.request".to_string(),
        rpc_id: "rpc:status".to_string(),
        task_ref: skiff_runtime_transport::protocol::TaskRef::parse(task_ref).expect("task ref"),
    }
}

fn cancel_request(task_ref: &str) -> TaskCancelRequestFrameHeader {
    TaskCancelRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.cancel.request".to_string(),
        rpc_id: "rpc:cancel".to_string(),
        task_ref: skiff_runtime_transport::protocol::TaskRef::parse(task_ref).expect("task ref"),
    }
}

fn task_ref(task_id: &str) -> String {
    skiff_runtime_transport::protocol::TaskRef::new(task_id, SERVICE_ID)
        .expect("task ref")
        .into_string()
}

fn sink_rig() -> (
    Arc<MemoryTaskStore>,
    Arc<Scheduler>,
    Arc<DurableTaskFrameSink>,
    Arc<FakeWriter>,
    Arc<TaskControlCounters>,
) {
    let store = Arc::new(MemoryTaskStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        store_dyn.clone(),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let writer = Arc::new(FakeWriter::default());
    let counters = Arc::new(TaskControlCounters::default());
    let sink = Arc::new(DurableTaskFrameSink::new(
        store_dyn,
        Arc::clone(&scheduler),
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::clone(&counters),
        noop_telemetry(),
        4096,
    ));
    (store, scheduler, sink, writer, counters)
}

fn scripted_control_rig(
    store: Arc<dyn TaskStore>,
    _scripted: Arc<ScriptedControlStore>,
) -> (
    Arc<Scheduler>,
    Arc<DurableTaskFrameSink>,
    Arc<FakeWriter>,
    Arc<TaskControlCounters>,
) {
    let scheduler = Arc::new(Scheduler::new(
        Arc::clone(&store),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let writer = Arc::new(FakeWriter::default());
    let counters = Arc::new(TaskControlCounters::default());
    let sink = Arc::new(DurableTaskFrameSink::new(
        store,
        Arc::clone(&scheduler),
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::clone(&counters),
        noop_telemetry(),
        4096,
    ));
    (scheduler, sink, writer, counters)
}

async fn claim_ready(store: &dyn TaskStore, task_id: &str) -> TaskRecord {
    let records = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan");
    let record = records
        .into_iter()
        .find(|record| record.task_id.as_str() == task_id)
        .expect("task must be due and ready");
    let expiry = store
        .now()
        .await
        .expect("now")
        .checked_add_millis(60_000)
        .expect("expiry");
    match store
        .claim(ClaimInput {
            task_id: record.task_id.clone(),
            owner: "test-scheduler".to_string(),
            lease_expiry: expiry,
            image_activatable: true,
        })
        .await
        .expect("claim")
    {
        skiff_task_control::store::ClaimOutcome::Claimed(record) => record,
        other => panic!("claim failed: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Submit handler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_success_creates_record_and_returns_task_ref() {
    let (store, _scheduler, sink, writer, counters) = sink_rig();
    let header = submit_header(Some(TASK_ID), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    sink.handle(
        &RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        },
        &bytes,
    )
    .expect("handle");

    let response = poll_writer(&writer, 1).await;
    let decoded = decode_task_submit_response_frame(&response).expect("response");
    assert_eq!(decoded.task_id, TASK_ID);
    assert_eq!(decoded.task_ref.task_id(), TASK_ID);
    assert_eq!(decoded.task_ref.owner(), SERVICE_ID);
    assert_eq!(decoded.request_id, TASK_ID);

    let records = store.records().await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].task_id.as_str(), TASK_ID);
    assert_eq!(records[0].state, TaskState::Scheduled);
    assert_eq!(
        records[0].test_case, None,
        "ordinary production submissions must not carry test-case authority"
    );
    assert_eq!(counters.submissions_accepted.load(Ordering::Relaxed), 1);
}

/// Scripted parent resolver: returns a configured capability only for one
/// exact caller request id on one exact session.
#[derive(Debug)]
struct ScriptedTaskSubmitParentResolver {
    capability: Option<String>,
    caller_request_id: String,
    session: RuntimeSessionEpoch,
}

impl TaskSubmitParentResolver for ScriptedTaskSubmitParentResolver {
    fn resolve(
        &self,
        session: &RuntimeSessionEpoch,
        _caller_kind: TaskCallerKind,
        caller_request_id: &str,
    ) -> Option<String> {
        if session == &self.session && caller_request_id == self.caller_request_id {
            self.capability.clone()
        } else {
            None
        }
    }
}

#[tokio::test]
async fn submit_from_test_request_parent_captures_test_case_authority() {
    let store = Arc::new(MemoryTaskStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        Arc::clone(&store_dyn),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let writer = Arc::new(FakeWriter::default());
    let counters = Arc::new(TaskControlCounters::default());
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    let sink = Arc::new(DurableTaskFrameSink::new(
        store_dyn,
        Arc::clone(&scheduler),
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::new(ScriptedTaskSubmitParentResolver {
            capability: Some("test-case:cap-1".to_string()),
            caller_request_id: "parent-request".to_string(),
            session: session.clone(),
        }) as Arc<dyn TaskSubmitParentResolver>,
        None,
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::clone(&counters),
        noop_telemetry(),
        4096,
    ));
    let header = submit_header(Some(TASK_ID), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    sink.handle(&session, &bytes).expect("handle");
    let _ = poll_writer(&writer, 1).await;

    let records = store.records().await;
    assert_eq!(records.len(), 1);
    let authority = records[0]
        .test_case
        .as_ref()
        .expect("test parent task must persist its test-case authority");
    assert_eq!(authority.test_case_capability, "test-case:cap-1");
    assert_eq!(authority.parent_request_id, "parent-request");
    assert_eq!(authority.origin_runtime_id, "runtime-a");
    assert_eq!(authority.origin_connection_generation, 1);
}

#[tokio::test]
async fn submit_actor_method_target_creates_durable_actor_record() {
    let (_store, _scheduler, sink, writer, counters) = sink_rig();
    let header = actor_method_submit_header(Some(TASK_ID));
    sink.handle_submit(
        RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        },
        header,
        vec![1, 2, 3],
    )
    .await
    .expect("handler");
    let response = poll_writer(&writer, 1).await;
    let decoded = decode_task_submit_response_frame(&response).expect("response");
    assert_eq!(decoded.task_id, TASK_ID);
    assert_eq!(counters.submissions_accepted.load(Ordering::Relaxed), 1);
    let records = _store.records().await;
    assert_eq!(records.len(), 1);
    match &records[0].target {
        DetachedCallTarget::ActorMethod {
            activation,
            implementation,
            method,
            declaration_owner,
            ..
        } => {
            assert_eq!(
                implementation.as_str(),
                "skiff-actor-implementation-v1:sha256:aaa"
            );
            assert_eq!(method.as_str(), "skiff-actor-method-v1:sha256:aaa");
            assert_eq!(declaration_owner.actor_symbol, "Actor");
            assert!(!activation.key.as_bytes().is_empty());
            assert_eq!(activation.create_input.as_bytes(), b"[]");
            assert!(activation.expected_type_plan_runtime.is_some());
            assert!(matches!(
                activation.expected_type_plan.root,
                skiff_artifact_model::RecoverableExpectedTypeRoot::TypeRef { .. }
            ));
        }
        other => panic!("expected actor-method target, got {other:?}"),
    }
}

#[tokio::test]
async fn submit_invalid_timing_and_quota_are_definite_rejections() {
    let (_store, _scheduler, sink, writer, _counters) = sink_rig();
    let header = submit_header(
        Some(TASK_ID),
        TaskTargetKind::Function,
        Some(TaskSubmitTiming::At { utc_millis: -1 }),
    );
    let bytes = encode_task_submit_request_frame(&header, &[]).expect("encode");
    sink.handle(
        &RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        },
        &bytes,
    )
    .expect("handle");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_submit_error_frame(&error).expect("error");
    assert_eq!(decoded.error.code, "invalidTiming");

    let oversized = sink_rig();
    let writer = oversized.3;
    let header = submit_header(Some("task-big"), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &vec![7; 8192]).expect("encode");
    oversized
        .2
        .handle(
            &RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            &bytes,
        )
        .expect("handle");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_submit_error_frame(&error).expect("error");
    assert_eq!(decoded.error.code, "quotaExceeded");
}

#[tokio::test]
async fn submit_same_task_id_is_idempotent() {
    let (store, _scheduler, sink, writer, _counters) = sink_rig();
    let header = submit_header(Some(TASK_ID), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    sink.handle(&session, &bytes).expect("first handle");
    sink.handle(&session, &bytes).expect("second handle");
    let first = poll_writer(&writer, 1).await;
    let second = poll_writer(&writer, 2).await;
    assert_eq!(
        decode_task_submit_response_frame(&first)
            .expect("response")
            .task_id,
        TASK_ID
    );
    assert_eq!(
        decode_task_submit_response_frame(&second)
            .expect("response")
            .task_id,
        TASK_ID
    );
    assert_eq!(store.records().await.len(), 1, "no second task");
}

#[tokio::test]
async fn submit_transient_create_queries_same_task_id() {
    // Pure transient: no record visible -> storeUnavailable.
    let (store, _scheduler, sink, writer, counters) = sink_rig();
    store.fail_next_transient(1).await;
    let header = submit_header(Some(TASK_ID), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    sink.handle(
        &RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        },
        &bytes,
    )
    .expect("handle");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_submit_error_frame(&error).expect("error");
    assert_eq!(decoded.error.code, "storeUnavailable");
    assert_eq!(counters.submissions_transient.load(Ordering::Relaxed), 1);

    // Ambiguous acceptance: create lost its response after commit; the same
    // TaskId status query sees the durable record and returns success.
    let ambiguous = Arc::new(AmbiguousCreateStore::new());
    let ambiguous_dyn = Arc::clone(&ambiguous) as Arc<dyn TaskStore>;
    let ambiguous_writer = Arc::new(FakeWriter::default());
    let ambiguous_sink = Arc::new(DurableTaskFrameSink::new(
        ambiguous_dyn.clone(),
        Arc::new(Scheduler::new(
            ambiguous_dyn.clone(),
            Arc::new(NoopAdmission),
            Arc::new(skiff_task_control::SystemClock),
            SchedulerConfig::default(),
            RetryBackoffPolicy::default(),
        )),
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        Arc::clone(&ambiguous_writer) as Arc<dyn WsSessionWriter>,
        Arc::new(TaskControlCounters::default()),
        noop_telemetry(),
        4096,
    ));
    ambiguous.fail_next_create();
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    ambiguous_sink
        .handle(
            &RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            &bytes,
        )
        .expect("handle");
    let response = poll_writer(&ambiguous_writer, 1).await;
    let decoded = decode_task_submit_response_frame(&response).expect("response");
    assert_eq!(decoded.task_id, TASK_ID);
}

// ---------------------------------------------------------------------------
// Immediate wake fast path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn immediate_submit_wakes_scheduler_without_waiting_for_scan() {
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let admission = Arc::new(ScriptedAdmission::new());
    let notifier = admission.install_notifier();
    let scheduler = Arc::new(Scheduler::new(
        Arc::clone(&store),
        admission.clone(),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig {
            scan_interval: Duration::from_secs(3600),
            lease_duration: DurableDuration::from_millis(7_201_000),
            ..SchedulerConfig::default()
        },
        RetryBackoffPolicy::default(),
    ));
    let run = tokio::spawn({
        let scheduler = Arc::clone(&scheduler);
        async move {
            scheduler.run().await;
        }
    });
    let writer = Arc::new(FakeWriter::default());
    let sink = Arc::new(DurableTaskFrameSink::new(
        Arc::clone(&store),
        Arc::clone(&scheduler),
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::new(TaskControlCounters::default()),
        noop_telemetry(),
        4096,
    ));
    let header = submit_header(Some(TASK_ID), TaskTargetKind::Function, None);
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode");
    sink.handle(
        &RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        },
        &bytes,
    )
    .expect("handle");
    admission.wait_for_calls(1, notifier).await;
    assert_eq!(admission.calls.load(Ordering::SeqCst), 1);
    run.abort();
}

// ---------------------------------------------------------------------------
// Admission seam
// ---------------------------------------------------------------------------

struct ControlRig {
    store: Arc<dyn TaskStore>,
    scheduler: Arc<Scheduler>,
    control: Arc<DurableTaskControl>,
    dispatcher: Arc<RequestDispatcher>,
    admission: Arc<RouterTaskAttemptAdmission>,
    peer: dispatch_harness::FakeRuntimePeer,
    session: RuntimeSessionEpoch,
    clock: Arc<TestClock>,
    worker: tokio::task::JoinHandle<()>,
}

fn control_rig() -> ControlRig {
    control_rig_with_sessions(vec![dispatch_harness::session_state("s1", "runtime-a", 1)])
}

fn control_rig_with_sessions(sessions: Vec<dispatch_harness::SessionState>) -> ControlRig {
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let clock = Arc::new(TestClock::default());
    clock.now_ms.store(1_700_000_000_000, Ordering::SeqCst);
    let deferred_scheduler: Arc<Mutex<Option<Arc<Scheduler>>>> = Arc::new(Mutex::new(None));
    let deferred_dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>> =
        Arc::new(Mutex::new(None));
    let counters = Arc::new(TaskControlCounters::default());
    let control = Arc::new(DurableTaskControl::new(
        Arc::clone(&store),
        Arc::clone(&deferred_scheduler),
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&counters),
        noop_telemetry(),
        Duration::from_millis(20),
    ));
    let worker = control.spawn_worker();
    let peer = dispatch_harness::FakeRuntimePeer::new();
    let abort = dispatch_harness::FakeSessionAbort::new();
    let candidate = dispatch_harness::FakeCandidateViewSource::new(sessions);
    let (actor, actor_port, deferred_actor_sink) = actor_lane_stub();
    let admission = Arc::new(RouterTaskAttemptAdmission::new(
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&control),
        Arc::clone(&clock) as Arc<dyn Clock>,
        5_000,
        Arc::clone(&counters),
        noop_telemetry(),
        actor,
        actor_port as Arc<dyn TaskActorOwnerPort>,
        30_000,
        deferred_actor_sink,
    ));
    let scheduler = Arc::new(Scheduler::new(
        Arc::clone(&store),
        Arc::clone(&admission) as Arc<dyn AttemptAdmission>,
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    *deferred_scheduler.lock().expect("scheduler lock") = Some(Arc::clone(&scheduler));
    let dispatcher = Arc::new(
        RequestDispatcher::new(
            RuntimeDispatcherOptions::new(
                8,
                Arc::new(candidate),
                Arc::new(dispatch_harness::FakeLeaseRevalidate::new()),
                Arc::new(peer.clone()),
                Arc::new(abort),
            )
            .expect("options")
            .with_task_attempt_terminal(
                Arc::clone(&control) as Arc<dyn skiff_router::dispatch::TaskAttemptTerminalSink>
            ),
        )
        .expect("dispatcher"),
    );
    *deferred_dispatcher.lock().expect("dispatcher lock") = Some(Arc::clone(&dispatcher));
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    ControlRig {
        store,
        scheduler,
        control,
        dispatcher,
        admission,
        peer,
        session,
        clock,
        worker,
    }
}

#[tokio::test]
async fn admission_accepted_writes_task_attempt_request() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    rig.store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    let decision = rig.admission.admit(&claimed).await;
    assert_eq!(decision, AdmissionDecision::Accepted);
    assert_eq!(rig.peer.record.lock().unwrap().attempts.len(), 1);
    assert_eq!(rig.control.pending_attempt_count(), 1);
    let request_id = &rig.peer.record.lock().unwrap().attempts[0];
    assert!(rig.dispatcher.is_task_attempt(request_id));
    assert_eq!(rig.dispatcher.pending_count(), 1);
}

#[tokio::test]
async fn test_case_function_attempt_carries_capability_and_prefers_origin_session() {
    let rig = control_rig_with_sessions(vec![
        dispatch_harness::session_state("s1", "runtime-a", 1),
        dispatch_harness::session_state("s2", "runtime-b", 2),
    ]);
    let now = rig.store.now().await.expect("now");
    rig.store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let mut claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    claimed.test_case = Some(TaskTestCaseAuthority {
        test_case_capability: "test-case:cap-1".to_string(),
        parent_request_id: "parent-request".to_string(),
        origin_runtime_id: "runtime-b".to_string(),
        origin_connection_generation: 2,
    });
    let decision = rig.admission.admit(&claimed).await;
    assert_eq!(decision, AdmissionDecision::Accepted);
    let peer = rig.peer.record.lock().unwrap();
    assert_eq!(peer.attempts.len(), 1);
    let header = &peer.attempt_headers[0];
    assert!(header.test_effects_enabled);
    assert_eq!(
        header.test_case_capability.as_deref(),
        Some("test-case:cap-1")
    );
    let request_id = &peer.attempts[0];
    let lease = rig
        .dispatcher
        .pending_lease(request_id)
        .expect("test task attempt must be pending");
    assert_eq!(lease.session_epoch.replica_id, "runtime-b");
    assert_eq!(lease.session_epoch.connection_generation, 2);
}

#[tokio::test]
async fn test_case_function_attempt_without_origin_candidate_is_permanent_failure() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    rig.store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let mut claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    claimed.test_case = Some(TaskTestCaseAuthority {
        test_case_capability: "test-case:cap-1".to_string(),
        parent_request_id: "parent-request".to_string(),
        origin_runtime_id: "runtime-missing".to_string(),
        origin_connection_generation: 9,
    });
    let decision = rig.admission.admit(&claimed).await;
    assert!(
        matches!(decision, AdmissionDecision::PermanentFailure { .. }),
        "a test-case task with no origin connection must fail closed permanently: {decision:?}"
    );
    assert_eq!(rig.peer.record.lock().unwrap().attempts.len(), 0);
}

#[tokio::test]
async fn test_case_submission_gate_observes_first_admission_outcome() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    rig.store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let mut claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    claimed.test_case = Some(TaskTestCaseAuthority {
        test_case_capability: "test-case:cap-1".to_string(),
        parent_request_id: "parent-request".to_string(),
        origin_runtime_id: "runtime-a".to_string(),
        origin_connection_generation: 1,
    });
    let task_id = claimed.task_id.clone();
    let admit = tokio::spawn({
        let admission = Arc::clone(&rig.admission);
        let record = claimed.clone();
        async move { admission.admit(&record).await }
    });
    let outcome = rig
        .control
        .wait_for_first_admission(&task_id, Duration::from_secs(2))
        .await;
    let decision = admit.await.expect("admission task");
    assert_eq!(decision, AdmissionDecision::Accepted);
    assert_eq!(
        outcome,
        Some(FirstAdmissionOutcome::Accepted),
        "the test-case submission gate must observe the first admitted attempt"
    );
}

#[tokio::test]
async fn admission_rejected_provable_when_image_not_admitted() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    let mut image = corpus_image();
    image.assembly = RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "f".repeat(64)
        )),
    };
    rig.store
        .create(record(TASK_ID, image, now, TaskState::Scheduled))
        .await
        .expect("create");
    let claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    let decision = rig.admission.admit(&claimed).await;
    assert!(matches!(
        decision,
        AdmissionDecision::RejectedProvable { .. }
    ));
    assert_eq!(rig.peer.record.lock().unwrap().attempts.len(), 0);
}

#[tokio::test]
async fn admission_rejected_provable_when_no_runtime_candidate() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    rig.store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let claimed = claim_ready(rig.store.as_ref(), TASK_ID).await;
    // Remove the candidate session by closing it through the dispatcher
    // observation so selection has nobody to admit.
    rig.dispatcher.on_session_closed(&rig.session);
    let decision = rig.admission.admit(&claimed).await;
    assert!(matches!(
        decision,
        AdmissionDecision::RejectedProvable { .. }
    ));
}

#[tokio::test]
async fn admission_uncertain_when_control_plane_not_assembled() {
    // A fresh control plane whose deferred dispatcher is not yet installed.
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let clock = Arc::new(TestClock::default());
    let counters = Arc::new(TaskControlCounters::default());
    let deferred_scheduler: Arc<Mutex<Option<Arc<Scheduler>>>> = Arc::new(Mutex::new(None));
    let deferred_dispatcher: Arc<Mutex<Option<Arc<RequestDispatcher>>>> =
        Arc::new(Mutex::new(None));
    let control = Arc::new(DurableTaskControl::new(
        Arc::clone(&store),
        deferred_scheduler,
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&counters),
        noop_telemetry(),
        Duration::from_millis(20),
    ));
    let (actor, actor_port, deferred_actor_sink) = actor_lane_stub();
    let admission = RouterTaskAttemptAdmission::new(
        Arc::new(FakeImageSource::new(corpus_image())),
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&control),
        Arc::clone(&clock) as Arc<dyn Clock>,
        5_000,
        Arc::clone(&counters),
        noop_telemetry(),
        actor,
        actor_port as Arc<dyn TaskActorOwnerPort>,
        30_000,
        deferred_actor_sink,
    );
    let now = store.now().await.expect("now");
    store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let claimed = claim_ready(store.as_ref(), TASK_ID).await;
    let decision = admission.admit(&claimed).await;
    assert!(matches!(decision, AdmissionDecision::Uncertain { .. }));
}

#[tokio::test]
async fn admission_permanent_failure_when_claim_has_no_lease() {
    let rig = control_rig();
    let now = rig.store.now().await.expect("now");
    let mut not_claimed = record(TASK_ID, corpus_image(), now, TaskState::Ready);
    not_claimed.state = TaskState::Leased;
    let decision = rig.admission.admit(&not_claimed).await;
    assert!(matches!(
        decision,
        AdmissionDecision::PermanentFailure { .. }
    ));
}

// ---------------------------------------------------------------------------
// Settlement mapping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn settlement_maps_response_end_error_timeout_and_disconnect() {
    let rig = control_rig();
    // 1) response.end -> succeeded
    rig.store
        .create(record(
            "task-end",
            corpus_image(),
            rig.store.now().await.expect("now"),
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    rig.scheduler.scan_once().await;
    assert_eq!(
        status_kind(rig.store.as_ref(), "task-end").await,
        TaskStatusKind::Running,
        "scheduler claim + admission must lease the task"
    );
    let end_request = rig.peer.record.lock().unwrap().attempts.pop().unwrap();
    let _ = rig.dispatcher.on_frame(
        &rig.session,
        skiff_router::dispatch::RuntimeResponseFrame::End {
            request_id: end_request,
            payload_present: false,
            payload: Vec::new(),
        },
    );
    wait_for_status(rig.store.as_ref(), "task-end", TaskStatusKind::Succeeded).await;
    rig.scheduler.renew_active_leases().await;

    // 2) response.error -> failed
    rig.store
        .create(record(
            "task-error",
            corpus_image(),
            rig.store.now().await.expect("now"),
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    rig.scheduler.scan_once().await;
    let error_request = rig.peer.record.lock().unwrap().attempts.pop().unwrap();
    let _ = rig.dispatcher.on_frame(
        &rig.session,
        skiff_router::dispatch::RuntimeResponseFrame::Error {
            request_id: error_request,
            error: skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(
                skiff_runtime_transport::protocol::RuntimeErrorFramePayload {
                    code: "targetFailed".to_string(),
                    message: "boom".to_string(),
                    status: None,
                    details: None,
                },
            ),
        },
    );
    wait_for_status(rig.store.as_ref(), "task-error", TaskStatusKind::Failed).await;
    rig.scheduler.renew_active_leases().await;

    // 3) ordinary request timeout -> failed (no rerun)
    rig.store
        .create(record(
            "task-timeout",
            corpus_image(),
            rig.store.now().await.expect("now"),
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    rig.scheduler.scan_once().await;
    let timeout_request = rig.peer.record.lock().unwrap().attempts.pop().unwrap();
    let _ = rig.dispatcher.timeout(&timeout_request);
    wait_for_status(rig.store.as_ref(), "task-timeout", TaskStatusKind::Failed).await;
    rig.scheduler.renew_active_leases().await;

    // 4) disconnect -> no settlement; lease bookkeeping forgotten so store
    //    lease expiry drives recovery.
    rig.store
        .create(record(
            "task-disconnect",
            corpus_image(),
            rig.store.now().await.expect("now"),
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    rig.scheduler.scan_once().await;
    assert_eq!(rig.scheduler.active_lease_count(), 1);
    let _ = rig.dispatcher.on_session_closed(&rig.session);
    assert_eq!(
        rig.scheduler.active_lease_count(),
        0,
        "disconnect must stop renewing so lease expiry recovers"
    );
    assert_eq!(
        status_kind(rig.store.as_ref(), "task-disconnect").await,
        TaskStatusKind::Running,
        "uncertain terminal must not settle"
    );
    rig.worker.abort();
}

#[tokio::test]
async fn scheduler_handles_all_four_admission_decisions_via_store() {
    // Scheduler-level mapping of the seam decisions (RejectedProvable /
    // Uncertain / PermanentFailure / Accepted) is exercised through the real
    // scheduler + memory store with a scripted seam.
    let store = Arc::new(MemoryTaskStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let seam = Arc::new(ScriptedSeam::new());
    let scheduler = Arc::new(Scheduler::new(
        Arc::clone(&store_dyn),
        Arc::clone(&seam) as Arc<dyn AttemptAdmission>,
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy {
            // Long backoff keeps the released task out of the next scans so
            // each scan consumes exactly one scripted decision.
            base: DurableDuration::from_millis(60_000),
            max: DurableDuration::from_millis(60_000),
            jitter_span: DurableDuration::from_millis(0),
            jitter: Box::new(skiff_task_control::scheduler::FixedJitter(0)),
        },
    ));
    let now = store.now().await.expect("now");

    // Accepted -> active lease tracked.
    store
        .create(record(
            "task-accepted",
            corpus_image(),
            now,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    seam.push(AdmissionDecision::Accepted);
    scheduler.scan_once().await;
    assert_eq!(scheduler.active_lease_count(), 1);

    // RejectedProvable -> released back to ready with retry not-before.
    store
        .create(record(
            "task-rejected",
            corpus_image(),
            now,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    seam.push(AdmissionDecision::RejectedProvable {
        reason: "no runtime".to_string(),
    });
    scheduler.scan_once().await;
    let rejected = find_record(&store, "task-rejected").await;
    assert_eq!(rejected.state, TaskState::Ready);
    assert!(rejected.retry_not_before.is_some());

    // Uncertain -> no settle, no release; lease stays until expiry.
    store
        .create(record(
            "task-uncertain",
            corpus_image(),
            now,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    seam.push(AdmissionDecision::Uncertain {
        reason: "unknown".to_string(),
    });
    scheduler.scan_once().await;
    let uncertain = find_record(&store, "task-uncertain").await;
    assert_eq!(uncertain.state, TaskState::Leased);
    assert_eq!(scheduler.active_lease_count(), 1);

    // PermanentFailure -> settled platform-failed.
    store
        .create(record(
            "task-permanent",
            corpus_image(),
            now,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    seam.push(AdmissionDecision::PermanentFailure {
        reason: "bad record".to_string(),
    });
    scheduler.scan_once().await;
    assert_eq!(
        status_kind(store_dyn.as_ref(), "task-permanent").await,
        TaskStatusKind::PlatformFailed
    );
}

#[derive(Debug)]
struct ScriptedSeam {
    decisions: Mutex<Vec<AdmissionDecision>>,
}

impl ScriptedSeam {
    fn new() -> Self {
        Self {
            decisions: Mutex::new(Vec::new()),
        }
    }

    fn push(&self, decision: AdmissionDecision) {
        self.decisions.lock().expect("decisions").push(decision);
    }
}

#[async_trait]
impl AttemptAdmission for ScriptedSeam {
    async fn admit(&self, _record: &TaskRecord) -> AdmissionDecision {
        self.decisions
            .lock()
            .expect("decisions")
            .pop()
            .unwrap_or(AdmissionDecision::Accepted)
    }
}

// ---------------------------------------------------------------------------
// Status / cancel projection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_and_cancel_project_reference_kinds() {
    let (store, _scheduler, sink, writer, _counters) = sink_rig();
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };

    // scheduled -> running after claim -> succeeded after settle.
    let now = store.now().await.expect("now");
    store
        .create(record(TASK_ID, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let request = status_request(&task_ref(TASK_ID));
    let bytes = encode_task_status_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let response = poll_writer(&writer, 1).await;
    let decoded = decode_task_status_response_frame(&response).expect("decode");
    assert_eq!(decoded.status.kind, TaskStatusKindWire::Scheduled);

    let claimed = claim_ready(store.as_ref(), TASK_ID).await;
    let request = status_request(&task_ref(TASK_ID));
    let bytes = encode_task_status_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let response = poll_writer(&writer, 2).await;
    let decoded = decode_task_status_response_frame(&response).expect("decode");
    assert_eq!(decoded.status.kind, TaskStatusKindWire::Running);

    let now = store.now().await.expect("now");
    store
        .settle(SettleInput {
            task_id: claimed.task_id.clone(),
            lease_id: claimed
                .active_lease
                .as_ref()
                .expect("lease")
                .lease_id
                .clone(),
            terminal: TaskTerminal {
                settled_at: now,
                outcome: TaskOutcome::Succeeded,
            },
        })
        .await
        .expect("settle");
    let request = status_request(&task_ref(TASK_ID));
    let bytes = encode_task_status_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let response = poll_writer(&writer, 3).await;
    let decoded = decode_task_status_response_frame(&response).expect("decode");
    assert_eq!(decoded.status.kind, TaskStatusKindWire::Succeeded);

    // Unknown task -> expired.
    let request = status_request(&task_ref("missing-task"));
    let bytes = encode_task_status_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let response = poll_writer(&writer, 4).await;
    let decoded = decode_task_status_response_frame(&response).expect("decode");
    assert_eq!(decoded.status.kind, TaskStatusKindWire::Expired);

    // Cancel: ready -> canceled; leased -> alreadyStarted; terminal ->
    // alreadyTerminal; missing -> expired.
    let task_id = "task-cancel";
    store
        .create(record(task_id, corpus_image(), now, TaskState::Scheduled))
        .await
        .expect("create");
    let _ = store
        .scan_due(DueScanInput { limit: 100 })
        .await
        .expect("scan");
    let request = cancel_request(&task_ref(task_id));
    let bytes = encode_task_cancel_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let response = poll_writer(&writer, 5).await;
    let decoded = decode_task_cancel_response_frame(&response).expect("decode");
    assert_eq!(decoded.result.kind, TaskCancelResultKindWire::Canceled);

    let leased_task = "task-cancel-leased";
    store
        .create(record(
            leased_task,
            corpus_image(),
            now,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");
    let _ = claim_ready(store.as_ref(), leased_task).await;
    let request = cancel_request(&task_ref(leased_task));
    let bytes = encode_task_cancel_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let response = poll_writer(&writer, 6).await;
    let decoded = decode_task_cancel_response_frame(&response).expect("decode");
    assert_eq!(
        decoded.result.kind,
        TaskCancelResultKindWire::AlreadyStarted
    );

    let request = cancel_request(&task_ref(task_id));
    let bytes = encode_task_cancel_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let response = poll_writer(&writer, 7).await;
    let decoded = decode_task_cancel_response_frame(&response).expect("decode");
    assert_eq!(
        decoded.result.kind,
        TaskCancelResultKindWire::AlreadyTerminal
    );

    let request = cancel_request(&task_ref("missing-cancel"));
    let bytes = encode_task_cancel_request_frame(&request).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let response = poll_writer(&writer, 8).await;
    let decoded = decode_task_cancel_response_frame(&response).expect("decode");
    assert_eq!(decoded.result.kind, TaskCancelResultKindWire::Expired);
}

#[tokio::test]
async fn status_and_cancel_transient_store_failure_is_store_unavailable_error() {
    let store = Arc::new(ScriptedControlStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let (scheduler, sink, writer, counters) = scripted_control_rig(store_dyn, Arc::clone(&store));
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };

    store.script_status(ControlQueryOutcome::Transient);
    let bytes =
        encode_task_status_request_frame(&status_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_status_error_frame(&error).expect("status error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::StoreUnavailable.as_str()
    );
    assert_eq!(
        counters.status_unavailable.load(Ordering::Relaxed),
        1,
        "transient status failure must increment statusUnavailable"
    );
    assert_eq!(counters.status_not_found.load(Ordering::Relaxed), 0);

    store.script_cancel(ControlQueryOutcome::Transient);
    let bytes =
        encode_task_cancel_request_frame(&cancel_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let error = poll_writer(&writer, 2).await;
    let decoded = decode_task_cancel_error_frame(&error).expect("cancel error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::StoreUnavailable.as_str()
    );
    assert_eq!(
        counters.cancel_unavailable.load(Ordering::Relaxed),
        1,
        "transient cancel failure must increment cancelUnavailable"
    );
    assert_eq!(counters.cancel_not_found.load(Ordering::Relaxed), 0);
    drop(scheduler);
}

#[tokio::test]
async fn status_and_cancel_store_not_found_is_not_found_error() {
    let store = Arc::new(ScriptedControlStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let (scheduler, sink, writer, counters) = scripted_control_rig(store_dyn, Arc::clone(&store));
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };

    store.script_status(ControlQueryOutcome::NotFound);
    let bytes =
        encode_task_status_request_frame(&status_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_status_error_frame(&error).expect("status error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::NotFound.as_str()
    );
    assert_eq!(counters.status_not_found.load(Ordering::Relaxed), 1);
    assert_eq!(counters.status_unavailable.load(Ordering::Relaxed), 0);

    store.script_cancel(ControlQueryOutcome::NotFound);
    let bytes =
        encode_task_cancel_request_frame(&cancel_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let error = poll_writer(&writer, 2).await;
    let decoded = decode_task_cancel_error_frame(&error).expect("cancel error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::NotFound.as_str()
    );
    assert_eq!(counters.cancel_not_found.load(Ordering::Relaxed), 1);
    assert_eq!(counters.cancel_unavailable.load(Ordering::Relaxed), 0);
    drop(scheduler);
}

#[tokio::test]
async fn status_and_cancel_unknown_owner_is_not_found_error() {
    let store = Arc::new(MemoryTaskStore::new());
    let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        store_dyn,
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let writer = Arc::new(FakeWriter::default());
    let counters = Arc::new(TaskControlCounters::default());
    let sink = Arc::new(DurableTaskFrameSink::new(
        Arc::clone(&store) as Arc<dyn TaskStore>,
        Arc::clone(&scheduler),
        Arc::new(FakeImageSource {
            image: corpus_image(),
            services: Vec::new(),
        }),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        Arc::clone(&counters),
        noop_telemetry(),
        4096,
    ));
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };

    // The taskRef owner is not in the image source's known services.
    let bytes =
        encode_task_status_request_frame(&status_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let error = poll_writer(&writer, 1).await;
    let decoded = decode_task_status_error_frame(&error).expect("status error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::NotFound.as_str()
    );
    assert_eq!(counters.status_not_found.load(Ordering::Relaxed), 1);
    assert_eq!(counters.status_expired.load(Ordering::Relaxed), 0);

    let bytes =
        encode_task_cancel_request_frame(&cancel_request(&task_ref(TASK_ID))).expect("encode");
    sink.handle(&session, &bytes).expect("cancel");
    let error = poll_writer(&writer, 2).await;
    let decoded = decode_task_cancel_error_frame(&error).expect("cancel error");
    assert_eq!(
        decoded.error.code,
        TaskControlRejectionCode::NotFound.as_str()
    );
    assert_eq!(counters.cancel_not_found.load(Ordering::Relaxed), 1);
    assert_eq!(counters.cancel_expired.load(Ordering::Relaxed), 0);
    drop(scheduler);
}

#[tokio::test]
async fn status_retention_expired_stays_stable_expired_response() {
    let (store, _scheduler, sink, writer, counters) = sink_rig();
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    let now = store.now().await.expect("now");
    let retention_ms = 30 * 24 * 60 * 60 * 1000i64;
    let past = DurableUtcTimestamp::from_millis((now.millis() - retention_ms - 1).max(0));
    store
        .create(record(
            "task-expired",
            corpus_image(),
            past,
            TaskState::Scheduled,
        ))
        .await
        .expect("create");

    let bytes = encode_task_status_request_frame(&status_request(&task_ref("task-expired")))
        .expect("encode");
    sink.handle(&session, &bytes).expect("status");
    let response = poll_writer(&writer, 1).await;
    let decoded = decode_task_status_response_frame(&response).expect("status response");
    assert_eq!(
        decoded.status.kind,
        TaskStatusKindWire::Expired,
        "retention expiry must stay a stable expired response"
    );
    assert_eq!(counters.status_expired.load(Ordering::Relaxed), 1);
    assert_eq!(counters.status_not_found.load(Ordering::Relaxed), 0);
    assert_eq!(counters.status_unavailable.load(Ordering::Relaxed), 0);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn poll_writer(writer: &FakeWriter, count: usize) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frames = writer.frames.lock().expect("frames");
            if frames.len() >= count {
                return frames[count - 1].clone();
            }
            drop(frames);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("writer frame did not arrive")
}

async fn status_kind(store: &dyn TaskStore, task_id: &str) -> TaskStatusKind {
    store
        .status(StatusInput {
            task_id: TaskId::new(task_id),
            retention: DurableDuration::from_millis(30 * 24 * 60 * 60 * 1000),
        })
        .await
        .expect("status")
        .kind
}

async fn wait_for_status(store: &dyn TaskStore, task_id: &str, expected: TaskStatusKind) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if status_kind(store, task_id).await == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("status did not converge");
}

async fn find_record(store: &Arc<MemoryTaskStore>, task_id: &str) -> TaskRecord {
    store
        .records()
        .await
        .into_iter()
        .find(|record| record.task_id.as_str() == task_id)
        .expect("record exists")
}
