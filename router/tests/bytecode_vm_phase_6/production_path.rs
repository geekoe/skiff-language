//! Production-shaped task/actor router proof: the durable TaskStore record
//! and Actor owner fence are created from the same wire frames and lane
//! consumers used by the production Router composition, not from hand-built
//! `TaskRecord` / `ActorOwnerFence` structs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use skiff_artifact_identity::{
    assign_service_deployment_identity, service_deployment_ref, ArtifactRelativePath,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, PackageArtifactRef,
    PackageBuildId, PackageLocalAbiIdentity, ServiceDeploymentRef,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::fixtures::service_deployment_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_RECORD_PATH, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_router::actor::{ActorLogicalKey, ActorOwnershipRegistry};
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::config::RouterConfig;
use skiff_router::routing::DispatchCapabilities;
use skiff_router::session::consumer::{ConsumerKind, ConsumerManifest};
use skiff_router::session::directory::RegistrationFacts;
use skiff_router::session::health::RuntimeHealthLedger;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::layer::{
    SessionFrameWriter, SessionLayer, SessionLayerOptions, SessionRegistrationFacts,
};
use skiff_router::supervisor::actor::assemble_actor_components;
use skiff_router::supervisor::actor_sink::ActorFrameSink;
use skiff_router::supervisor::session_ports::SessionHandle;
use skiff_router::supervisor::ws::WsSessionWriter;
use skiff_router::task::{
    ActorAttemptTerminal, ActorAttemptTerminalSink, DurableTaskControl, DurableTaskFrameSink,
    NoopTaskSubmitParentResolver, ReleaseTaskExecutionImageSource, RouterTaskAttemptAdmission,
    SessionTaskActorOwnerPort, TaskControlCounters, TaskExecutionImageSource,
};
use skiff_router::telemetry::{NoopTaskTelemetrySink, TaskTelemetrySink};
use skiff_router::ws::Clock;
use skiff_runtime_transport::actor_method::{
    encode_actor_method_frame, ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
    ActorMethodFrame, ActorMethodReturnFrameHeader, ActorOwnerFileFrameHeader,
    ActorOwnerUnitFrameHeader, ACTOR_RETURN_ENCODING_V1,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_invoke_frame,
    encode_actor_owner_control_ack_frame, ActorOwnerControlAckFrameHeader,
    ActorOwnerControlOperation, ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE,
};
use skiff_runtime_transport::protocol::{
    decode_task_submit_error_frame, decode_task_submit_response_frame,
    encode_task_submit_request_frame, ActivationIdentityFrameMetadata,
    TaskActorActivationSnapshotFrameMetadata, TaskActorMethodTargetFrameMetadata, TaskCallerKind,
    TaskSubmitRequestFrameHeaderV2, TaskTargetKind, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_task_control::clock::TaskClock;
use skiff_task_control::model::{
    DurableUtcTimestamp, TaskId, TaskOutcome, TaskRecord, TaskState, TaskTerminal,
};
use skiff_task_control::scheduler::{
    AdmissionDecision, RetryBackoffPolicy, Scheduler, SchedulerConfig,
};
use skiff_task_control::store::{
    ClaimInput, DueScanInput, LeaseRecoveryInput, RenewInput, SettleInput, TaskStore,
};
use skiff_task_control::MemoryTaskStore;
use tokio::time::timeout;

const PROFILE: &str = "prod";
const SERVICE_ID: &str = "example.echo";
const VERSION: &str = "1.0.0";
const RUNTIME_ID: &str = "runtime-a";

fn framed(prefix: &str, byte: u8) -> String {
    let hex = String::from_utf8(vec![byte; 64]).expect("identity digest");
    format!("{prefix}:{hex}")
}

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new(framed(
        skiff_artifact_identity::ACTOR_ABI_IDENTITY_PREFIX,
        b'a',
    ))
}

fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(framed(
        skiff_artifact_identity::ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
        b'b',
    ))
}

fn actor_method() -> ActorMethodIdentity {
    ActorMethodIdentity::new(framed(
        skiff_artifact_identity::ACTOR_METHOD_IDENTITY_PREFIX,
        b'c',
    ))
}

fn actor_type_identity() -> String {
    format!("skiff-actor-type-v1:sha256:{}", "a".repeat(64))
}

