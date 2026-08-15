//! E2b actor-method task execution tests: the five get-or-activate branches,
//! task-attempt actor terminal settlement mapping, and snapshot-restore
//! first-wins semantics.

mod dispatch_harness;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use skiff_artifact_identity::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, ACTOR_METHOD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, PackageArtifactRef,
    PackageBuildId, PackageLocalAbiIdentity, RecoverableExpectedTypePlan,
    RecoverableExpectedTypeRoot, ServiceDeploymentRef, TypeRefIr,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::actor::{
    ActivateInitialControlRequest, ActivationControlPort, ActorActivationBrokerOptions,
    ActorActivationRequestBroker, ActorInvocationRelay, ActorInvocationRelayOptions,
    ActorLeaseExpiryScheduler, ActorMethodCatalogView, ActorOwnerControlBroker,
    ActorOwnerRouteAuthority, ActorOwnershipRegistry, CommitFenceFacts, IdleEvictControlPort,
    LeaseSchedulerOptions,
};
use skiff_router::session::demux::InboundFrameSink;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::supervisor::actor::ActorComponents;
use skiff_router::supervisor::actor_sink::ActorFrameSink;
use skiff_router::supervisor::session_ports::SessionHandle;
use skiff_router::supervisor::ws::WsSessionWriter;
use skiff_router::task::{
    DurableTaskControl, RouterTaskAttemptAdmission, TaskActorOwnerPort, TaskControlCounters,
    TaskExecutionImageSource,
};
use skiff_router::telemetry::{NoopTaskTelemetrySink, TaskTelemetrySink};
use skiff_router::ws::Clock;
use skiff_runtime_transport::actor_method::{
    encode_actor_method_frame, ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
    ActorMethodErrorFrameHeader, ActorMethodErrorFramePayload, ActorMethodFrame,
    ActorMethodReturnFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
    ACTOR_RETURN_ENCODING_V1,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_invoke_frame, ActorOwnerFailureFrameHeader,
    ActorOwnerFailureReasonFrameHeader, ACTOR_OWNER_FAILURE_FRAME_TYPE,
};
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_task_control::model::{
    ActorActivationSnapshot, ActorDeclarationOwner, ActorDeclarationOwnerFile,
    ActorDeclarationOwnerUnit, DurableDuration, DurableUtcTimestamp, RecoverablePayload,
    ServiceOwner, TaskExecutionImageRef, TaskId, TaskRecord, TaskState, TaskStatusKind,
    TaskTestCaseAuthority, TaskTraceContext,
};
use skiff_task_control::scheduler::{
    AdmissionDecision, AttemptAdmission, RetryBackoffPolicy, Scheduler, SchedulerConfig,
};
use skiff_task_control::store::{ClaimInput, DueScanInput, StatusInput, TaskStore};
use skiff_task_control::MemoryTaskStore;
use tokio::time::timeout;

const SERVICE_ID: &str = "example.com/service-1";

fn framed(prefix: &str, byte: u8) -> String {
    let hex = String::from_utf8(vec![byte; 64]).expect("hex digit");
    format!("{prefix}:{hex}")
}

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new(framed(ACTOR_ABI_IDENTITY_PREFIX, b'a'))
}

fn implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(framed(ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, b'a'))
}

fn implementation_new() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(framed(ACTOR_IMPLEMENTATION_IDENTITY_PREFIX, b'b'))
}

fn method_identity() -> ActorMethodIdentity {
    ActorMethodIdentity::new(framed(ACTOR_METHOD_IDENTITY_PREFIX, b'a'))
}

fn actor_type_identity() -> String {
    "skiff-actor-type-v1:sha256:".to_string() + &"a".repeat(64)
}

fn actor_id_type_identity() -> String {
    "skiff-actor-id-type-v1:sha256:".to_string() + &"a".repeat(64)
}

fn declaration_owner() -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: ActorOwnerUnitFrameHeader::Service,
        file: ActorOwnerFileFrameHeader::LoadedFileIndex(0),
        actor_symbol: "Actor".to_string(),
    }
}

fn store_declaration_owner() -> ActorDeclarationOwner {
    ActorDeclarationOwner {
        unit: ActorDeclarationOwnerUnit::Service,
        file: ActorDeclarationOwnerFile::LoadedFileIndex(0),
        actor_symbol: "Actor".to_string(),
    }
}