fn actor_id_type_identity() -> String {
    format!("skiff-actor-id-type-v1:sha256:{}", "b".repeat(64))
}

fn actor_id_hash() -> String {
    format!("sha256:{}", "c".repeat(64))
}

fn declaration_owner() -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: ActorOwnerUnitFrameHeader::Service,
        file: ActorOwnerFileFrameHeader::LoadedFileIndex(0),
        actor_symbol: "Counter".to_string(),
    }
}

fn actor_key() -> ActorLogicalKey {
    ActorLogicalKey {
        service_id: SERVICE_ID.to_string(),
        actor_type_identity: actor_type_identity(),
        actor_id_type_identity: actor_id_type_identity(),
        actor_id_encoding_version: "skiff-actor-id-encoding-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "YWxpY2U=".to_string(),
        actor_id_hash: actor_id_hash(),
    }
}

fn actor_key_json() -> serde_json::Value {
    serde_json::json!({
        "serviceId": SERVICE_ID,
        "actorTypeIdentity": actor_type_identity(),
        "actorIdTypeIdentity": actor_id_type_identity(),
        "actorIdEncodingVersion": "skiff-actor-id-encoding-v1",
        "canonicalActorIdKeyBytesBase64": "YWxpY2U=",
        "actorIdHash": actor_id_hash(),
    })
}

#[derive(Debug, Default)]
struct RecordingWriter {
    frames: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl WsSessionWriter for RecordingWriter {
    fn write(&self, _runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        self.frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(bytes);
        Ok(())
    }
}

impl SessionFrameWriter for RecordingWriter {
    fn enqueue(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(bytes);
        Ok(())
    }
}

#[derive(Debug)]
struct TestClock {
    now_ms: AtomicU64,
}

impl TestClock {
    fn new(now_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(now_ms),
        }
    }

    fn set_millis(&self, now_ms: u64) {
        self.now_ms.store(now_ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

impl TaskClock for TestClock {
    fn now_millis(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst) as i64
    }
}

#[derive(Debug, Default)]
struct RecordingActorAttemptTerminalSink {
    terminals: Mutex<Vec<ActorAttemptTerminal>>,
}

impl ActorAttemptTerminalSink for RecordingActorAttemptTerminalSink {
    fn on_actor_terminal(
        &self,
        _request_id: &str,
        _task_id: &str,
        _attempt_id: &str,
        _lease_id: &str,
        terminal: ActorAttemptTerminal,
    ) {
        self.terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(terminal);
    }
}

struct ArtifactRoot {
    root: PathBuf,
}

impl ArtifactRoot {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "skiff-bcvm-p6-router-proof-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create proof artifact root");
        Self { root }
    }
}

impl Drop for ArtifactRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn package_ref() -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: SERVICE_ID.to_string(),
        package_version: VERSION.to_string(),
        package_build_id: PackageBuildId::new(framed(PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, b'e')),
        package_local_abi_identity: PackageLocalAbiIdentity::new(framed(
            PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
            b'f',
        )),
    }
}

fn materialize_artifact_root() -> (ArtifactRoot, CanonicalArtifactStore, ServiceDeploymentRef) {
    let root = ArtifactRoot::new();
    let store = CanonicalArtifactStore::create(&root.root).expect("create artifact store");
    let mut deployment = service_deployment_fixture().expect("deployment fixture");
    assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
    let reference = service_deployment_ref(&deployment);
    store
        .write_service_deployment(&deployment)
        .expect("write service deployment");
    store
        .write_release_pointer(&ReleasePointer::new(PROFILE, reference.clone()).expect("pointer"))
        .expect("write release pointer");

    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        vec![ActorRoutingMethod {
            actor: ActorRoutingRef {
                service_id: SERVICE_ID.to_string(),
                actor_abi_identity: actor_abi(),
            },
            actor_implementation_identity: actor_implementation(),
            method_identity: actor_method(),
            deployment: reference.clone(),
            package: package_ref(),
        }],
    )
    .expect("actor routing projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection");
    let path = root.root.join(ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    std::fs::create_dir_all(path.parent().expect("projection parent"))
        .expect("create projection dirs");
    std::fs::write(path, bytes).expect("write projection");
    (root, store, reference)
}

fn router_config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        run_dir: None,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: PROFILE.to_string(),
        release_mode: Some(true),
        request_timeout_ms: 5_000,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff-proof".to_string(),
        },
        telemetry: None,
        profile_sampling: None,
        websocket_path: "/ws".to_string(),
    }
}

struct ProofHarness {
    _root: ArtifactRoot,
    deployment: ServiceDeploymentRef,
    store: Arc<MemoryTaskStore>,
    clock: Arc<TestClock>,
    writer: Arc<RecordingWriter>,
    session: RuntimeSessionEpoch,
    sink: Arc<DurableTaskFrameSink>,
    admission: Arc<RouterTaskAttemptAdmission>,
    actor_sink: Arc<ActorFrameSink>,
    terminal_sink: Arc<RecordingActorAttemptTerminalSink>,
    registry: Arc<ActorOwnershipRegistry>,
}

impl ProofHarness {
    fn new() -> Self {
        let (root, artifact_store, deployment) = materialize_artifact_root();
        let clock = Arc::new(TestClock::new(1_700_000_000_000));
        let store_clock: Arc<dyn TaskClock> = Arc::clone(&clock) as Arc<dyn TaskClock>;
        let store = Arc::new(MemoryTaskStore::with_clock(store_clock));
        let writer = Arc::new(RecordingWriter::default());
        let store_dyn = Arc::clone(&store) as Arc<dyn TaskStore>;

        let config = router_config(&root.root);
        let session_layer = Arc::new(
            SessionLayer::with_options(
                config,
                SessionLayerOptions {
                    manifest: ConsumerManifest::default_installed(),
                    consumers: vec![Arc::new(RuntimeHealthLedger::new())],
                    timing: Default::default(),
                    budgets: Default::default(),
                    writer_delay: None,
                },
            )
            .expect("session layer"),
        );
        let session_handle = SessionHandle::new();
        session_handle.set(Arc::clone(&session_layer));
        let session = RuntimeSessionEpoch {
            replica_id: RUNTIME_ID.to_string(),
            connection_generation: 1,
        };
        session_layer
            .register_frame_writer(&session, Arc::clone(&writer) as Arc<dyn SessionFrameWriter>);
        session_layer
            .directory_lock()
            .publish_pending(&session, &[ConsumerKind::HealthLedger])
            .expect("publish pending session");
        let canonical_root = std::fs::canonicalize(&root.root)
            .expect("canonicalize artifact root")
            .to_string_lossy()
            .to_string();
        session_layer.record_registration_facts(
            &session,
            SessionRegistrationFacts {
                dispatch: DispatchCapabilities::default(),
                registration: RegistrationFacts {
                    registered_build_ids: Vec::new(),
                    lazy_load: true,
                    artifact_root: Some(canonical_root),
                },
            },
        );
        session_layer.directory_lock().mark_registered(&session);

        let actor_projection = ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("projection path"),
        );
        let actor = assemble_actor_components(&root.root, actor_projection, session_handle.clone())
            .expect("actor components");
        let terminal_sink = Arc::new(RecordingActorAttemptTerminalSink::default());
        let actor_sink = Arc::new(ActorFrameSink::new(
            Arc::clone(&actor),
            session_handle.clone(),
            Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&terminal_sink) as Arc<dyn ActorAttemptTerminalSink>,
        ));

        let counters = Arc::new(TaskControlCounters::default());
        let telemetry = Arc::new(NoopTaskTelemetrySink) as Arc<dyn TaskTelemetrySink>;
        let deferred_scheduler: Arc<Mutex<Option<Arc<Scheduler>>>> = Arc::new(Mutex::new(None));
        let deferred_dispatcher: Arc<
            Mutex<Option<Arc<skiff_router::dispatch::RequestDispatcher>>>,
        > = Arc::new(Mutex::new(None));
        let control = Arc::new(DurableTaskControl::new(
            Arc::clone(&store_dyn),
            Arc::clone(&deferred_scheduler),
            Arc::clone(&deferred_dispatcher),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&counters),
            Arc::clone(&telemetry),
            Duration::from_millis(20),
        ));
        let deferred_actor_sink: Arc<Mutex<Option<Arc<ActorFrameSink>>>> =
            Arc::new(Mutex::new(Some(Arc::clone(&actor_sink))));
        let actor_port = Arc::new(SessionTaskActorOwnerPort::new(
            session_handle.clone(),
            Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
        ));
        let image_source: Arc<dyn TaskExecutionImageSource> =
            Arc::new(ReleaseTaskExecutionImageSource::new(
                PROFILE.to_string(),
                Arc::new(skiff_router::release::StoreReleaseResolver::new(
                    artifact_store.clone(),
                )),
            ));
        let admission = Arc::new(RouterTaskAttemptAdmission::new(
            Arc::clone(&image_source),
            Arc::clone(&deferred_dispatcher),
            Arc::clone(&control),
            Arc::clone(&clock) as Arc<dyn Clock>,
            5_000,
            Arc::clone(&counters),
            Arc::clone(&telemetry),
            Arc::clone(&actor),
            Arc::clone(&actor_port) as Arc<dyn skiff_router::task::TaskActorOwnerPort>,
            5_000,
            Arc::clone(&deferred_actor_sink),
        ));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store_dyn),
            Arc::clone(&admission) as Arc<dyn skiff_task_control::scheduler::AttemptAdmission>,
            Arc::clone(&clock) as Arc<dyn TaskClock>,
            SchedulerConfig::default(),
            RetryBackoffPolicy::default(),
        ));
        *deferred_scheduler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&scheduler));
        let sink = Arc::new(DurableTaskFrameSink::new(
            Arc::clone(&store_dyn),
            scheduler,
            image_source,
            Arc::new(NoopTaskSubmitParentResolver)
                as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
            Some(Arc::clone(&control)),
            Arc::clone(&writer) as Arc<dyn WsSessionWriter>,
            counters,
            telemetry,
            4096,
        ));

        Self {
            _root: root,
            deployment,
            store,
            clock,
            writer,
            session,
            sink,
            admission,
            actor_sink,
            terminal_sink,
            registry: actor.registry.clone(),
        }
    }

    async fn submit(&self, header: TaskSubmitRequestFrameHeaderV2, payload: &[u8]) {
        let bytes = encode_task_submit_request_frame(&header, payload).expect("encode task submit");
        self.sink
            .handle_submit(self.session.clone(), header, payload.to_vec())
            .await
            .expect("production task sink accepted frame");
        let _ = bytes;
    }

    async fn claim(&self, task_id: &str) -> TaskRecord {
        let now = self.store.now().await.expect("store now");
        let records = self
            .store
            .scan_due(DueScanInput { limit: 10 })
            .await
            .expect("scan due");
        let record = records
            .into_iter()
            .find(|record| record.task_id.as_str() == task_id)
            .expect("task is due and ready");
        let expiry = now.checked_add_millis(60_000).expect("lease expiry");
        match self
            .store
            .claim(ClaimInput {
                task_id: record.task_id.clone(),
                owner: "proof-scheduler".to_string(),
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
}

fn function_header(
    task_id: &str,
    deployment: &ServiceDeploymentRef,
) -> TaskSubmitRequestFrameHeaderV2 {
    TaskSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.submit.request".to_string(),
        rpc_id: format!("rpc:{task_id}"),
        runtime_id: RUNTIME_ID.to_string(),
        caller_kind: TaskCallerKind::Request,
        caller_request_id: format!("parent:{task_id}"),
        target_kind: TaskTargetKind::Function,
        service_id: SERVICE_ID.to_string(),
        service_version: VERSION.to_string(),
        service_protocol_identity: format!("{SERVICE_ID}:{VERSION}"),
        target: format!("{SERVICE_ID}:run"),
        timing: None,
        task_id: Some(task_id.to_string()),
        build_id: Some(deployment.deployment_artifact_identity.to_string()),
        activation_identity: ActivationIdentityFrameMetadata {
            assembly_identity: format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
            generation: 1,
            runtime_replica_id: RUNTIME_ID.to_string(),
            deployment_revision: "revision-1".to_string(),
        },
        trace_id: Some(format!("trace:{task_id}")),
        caller_target: Some(format!("{SERVICE_ID}:run")),
        max_queue_wait_ms: None,
        actor_method: None,
    }
}

fn actor_method_header(
    task_id: &str,
    deployment: &ServiceDeploymentRef,
) -> TaskSubmitRequestFrameHeaderV2 {
    let mut header = function_header(task_id, deployment);
    header.target_kind = TaskTargetKind::ActorMethod;
    header.target = format!("{SERVICE_ID}:increment");
    header.caller_target = Some(format!("{SERVICE_ID}:increment"));
    header.actor_method = Some(TaskActorMethodTargetFrameMetadata {
        actor_ref: ActorLogicalRefFrameHeader {
            service_id: SERVICE_ID.to_string(),
            actor_type_identity: actor_type_identity(),
            actor_id_type_identity: actor_id_type_identity(),
            actor_id_encoding_version: "skiff-actor-id-encoding-v1".to_string(),
            canonical_actor_id_key_bytes_base64: "YWxpY2U=".to_string(),
            actor_id_hash: actor_id_hash(),
            epoch: 1,
        },
        declaration_owner: declaration_owner(),
        actor_abi_identity: actor_abi(),
        actor_implementation_identity: actor_implementation(),
        method_identity: actor_method(),
        activation: TaskActorActivationSnapshotFrameMetadata {
            key: base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&actor_key_json()).expect("actor key json")),
            create_input: base64::engine::general_purpose::STANDARD.encode(b"[]"),
            expected_type_plan: serde_json::json!({
                "label": "record",
                "node": { "kind": "record", "fields": [] }
            }),
        },
    });
    header
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
    .expect("actor method return")
}