fn actor_key_json(create_input: &[u8]) -> serde_json::Value {
    let _ = create_input;
    serde_json::json!({
        "serviceId": SERVICE_ID,
        "actorTypeIdentity": actor_type_identity(),
        "actorIdTypeIdentity": actor_id_type_identity(),
        "actorIdEncodingVersion": "v1",
        "canonicalActorIdKeyBytesBase64": "a2V5",
        "actorIdHash": format!("sha256:{}", "a".repeat(64)),
    })
}

fn actor_key() -> skiff_router::actor::ActorLogicalKey {
    let wire: skiff_runtime_transport::protocol::ActorKeyFrameMetadata =
        serde_json::from_value(actor_key_json(b"[]")).expect("key json");
    skiff_router::actor::ActorLogicalKey::from_wire(&wire)
}

fn snapshot(create_input: &[u8]) -> ActorActivationSnapshot {
    ActorActivationSnapshot {
        key: RecoverablePayload::new(
            serde_json::to_vec(&actor_key_json(create_input)).expect("key bytes"),
        ),
        create_input: RecoverablePayload::new(create_input.to_vec()),
        expected_type_plan: RecoverableExpectedTypePlan {
            root: RecoverableExpectedTypeRoot::TypeRef {
                ty: TypeRefIr::Record {
                    fields: Default::default(),
                },
            },
            root_type_identity_ref: None,
            runtime_carrier_check_required: false,
            interface_projection_refs: Vec::new(),
            interface_method_refs: Vec::new(),
            field_refs: Vec::new(),
            union_branch_refs: Vec::new(),
        },
        expected_type_plan_runtime: Some(serde_json::json!({
            "label": "record",
            "node": { "kind": "record", "fields": [] }
        })),
    }
}

fn actor_record(
    task_id: &str,
    implementation: &ActorImplementationIdentity,
    create_input: &[u8],
    due_at: DurableUtcTimestamp,
) -> TaskRecord {
    TaskRecord {
        task_id: TaskId::new(task_id),
        owner: ServiceOwner::new(SERVICE_ID),
        execution: corpus_image(),
        target: skiff_task_control::model::DetachedCallTarget::ActorMethod {
            actor: ActorRoutingRef {
                service_id: SERVICE_ID.to_string(),
                actor_abi_identity: actor_abi(),
            },
            activation: snapshot(create_input),
            implementation: implementation.clone(),
            method: method_identity(),
            declaration_owner: store_declaration_owner(),
        },
        payload: RecoverablePayload::new(br#"[1,2,3]"#.to_vec()),
        due_at,
        state: TaskState::Scheduled,
        attempt_generation: 0,
        active_lease: None,
        terminal: None,
        trace: TaskTraceContext {
            trace_id: "trace-actor".to_string(),
            span_id: None,
        },
        created_at: due_at,
        retry_not_before: None,
        test_case: None,
    }
}

fn corpus_image() -> TaskExecutionImageRef {
    TaskExecutionImageRef {
        target_profile: dispatch_harness::CORPUS_PROFILE.to_string(),
        package_version: dispatch_harness::CORPUS_CONTRACT_VERSION.to_string(),
        deployment: dispatch_harness::corpus_deployment_ref(),
    }
}

/// Temp artifact root carrying the fixture actor routing projection record
/// (M4: the catalog view lazy-loads from the artifact store; no epoch).
struct CatalogRoot {
    root: PathBuf,
}

impl CatalogRoot {
    fn new() -> Self {
        // Unique per view/rig: parallel tests must not share (and clobber)
        // the `records/actor-routing/current.json` record path.
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skiff-router-task-actor-method-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create artifact root");
        skiff_deployment::storage::CanonicalArtifactStore::create(&root)
            .expect("create artifact store");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            vec![ActorRoutingMethod {
                actor: ActorRoutingRef {
                    service_id: SERVICE_ID.to_string(),
                    actor_abi_identity: actor_abi(),
                },
                actor_implementation_identity: implementation(),
                method_identity: method_identity(),
                deployment: dispatch_harness::corpus_deployment_ref(),
                package: PackageArtifactRef {
                    package_id: "example.com/pkg".to_string(),
                    package_version: "1.0.0".to_string(),
                    package_build_id: PackageBuildId::new(framed(
                        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
                        b'c',
                    )),
                    package_local_abi_identity: PackageLocalAbiIdentity::new(framed(
                        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
                        b'c',
                    )),
                },
            }],
        )
        .expect("projection");
        let bytes =
            skiff_canonical_json::canonical_json_bytes(&projection).expect("canonical projection");
        let path = root.join(skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH);
        std::fs::create_dir_all(path.parent().expect("projection parent"))
            .expect("create projection dirs");
        std::fs::write(path, bytes).expect("write projection record");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for CatalogRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn projection_ref() -> skiff_router::artifact::ActorRoutingProjectionRef {
    skiff_router::artifact::ActorRoutingProjectionRef::new(
        skiff_artifact_identity::ArtifactRelativePath::new(
            skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH,
            "actor routing projection record",
        )
        .expect("projection path"),
    )
}

/// Always-eligible image source for the admission gate (the fixture
/// deployment is the corpus deployment).
#[derive(Debug, Default)]
struct FakeImageSource;

impl TaskExecutionImageSource for FakeImageSource {
    fn resolve(
        &self,
        _header: &skiff_runtime_transport::protocol::TaskSubmitRequestFrameHeaderV2,
    ) -> Option<TaskExecutionImageRef> {
        Some(corpus_image())
    }

    fn contains_service(&self, _service_id: &str) -> bool {
        true
    }

    fn contains_deployment(&self, _deployment: &ServiceDeploymentRef) -> bool {
        true
    }
}

#[derive(Debug, Default)]
struct TestClock {
    now_ms: AtomicU64,
}

impl TestClock {
    fn new(millis: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(millis),
        }
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
    candidates: Mutex<Vec<RuntimeSessionEpoch>>,
    sessions: Mutex<HashMap<String, RuntimeSessionEpoch>>,
    frames: Mutex<Vec<(RuntimeSessionEpoch, Vec<u8>)>>,
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

#[derive(Debug, Default)]
struct FakeActivationControl {
    requests: Mutex<Vec<ActivateInitialControlRequest>>,
}

impl ActivationControlPort for FakeActivationControl {
    fn send_activate_initial(&self, request: &ActivateInitialControlRequest) -> Result<(), String> {
        self.requests
            .lock()
            .expect("control requests")
            .push(request.clone());
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeIdleEvict;

impl IdleEvictControlPort for FakeIdleEvict {
    fn send_idle_evict(
        &self,
        _key: &skiff_router::actor::ActorLogicalKey,
        _fence: &skiff_router::actor::ActorOwnerFence,
        _eviction_request_id: &str,
        _connection: &str,
    ) -> Result<(), String> {
        Ok(())
    }
}

struct Rig {
    store: Arc<dyn TaskStore>,
    control: Arc<DurableTaskControl>,
    admission: Arc<RouterTaskAttemptAdmission>,
    sink: Arc<ActorFrameSink>,
    actor: Arc<ActorComponents>,
    activation_control: Arc<FakeActivationControl>,
    port: Arc<FakeTaskActorOwnerPort>,
    clock: Arc<TestClock>,
    session: RuntimeSessionEpoch,
    worker: tokio::task::JoinHandle<()>,
    /// Keeps the artifact root alive for the whole test: the M4 catalog view
    /// loads lazily on the first query (during admission), so the fixture
    /// root must outlive rig construction.
    _catalog_root: CatalogRoot,
}

fn rig() -> Rig {
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let clock = Arc::new(TestClock::new(1_700_000_000_000));
    let counters = Arc::new(TaskControlCounters::default());
    let deferred_scheduler: Arc<Mutex<Option<Arc<Scheduler>>>> = Arc::new(Mutex::new(None));
    let deferred_dispatcher: Arc<Mutex<Option<Arc<skiff_router::dispatch::RequestDispatcher>>>> =
        Arc::new(Mutex::new(None));
    let control = Arc::new(DurableTaskControl::new(
        Arc::clone(&store),
        Arc::clone(&deferred_scheduler),
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&counters),
        Arc::new(NoopTaskTelemetrySink) as Arc<dyn TaskTelemetrySink>,
        Duration::from_millis(20),
    ));
    let worker = control.spawn_worker();
    let catalog_root = CatalogRoot::new();
    let session_handle = SessionHandle::new();
    let registry = Arc::new(ActorOwnershipRegistry::new());
    let activation_control = Arc::new(FakeActivationControl::default());
    let activation_broker = Arc::new(ActorActivationRequestBroker::new(
        Arc::clone(&registry),
        Arc::clone(&activation_control) as Arc<dyn ActivationControlPort>,
        ActorActivationBrokerOptions::default(),
    ));
    let relay = Arc::new(ActorInvocationRelay::new(
        ActorInvocationRelayOptions::default(),
    ));
    let control_broker = Arc::new(ActorOwnerControlBroker::new(Default::default()));
    let lease_scheduler = Arc::new(ActorLeaseExpiryScheduler::new(
        Arc::clone(&registry),
        Arc::new(FakeIdleEvict),
        LeaseSchedulerOptions::default(),
    ));
    let catalog_view = Arc::new(
        ActorMethodCatalogView::new(catalog_root.path(), projection_ref()).expect("catalog view"),
    );
    let actor = Arc::new(ActorComponents {
        registry: Arc::clone(&registry),
        activation_broker,
        relay,
        control_broker,
        lease_scheduler,
        catalog_view,
        idle_evictions: Arc::new(Mutex::new(HashMap::new())),
    });
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    let port = Arc::new(FakeTaskActorOwnerPort {
        candidates: Mutex::new(vec![session.clone()]),
        sessions: Mutex::new(HashMap::from([("runtime-a".to_string(), session.clone())])),
        frames: Mutex::new(Vec::new()),
    });
    let sink = Arc::new(ActorFrameSink::new(
        Arc::clone(&actor),
        session_handle,
        Arc::new(FakeWriter::default()) as Arc<dyn WsSessionWriter>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&control) as Arc<dyn skiff_router::task::ActorAttemptTerminalSink>,
    ));
    let deferred_actor_sink: Arc<Mutex<Option<Arc<ActorFrameSink>>>> =
        Arc::new(Mutex::new(Some(Arc::clone(&sink))));
    let admission = Arc::new(RouterTaskAttemptAdmission::new(
        Arc::new(FakeImageSource),
        Arc::clone(&deferred_dispatcher),
        Arc::clone(&control),
        Arc::clone(&clock) as Arc<dyn Clock>,
        5_000,
        Arc::clone(&counters),
        Arc::new(NoopTaskTelemetrySink) as Arc<dyn TaskTelemetrySink>,
        Arc::clone(&actor),
        Arc::clone(&port) as Arc<dyn TaskActorOwnerPort>,
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
    *deferred_scheduler.lock().expect("scheduler lock") = Some(scheduler);
    Rig {
        store,
        control,
        admission,
        sink,
        actor,
        activation_control,
        port,
        clock,
        session,
        worker,
        _catalog_root: catalog_root,
    }
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

async fn create_and_claim(rig: &Rig, record: TaskRecord) -> TaskRecord {
    let task_id = record.task_id.as_str().to_string();
    rig.store.create(record).await.expect("create actor task");
    claim_ready(rig.store.as_ref(), &task_id).await
}

async fn status_kind(store: &dyn TaskStore, task_id: &str) -> TaskStatusKind {
    store
        .status(StatusInput {
            task_id: TaskId::new(task_id),
            retention: DurableDuration::from_millis(60_000),
        })
        .await
        .expect("status")
        .kind
}

async fn wait_for_status(store: &dyn TaskStore, task_id: &str, expected: TaskStatusKind) {
    timeout(Duration::from_secs(2), async {
        loop {
            if status_kind(store, task_id).await == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("status converged");
}

fn commit_owner(
    rig: &Rig,
    implementation: &ActorImplementationIdentity,
    create_input: &[u8],
) -> skiff_router::actor::ActorOwnerFence {
    let key = actor_key();
    let facts = rig.actor.registry.ensure_present(
        &key,
        actor_abi(),
        implementation.clone(),
        declaration_owner(),
        create_input,
    );
    let token = rig
        .actor
        .registry
        .reserve(
            &key,
            facts.epoch,
            "runtime-a",
            &ActorOwnerRouteAuthority {
                build_id: dispatch_harness::CORPUS_DEPLOYMENT_ARTIFACT_IDENTITY.to_string(),
            },
            0,
        )
        .expect("reserve");
    rig.actor
        .registry
        .commit(
            &token,
            &CommitFenceFacts {
                actor_abi_identity: actor_abi(),
                actor_implementation_identity: implementation.clone(),
                declaration_owner: declaration_owner(),
                owner_lease_id: "owner-lease-1".to_string(),
            },
            0,
            100_000,
        )
        .expect("commit")
}

fn owner_return_frame(invocation_id: &str) -> Vec<u8> {
    encode_actor_method_frame(&ActorMethodFrame::Return(
        ActorMethodReturnFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.return".to_string(),
            invocation_id: invocation_id.to_string(),
            return_encoding_version: ACTOR_RETURN_ENCODING_V1.to_string(),
        },
        Vec::new(),
    ))
    .expect("return frame")
}

fn owner_error_frame(invocation_id: &str, error: ActorMethodErrorFramePayload) -> Vec<u8> {
    encode_actor_method_frame(&ActorMethodFrame::Error(ActorMethodErrorFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.method.error".to_string(),
        invocation_id: invocation_id.to_string(),
        error,
    }))
    .expect("error frame")
}

fn owner_failure_frame(invocation_id: &str) -> Vec<u8> {
    skiff_runtime_transport::actor_owner::encode_actor_owner_failure_frame(
        &ActorOwnerFailureFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_OWNER_FAILURE_FRAME_TYPE.to_string(),
            invocation_id: invocation_id.to_string(),
            owner_runtime_id: "runtime-a".to_string(),
            owner_lease_id: "owner-lease-1".to_string(),
            epoch: 1,
            actor_implementation_identity: implementation(),
            reason: ActorOwnerFailureReasonFrameHeader {
                code: "ActorCreateFailed".to_string(),
                message: "create rejected the input".to_string(),
            },
        },
    )
    .expect("owner failure frame")
}

fn invoke_invocation_id(rig: &Rig) -> String {
    let frames = rig.port.frames.lock().expect("port frames");
    let (_, bytes) = frames.last().expect("owner invoke frame written");
    let (header, _) = decode_actor_owner_invoke_frame(bytes).expect("decode owner invoke");
    header.invoke.invocation_id
}

// ---------------------------------------------------------------------------
// Branch 1: live incarnation, same implementation
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn branch1_live_incarnation_same_implementation_admits_ordinary_invocation() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        let decision = rig.admission.admit(&claimed).await;
        assert_eq!(decision, AdmissionDecision::Accepted);
        assert_eq!(rig.control.pending_attempt_count(), 1);
        let frames = rig.port.frames.lock().expect("frames");
        assert_eq!(frames.len(), 1, "exactly one owner invoke, no activation");
        let (header, payload) = decode_actor_owner_invoke_frame(&frames[0].1).expect("decode");
        assert_eq!(header.activation_bootstrap, None);
        assert_eq!(payload, br#"[1,2,3]"#);
        assert_eq!(
            header.invoke.actor_implementation_identity,
            implementation()
        );
        assert_eq!(
            header.invoke.test_case_capability, None,
            "ordinary production actor attempts must not carry test-case capability"
        );
        assert_eq!(header.invoke.test_case_parent_request_id, None);
        rig.worker.abort();
    }

    #[tokio::test]
    async fn test_case_actor_attempt_carries_capability_and_parent_on_invoke() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let mut record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        record.test_case = Some(TaskTestCaseAuthority {
            test_case_capability: "test-case:cap-1".to_string(),
            parent_request_id: "parent-request".to_string(),
            origin_runtime_id: "runtime-a".to_string(),
            origin_connection_generation: 1,
        });
        let claimed = create_and_claim(&rig, record).await;
        let decision = rig.admission.admit(&claimed).await;
        assert_eq!(decision, AdmissionDecision::Accepted);
        let frames = rig.port.frames.lock().expect("frames");
        assert_eq!(frames.len(), 1, "exactly one owner invoke");
        let (header, _) = decode_actor_owner_invoke_frame(&frames[0].1).expect("decode");
        assert_eq!(
            header.invoke.test_case_capability.as_deref(),
            Some("test-case:cap-1")
        );
        assert_eq!(
            header.invoke.test_case_parent_request_id.as_deref(),
            Some("parent-request")
        );
        let invocation_id = header.invoke.invocation_id;
        assert_eq!(
            rig.actor
                .relay
                .parent_test_capability("runtime-a#1", &invocation_id),
            Some("test-case:cap-1".to_string()),
            "the relay must retain the invocation's case capability for recursive task submits"
        );
        rig.worker.abort();
    }

    #[tokio::test]
    async fn test_case_actor_attempt_without_origin_candidate_is_permanent_failure() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let mut record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        record.test_case = Some(TaskTestCaseAuthority {
            test_case_capability: "test-case:cap-1".to_string(),
            parent_request_id: "parent-request".to_string(),
            origin_runtime_id: "runtime-missing".to_string(),
            origin_connection_generation: 9,
        });
        let claimed = create_and_claim(&rig, record).await;
        let decision = rig.admission.admit(&claimed).await;
        assert!(
            matches!(decision, AdmissionDecision::PermanentFailure { .. }),
            "test-case actor task with a missing origin connection must fail closed: {decision:?}"
        );
        assert_eq!(rig.port.frames.lock().expect("frames").len(), 0);
        rig.worker.abort();
    }

    #[tokio::test]
    async fn test_case_actor_attempt_cross_service_is_permanent_failure() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let mut record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        if let skiff_task_control::model::DetachedCallTarget::ActorMethod {
            actor,
            activation,
            ..
        } = &mut record.target
        {
            actor.service_id = "example.com/service-2".to_string();
            activation.key = RecoverablePayload::new(
                serde_json::to_vec(&serde_json::json!({
                    "serviceId": "example.com/service-2",
                    "actorTypeIdentity": actor_type_identity(),
                    "actorIdTypeIdentity": actor_id_type_identity(),
                    "actorIdEncodingVersion": "v1",
                    "canonicalActorIdKeyBytesBase64": "a2V5",
                    "actorIdHash": format!("sha256:{}", "a".repeat(64)),
                }))
                .expect("cross-service key json"),
            );
        }
        record.test_case = Some(TaskTestCaseAuthority {
            test_case_capability: "test-case:cap-1".to_string(),
            parent_request_id: "parent-request".to_string(),
            origin_runtime_id: "runtime-a".to_string(),
            origin_connection_generation: 1,
        });
        let claimed = create_and_claim(&rig, record).await;
        let decision = rig.admission.admit(&claimed).await;
        assert!(
            matches!(decision, AdmissionDecision::PermanentFailure { ref reason } if reason.contains("differs from the parent service")),
            "test-case actor tasks must not cross the parent service: {decision:?}"
        );
        assert_eq!(rig.port.frames.lock().expect("frames").len(), 0);
        rig.worker.abort();
    }

    // ---------------------------------------------------------------------------
    // Branch 2: registry entry exists, no live incarnation
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn branch2_entry_exists_uses_entry_create_input_to_activate() {
        let rig = rig();
        // Registry entry exists with entry-frozen create input; no live owner.
        let key = actor_key();
        rig.actor.registry.ensure_present(
            &key,
            actor_abi(),
            implementation(),
            declaration_owner(),
            br#"[9]"#,
        );
        let record = actor_record(
            "task-actor",
            &implementation(),
            br#"[1]"#, // task snapshot differs; entry input must win
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        let admission = Arc::clone(&rig.admission);
        let record = claimed.clone();
        let admit = tokio::spawn(async move { admission.admit(&record).await });
        let request = wait_for_activation_request(&rig).await;
        assert_eq!(
            request.bootstrap_bytes, br#"[9]"#,
            "entry create input wins"
        );
        let ack = rig.actor.activation_broker.on_activation_ack(
            &request.request_id,
            &request.owner_runtime_id,
            &request.owner_connection,
            true,
            rig.clock.now_ms(),
        );
        assert!(matches!(
            ack,
            skiff_router::actor::ActivationAckOutcome::Committed { .. }
        ));
        let decision = admit.await.expect("admit task");
        assert_eq!(decision, AdmissionDecision::Accepted);
        let frames = rig.port.frames.lock().expect("frames");
        assert_eq!(frames.len(), 1, "one owner invoke after activation");
        assert_eq!(
            rig.actor.registry.entry(&key).expect("entry").create_input,
            br#"[9]"#
        );
        rig.worker.abort();
    }

    // ---------------------------------------------------------------------------
    // Branch 3: registry entry lost, snapshot restores a minimal entry
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn branch3_snapshot_restores_minimal_entry_and_first_restore_wins() {
        let rig = rig();
        let record = actor_record(
            "task-actor",
            &implementation(),
            br#"[7]"#,
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        let admission = Arc::clone(&rig.admission);
        let record = claimed.clone();
        let admit = tokio::spawn(async move { admission.admit(&record).await });
        let request = wait_for_activation_request(&rig).await;
        assert_eq!(
            request.bootstrap_bytes, br#"[7]"#,
            "snapshot create input restores the minimal entry"
        );
        let ack = rig.actor.activation_broker.on_activation_ack(
            &request.request_id,
            &request.owner_runtime_id,
            &request.owner_connection,
            true,
            rig.clock.now_ms(),
        );
        assert!(matches!(
            ack,
            skiff_router::actor::ActivationAckOutcome::Committed { .. }
        ));
        let decision = admit.await.expect("admit task");
        assert_eq!(decision, AdmissionDecision::Accepted);
        assert_eq!(
            rig.actor
                .registry
                .entry(&actor_key())
                .expect("restored entry")
                .create_input,
            br#"[7]"#
        );
        assert_eq!(
            rig.activation_control
                .requests
                .lock()
                .expect("requests")
                .len(),
            1
        );
        rig.worker.abort();
    }

    #[tokio::test]
    async fn branch3_concurrent_snapshot_restores_put_if_absent_once() {
        let rig = rig();
        let now = rig.store.now().await.expect("now");
        let first = create_and_claim(
            &rig,
            actor_record("task-actor", &implementation(), br#"[1]"#, now),
        )
        .await;
        let second = create_and_claim(
            &rig,
            actor_record("task-actor-2", &implementation(), br#"[2]"#, now),
        )
        .await;
        let admission_a = Arc::clone(&rig.admission);
        let admission_b = Arc::clone(&rig.admission);
        let record_a = first.clone();
        let record_b = second.clone();
        let task_a = tokio::spawn(async move { admission_a.admit(&record_a).await });
        let task_b = tokio::spawn(async move { admission_b.admit(&record_b).await });
        let request = wait_for_activation_request(&rig).await;
        let ack = rig.actor.activation_broker.on_activation_ack(
            &request.request_id,
            &request.owner_runtime_id,
            &request.owner_connection,
            true,
            rig.clock.now_ms(),
        );
        assert!(matches!(
            ack,
            skiff_router::actor::ActivationAckOutcome::Committed { .. }
        ));
        let decision_a = task_a.await.expect("task a");
        let decision_b = task_b.await.expect("task b");
        assert_eq!(decision_a, AdmissionDecision::Accepted);
        assert_eq!(decision_b, AdmissionDecision::Accepted);
        assert_eq!(
            rig.activation_control
                .requests
                .lock()
                .expect("requests")
                .len(),
            1,
            "concurrent restores share one identity-fenced claim"
        );
        let restored = rig
            .actor
            .registry
            .entry(&actor_key())
            .expect("restored entry")
            .create_input;
        assert!(
            restored == br#"[1]"# || restored == br#"[2]"#,
            "first successful restore wins: {restored:?}"
        );
        rig.worker.abort();
    }

    // ---------------------------------------------------------------------------
    // Branch 4: ActorUpgradingError -> release + backoff
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn branch4_upgrading_error_releases_attempt_with_backoff() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        assert_eq!(
            rig.admission.admit(&claimed).await,
            AdmissionDecision::Accepted
        );
        let invocation_id = invoke_invocation_id(&rig);
        let bytes = owner_error_frame(
            &invocation_id,
            ActorMethodErrorFramePayload::ActorUpgradingError {
                actor_ref: ActorLogicalRefFrameHeader {
                    service_id: SERVICE_ID.to_string(),
                    actor_type_identity: actor_type_identity(),
                    actor_id_type_identity: actor_id_type_identity(),
                    actor_id_encoding_version: "v1".to_string(),
                    canonical_actor_id_key_bytes_base64: "a2V5".to_string(),
                    actor_id_hash: format!("sha256:{}", "a".repeat(64)),
                    epoch: 1,
                },
                retry_after_ms: 5_000,
            },
        );
        rig.sink.handle(&rig.session, &bytes).expect("handle error");
        let released = timeout(Duration::from_secs(2), async {
            loop {
                let record = rig
                    .store
                    .status(StatusInput {
                        task_id: TaskId::new("task-actor"),
                        retention: DurableDuration::from_millis(60_000),
                    })
                    .await
                    .expect("status");
                match record.kind {
                    TaskStatusKind::Running => {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    TaskStatusKind::Ready => {
                        let records = rig
                            .store
                            .scan_due(DueScanInput { limit: 10 })
                            .await
                            .expect("scan");
                        let record = records
                            .into_iter()
                            .find(|record| record.task_id.as_str() == "task-actor")
                            .expect("ready task");
                        return record;
                    }
                    other => panic!("unexpected status {other:?}"),
                }
            }
        })
        .await
        .expect("released to ready");
        let not_before = released.retry_not_before.expect("backoff set");
        let now = rig.store.now().await.expect("now");
        assert!(
            not_before > now,
            "release must carry future retry-not-before ({not_before} > {now})"
        );
        assert!(
            rig.control
                .counters()
                .settlements_upgrading
                .load(Ordering::Relaxed)
                >= 1
        );
        rig.worker.abort();
    }

    // ---------------------------------------------------------------------------
    // Branch 5: fence taken over by a new implementation -> platform-failed
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn branch5_taken_over_implementation_rejects_old_task_platform_failed() {
        let rig = rig();
        commit_owner(&rig, &implementation_new(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        let decision = rig.admission.admit(&claimed).await;
        assert!(matches!(
            decision,
            AdmissionDecision::PermanentFailure { reason } if reason.contains("ActorVersionRejectedError")
        ));
        assert!(
            rig.control
                .counters()
                .admissions_permanent_failure
                .load(Ordering::Relaxed)
                >= 1
        );
        rig.worker.abort();
    }

    // ---------------------------------------------------------------------------
    // Settlement mapping through the actor frame sink
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn actor_attempt_return_settles_succeeded() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        assert_eq!(
            rig.admission.admit(&claimed).await,
            AdmissionDecision::Accepted
        );
        let invocation_id = invoke_invocation_id(&rig);
        rig.sink
            .handle(&rig.session, &owner_return_frame(&invocation_id))
            .expect("handle return");
        wait_for_status(rig.store.as_ref(), "task-actor", TaskStatusKind::Succeeded).await;
        rig.worker.abort();
    }

    #[tokio::test]
    async fn actor_attempt_owner_failure_settles_failed() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        assert_eq!(
            rig.admission.admit(&claimed).await,
            AdmissionDecision::Accepted
        );
        let invocation_id = invoke_invocation_id(&rig);
        rig.sink
            .handle(&rig.session, &owner_failure_frame(&invocation_id))
            .expect("handle owner failure");
        wait_for_status(rig.store.as_ref(), "task-actor", TaskStatusKind::Failed).await;
        rig.worker.abort();
    }

    #[tokio::test]
    async fn actor_attempt_version_rejected_settles_platform_failed() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        assert_eq!(
            rig.admission.admit(&claimed).await,
            AdmissionDecision::Accepted
        );
        let invocation_id = invoke_invocation_id(&rig);
        let bytes = owner_error_frame(
            &invocation_id,
            ActorMethodErrorFramePayload::ActorVersionRejectedError {
                actor_ref: ActorLogicalRefFrameHeader {
                    service_id: SERVICE_ID.to_string(),
                    actor_type_identity: actor_type_identity(),
                    actor_id_type_identity: actor_id_type_identity(),
                    actor_id_encoding_version: "v1".to_string(),
                    canonical_actor_id_key_bytes_base64: "a2V5".to_string(),
                    actor_id_hash: format!("sha256:{}", "a".repeat(64)),
                    epoch: 1,
                },
                requested_implementation_identity: implementation(),
                accepted_implementation_identity: implementation_new(),
            },
        );
        rig.sink.handle(&rig.session, &bytes).expect("handle error");
        wait_for_status(
            rig.store.as_ref(),
            "task-actor",
            TaskStatusKind::PlatformFailed,
        )
        .await;
        rig.worker.abort();
    }

    #[tokio::test]
    async fn actor_attempt_owner_disconnect_is_uncertain_no_settlement() {
        let rig = rig();
        commit_owner(&rig, &implementation(), b"[]");
        let record = actor_record(
            "task-actor",
            &implementation(),
            b"[]",
            rig.store.now().await.expect("now"),
        );
        let claimed = create_and_claim(&rig, record).await;
        assert_eq!(
            rig.admission.admit(&claimed).await,
            AdmissionDecision::Accepted
        );
        // The owner session closes: no settlement, the attempt stays leased and
        // lease expiry recovery owns the next attempt.
        rig.sink.on_runtime_session_closed(&rig.session);
        assert_eq!(
            status_kind(rig.store.as_ref(), "task-actor").await,
            TaskStatusKind::Running
        );
        rig.worker.abort();
    }
}

async fn wait_for_activation_request(rig: &Rig) -> ActivateInitialControlRequest {
    timeout(Duration::from_secs(2), async {
        loop {
            let requests = rig.activation_control.requests.lock().expect("requests");
            if let Some(request) = requests.first() {
                return request.clone();
            }
            drop(requests);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("activateInitial request")
}