async fn wait_for_actor_frame(
    writer: &Arc<RecordingWriter>,
    mut accept: impl FnMut(&[u8]) -> bool,
) -> Vec<u8> {
    timeout(Duration::from_secs(2), async {
        loop {
            let frames = writer
                .frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(bytes) = frames.into_iter().rev().find(|bytes| accept(bytes)) {
                return bytes;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for actor frame")
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_router::session::InboundFrameSink;
    use skiff_task_control::scheduler::AttemptAdmission;

    #[tokio::test(flavor = "multi_thread")]
    async fn task_production_wire_accept_claim_lease_fence_retry_and_exact_payload() {
        let harness = ProofHarness::new();
        let task_id = "task-proof-function";
        let payload = b"\x00\x01\x02\x03";
        harness
            .submit(function_header(task_id, &harness.deployment), payload)
            .await;

        let records = harness.store.records().await;
        let frames = harness
            .writer
            .frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(error_bytes) = frames.iter().find(|bytes| {
            skiff_runtime_transport::protocol::decode_binary_frame(bytes)
                .ok()
                .and_then(|frame| {
                    frame
                        .header
                        .get("type")
                        .and_then(|value| value.as_str())
                        .map(|value| value == "task.submit.error")
                })
                .unwrap_or(false)
        }) {
            let error_header =
                decode_task_submit_error_frame(error_bytes).expect("decode submit error");
            panic!(
                "task sink rejected proof submission: {:?} {}",
                error_header.error.code, error_header.error.message
            );
        }
        assert_eq!(
            records.len(),
            1,
            "task sink wrote {} outbound frames: {:?}",
            frames.len(),
            frames
                .iter()
                .map(|bytes| {
                    skiff_runtime_transport::protocol::decode_binary_frame(bytes)
                        .ok()
                        .and_then(|frame| {
                            frame
                                .header
                                .get("type")
                                .and_then(|value| value.as_str())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "undecodable".to_string())
                })
                .collect::<Vec<_>>()
        );
        let accepted = &records[0];
        assert_eq!(accepted.task_id.as_str(), task_id);
        assert_eq!(accepted.owner.as_str(), SERVICE_ID);
        assert_eq!(
            accepted.execution.deployment.deployment_artifact_identity,
            harness.deployment.deployment_artifact_identity
        );
        assert_eq!(accepted.payload.as_bytes(), payload);
        assert_eq!(accepted.state, TaskState::Scheduled);
        assert!(matches!(
            accepted.target,
            skiff_task_control::model::DetachedCallTarget::Function { .. }
        ));

        let frames = harness
            .writer
            .frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let response = decode_task_submit_response_frame(
            frames
                .iter()
                .find(|bytes| {
                    skiff_runtime_transport::protocol::decode_binary_frame(bytes)
                        .ok()
                        .and_then(|frame| {
                            frame
                                .header
                                .get("type")
                                .and_then(|value| value.as_str())
                                .map(|value| value == "task.submit.response")
                        })
                        .unwrap_or(false)
                })
                .expect("submit response"),
        )
        .expect("decode submit response");
        assert_eq!(response.task_id, task_id);
        assert_eq!(response.status, "submitted");

        let claimed = harness.claim(task_id).await;
        assert_eq!(claimed.state, TaskState::Leased);
        let lease = claimed.active_lease.as_ref().expect("active lease");
        assert_eq!(lease.owner, "proof-scheduler");
        assert_eq!(claimed.attempt_generation, 1);

        let now = harness.store.now().await.expect("now");
        let renewed = harness
            .store
            .renew(RenewInput {
                task_id: TaskId::new(task_id),
                lease_id: lease.lease_id.clone(),
                new_expiry: now.checked_add_millis(120_000).expect("new expiry"),
            })
            .await
            .expect("renew");
        let renewed = match renewed {
            skiff_task_control::store::RenewOutcome::Renewed(record) => record,
            other => panic!("renew rejected: {other:?}"),
        };
        assert_eq!(
            renewed
                .active_lease
                .as_ref()
                .expect("renewed lease")
                .lease_id,
            lease.lease_id
        );

        harness
            .clock
            .set_millis(u64::try_from(now.millis() + 120_001).expect("proof clock"));
        let recovered = harness
            .store
            .recover_expired_lease(LeaseRecoveryInput {
                task_id: TaskId::new(task_id),
                retry_not_before: DurableUtcTimestamp::from_millis(now.millis() + 65_000),
            })
            .await
            .expect("recover");
        assert!(matches!(
            recovered,
            skiff_task_control::store::LeaseRecoveryOutcome::Recovered(_)
        ));

        let stale_renew = harness
            .store
            .renew(RenewInput {
                task_id: TaskId::new(task_id),
                lease_id: lease.lease_id.clone(),
                new_expiry: DurableUtcTimestamp::from_millis(now.millis() + 180_000),
            })
            .await
            .expect("stale renew query");
        assert!(matches!(
            stale_renew,
            skiff_task_control::store::RenewOutcome::Rejected(
                skiff_task_control::store::RenewRejection::ExpiredLease
                    | skiff_task_control::store::RenewRejection::NotLeased
            )
        ));

        let stale_settle = harness
            .store
            .settle(SettleInput {
                task_id: TaskId::new(task_id),
                lease_id: lease.lease_id.clone(),
                terminal: TaskTerminal {
                    settled_at: DurableUtcTimestamp::from_millis(now.millis() + 60_002),
                    outcome: TaskOutcome::Succeeded,
                },
            })
            .await
            .expect("stale settle query");
        assert!(matches!(
            stale_settle,
            skiff_task_control::store::SettleOutcome::ExpiredLease
                | skiff_task_control::store::SettleOutcome::StaleLease
                | skiff_task_control::store::SettleOutcome::NotLeased
        ));

        let retried = harness.claim(task_id).await;
        assert_eq!(retried.attempt_generation, 2);
        assert_eq!(retried.payload.as_bytes(), payload);
        assert_eq!(
            retried.execution.deployment.deployment_artifact_identity,
            harness.deployment.deployment_artifact_identity
        );

        harness
            .submit(function_header(task_id, &harness.deployment), payload)
            .await;
        assert_eq!(
            harness.store.records().await.len(),
            1,
            "duplicate identical create"
        );
        let mut conflicting = function_header(task_id, &harness.deployment);
        conflicting.rpc_id = "rpc:conflict".to_string();
        harness
            .sink
            .handle_submit(
                harness.session.clone(),
                conflicting,
                b"different-payload".to_vec(),
            )
            .await
            .expect("conflicting submit handled");
        let frames = harness
            .writer
            .frames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let error = frames
            .iter()
            .find(|bytes| {
                skiff_runtime_transport::protocol::decode_binary_frame(bytes)
                    .ok()
                    .and_then(|frame| {
                        frame
                            .header
                            .get("type")
                            .and_then(|value| value.as_str())
                            .map(|value| value == "task.submit.error")
                    })
                    .unwrap_or(false)
            })
            .expect("conflicting submit error");
        let error_header = decode_task_submit_error_frame(error).expect("decode error");
        assert_eq!(error_header.error.code, "rejected");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn actor_production_wire_accept_get_or_activate_invoke_and_terminal() {
        let harness = ProofHarness::new();
        let task_id = "task-proof-actor";
        let payload = b"\x10\x20\x30";
        harness
            .submit(actor_method_header(task_id, &harness.deployment), payload)
            .await;

        let records = harness.store.records().await;
        assert_eq!(records.len(), 1);
        let accepted = &records[0];
        assert_eq!(accepted.payload.as_bytes(), payload);
        let skiff_task_control::model::DetachedCallTarget::ActorMethod {
            actor,
            activation,
            implementation,
            method,
            ..
        } = &accepted.target
        else {
            panic!("actor task target must be ActorMethod");
        };
        assert_eq!(actor.actor_abi_identity, actor_abi());
        assert_eq!(*implementation, actor_implementation());
        assert_eq!(*method, actor_method());
        assert_eq!(
            activation.key.as_bytes(),
            serde_json::to_vec(&actor_key_json()).unwrap()
        );

        let claimed = harness.claim(task_id).await;
        let admission = Arc::clone(&harness.admission);
        let claimed_for_admission = claimed.clone();
        let decision_task =
            tokio::spawn(async move { admission.admit(&claimed_for_admission).await });

        let control_bytes = wait_for_actor_frame(&harness.writer, |bytes| {
            skiff_runtime_transport::actor_owner::decode_actor_owner_control_frame(bytes).is_ok()
        })
        .await;
        let control =
            decode_actor_owner_control_frame(&control_bytes).expect("decode activate control");
        assert_eq!(
            control.operation,
            ActorOwnerControlOperation::ActivateInitial
        );
        assert_eq!(control.target_runtime_id, RUNTIME_ID);
        assert_eq!(
            control.route_authority.build_id,
            harness.deployment.deployment_artifact_identity.to_string()
        );
        assert_eq!(control.fence.actor_abi_identity, actor_abi());
        assert_eq!(
            control.fence.actor_implementation_identity,
            actor_implementation()
        );
        assert!(!control.fence.owner_lease_id.is_empty());

        let ack = encode_actor_owner_control_ack_frame(&ActorOwnerControlAckFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE.to_string(),
            runtime_id: RUNTIME_ID.to_string(),
            request_id: control.request_id.clone(),
            operation: ActorOwnerControlOperation::ActivateInitial,
            accepted: true,
            reason: None,
        })
        .expect("ack");
        harness
            .actor_sink
            .handle(&harness.session, &ack)
            .expect("activation ack handled");

        let decision = decision_task.await.expect("admission task");
        assert_eq!(decision, AdmissionDecision::Accepted);

        let invoke_bytes = wait_for_actor_frame(&harness.writer, |bytes| {
            skiff_runtime_transport::actor_owner::decode_actor_owner_invoke_frame(bytes).is_ok()
        })
        .await;
        let (invoke, invoke_payload) =
            decode_actor_owner_invoke_frame(&invoke_bytes).expect("decode owner invoke");
        assert_eq!(invoke.owner_fence.epoch, control.fence.epoch);
        assert_eq!(
            invoke.owner_fence.owner_lease_id,
            control.fence.owner_lease_id
        );
        assert_eq!(
            invoke.owner_fence.actor_implementation_identity,
            actor_implementation()
        );
        assert_eq!(
            invoke.route_authority.build_id,
            harness.deployment.deployment_artifact_identity.to_string()
        );
        assert_eq!(invoke.invoke.method_identity, actor_method());
        assert_eq!(invoke.invoke.actor_ref.epoch, control.fence.epoch);
        assert_eq!(invoke_payload, payload);
        assert_eq!(invoke.activation_bootstrap, None);

        let fence = harness
            .registry
            .current_owner(&actor_key())
            .expect("production committed owner fence");
        assert_eq!(fence.owner_lease_id, control.fence.owner_lease_id);
        assert_eq!(
            fence.build_id,
            harness.deployment.deployment_artifact_identity.to_string()
        );
        assert_eq!(fence.actor_implementation_identity, actor_implementation());
        assert_eq!(fence.epoch, control.fence.epoch);

        let invocation_id = invoke.invoke.invocation_id.clone();
        harness
            .actor_sink
            .handle(&harness.session, &owner_return_frame(&invocation_id))
            .expect("actor method return handled");
        let terminals = harness
            .terminal_sink
            .terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(terminals, vec![ActorAttemptTerminal::Succeeded]);
    }
}
