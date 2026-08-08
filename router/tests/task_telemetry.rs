//! E3b focused telemetry tests: task control-plane event emission (submit /
//! cancel / scheduler observation), the router telemetry producer batching
//! protocol, and the backlog gauge shape. All events reuse the
//! `skiff-telemetry-v1` `TelemetryEvent` schema with TaskId correlation.

mod dispatch_harness;
mod health_common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use skiff_artifact_model::PackageCallableId;
use skiff_router::session::demux::InboundFrameSink;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::supervisor::ws::WsSessionWriter;
use skiff_router::task::{
    DurableTaskFrameSink, NoopTaskSubmitParentResolver, RouterTaskSchedulerObservation,
    TaskControlCounters, TaskExecutionImageSource,
};
use skiff_router::telemetry::{
    task_event, RouterTelemetryFileSink, RouterTelemetryProducer, TaskTelemetrySink,
};
use skiff_runtime_transport::protocol::{
    encode_task_cancel_request_frame, encode_task_submit_request_frame,
    ActivationIdentityFrameMetadata, TaskCancelRequestFrameHeader, TaskRef,
    TaskSubmitRequestFrameHeaderV2, TaskSubmitTiming, TaskTargetKind, TelemetryEvent,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_task_control::model::{
    DetachedCallTarget, DurableUtcTimestamp, LeaseId, RecoverablePayload, ServiceOwner,
    TaskExecutionImageRef, TaskId, TaskRecord, TaskState, TaskTraceContext,
};
use skiff_task_control::scheduler::{
    AdmissionDecision, AttemptAdmission, RetryBackoffPolicy, Scheduler, SchedulerConfig,
    SchedulerObservation,
};
use skiff_task_control::store::{ClaimRejection, RenewRejection, TaskStore};
use skiff_task_control::MemoryTaskStore;

const SERVICE_ID: &str = "example.com/service-1";
const TASK_ID: &str = "telemetry-task-1";

// ---------------------------------------------------------------------------
// Recording sink + minimal control-plane fixtures
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct RecordingTelemetry {
    events: Mutex<Vec<TelemetryEvent>>,
}

impl RecordingTelemetry {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn events(&self) -> Vec<TelemetryEvent> {
        self.events.lock().expect("telemetry lock").clone()
    }

    fn names(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| event.name)
            .collect()
    }

    fn task_ids(&self, name: &str) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|event| event.name.as_deref() == Some(name))
            .filter_map(|event| {
                event
                    .attrs
                    .and_then(|attrs| attrs.get("taskId").cloned())
                    .and_then(|value| value.as_str().map(str::to_string))
            })
            .collect()
    }
}

impl TaskTelemetrySink for RecordingTelemetry {
    fn emit(&self, event: TelemetryEvent) -> bool {
        self.events.lock().expect("telemetry lock").push(event);
        true
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

async fn poll_frames(writer: &FakeWriter, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if writer.frames.lock().expect("writer frames").len() >= count {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("writer frame did not arrive");
}

#[derive(Debug, Clone)]
struct FakeImageSource {
    services: Vec<String>,
}

impl FakeImageSource {
    fn known() -> Self {
        Self {
            services: vec![SERVICE_ID.to_string()],
        }
    }

    fn unknown() -> Self {
        Self {
            services: Vec::new(),
        }
    }
}

impl TaskExecutionImageSource for FakeImageSource {
    fn resolve(&self, _header: &TaskSubmitRequestFrameHeaderV2) -> Option<TaskExecutionImageRef> {
        Some(corpus_image())
    }

    fn contains_service(&self, service_id: &str) -> bool {
        self.services.iter().any(|known| known == service_id)
    }

    fn contains_deployment(&self, deployment: &skiff_artifact_model::ServiceDeploymentRef) -> bool {
        self.services
            .iter()
            .any(|known| known == &deployment.service_id)
    }
}

fn corpus_image() -> TaskExecutionImageRef {
    TaskExecutionImageRef {
        target_profile: dispatch_harness::CORPUS_PROFILE.to_string(),
        package_version: dispatch_harness::CORPUS_CONTRACT_VERSION.to_string(),
        deployment: dispatch_harness::corpus_deployment_ref(),
    }
}

fn submit_header(task_id: Option<&str>) -> TaskSubmitRequestFrameHeaderV2 {
    TaskSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.submit.request".to_string(),
        rpc_id: "rpc:submit".to_string(),
        runtime_id: "runtime-a".to_string(),
        caller_kind: skiff_runtime_transport::protocol::TaskCallerKind::Request,
        caller_request_id: "parent-request-1".to_string(),
        target_kind: TaskTargetKind::Function,
        service_id: SERVICE_ID.to_string(),
        service_version: dispatch_harness::CORPUS_CONTRACT_VERSION.to_string(),
        service_protocol_identity: "example.com/service-1:1.0.0".to_string(),
        target: "example.com/service-1:fn".to_string(),
        timing: None,
        task_id: task_id.map(str::to_string),
        build_id: None,
        activation_identity: ActivationIdentityFrameMetadata {
            assembly_identity: dispatch_harness::CORPUS_ASSEMBLY_IDENTITY.to_string(),
            generation: dispatch_harness::CORPUS_GENERATION,
            runtime_replica_id: "runtime-a".to_string(),
            deployment_revision: dispatch_harness::CORPUS_DEPLOYMENT_REVISION.to_string(),
        },
        trace_id: Some("trace-telemetry".to_string()),
        caller_target: None,
        max_queue_wait_ms: None,
        actor_method: None,
    }
}

fn cancel_request(task_ref: &str) -> TaskCancelRequestFrameHeader {
    TaskCancelRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "task.cancel.request".to_string(),
        rpc_id: "rpc:cancel".to_string(),
        task_ref: TaskRef::parse(task_ref).expect("task ref"),
    }
}

fn task_ref(task_id: &str) -> String {
    TaskRef::new(task_id, SERVICE_ID)
        .expect("task ref")
        .into_string()
}

fn session() -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    }
}

#[derive(Debug, Default)]
struct NoopAdmission;

#[async_trait]
impl AttemptAdmission for NoopAdmission {
    async fn admit(&self, _record: &skiff_task_control::model::TaskRecord) -> AdmissionDecision {
        AdmissionDecision::Accepted
    }
}

fn task_record(due_at: DurableUtcTimestamp) -> TaskRecord {
    TaskRecord {
        task_id: TaskId::new(TASK_ID),
        owner: ServiceOwner::new(SERVICE_ID),
        execution: corpus_image(),
        target: DetachedCallTarget::Function {
            callable: PackageCallableId::new("example.com/service-1:fn"),
        },
        payload: RecoverablePayload::new(vec![1, 2, 3]),
        due_at,
        state: TaskState::Ready,
        attempt_generation: 1,
        active_lease: Some(skiff_task_control::model::TaskLease {
            lease_id: LeaseId::new("lease-1"),
            attempt_id: skiff_task_control::model::AttemptId::new("attempt-1"),
            owner: "scheduler-1".to_string(),
            expiry: DurableUtcTimestamp::from_millis(due_at.millis() + 60_000),
        }),
        terminal: None,
        trace: TaskTraceContext {
            trace_id: "trace-1".to_string(),
            span_id: None,
        },
        created_at: DurableUtcTimestamp::from_millis(due_at.millis() - 1_000),
        retry_not_before: None,
        test_case: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_sink_emits_submit_accepted_rejected_and_cancel_events() {
    let telemetry = RecordingTelemetry::new();
    let writer = Arc::new(FakeWriter::default());
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        store.clone(),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let sink = Arc::new(DurableTaskFrameSink::new(
        store,
        scheduler,
        Arc::new(FakeImageSource::known()),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        writer.clone() as Arc<dyn WsSessionWriter>,
        Arc::new(TaskControlCounters::default()),
        telemetry.clone(),
        4096,
    ));

    let header = submit_header(Some(TASK_ID));
    let bytes = encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode submit");
    sink.handle(&session(), &bytes).expect("handle submit");
    poll_frames(&writer, 1).await;
    assert!(
        telemetry
            .names()
            .iter()
            .any(|name| name == "task.submit.accepted"),
        "accepted event missing: {:?}",
        telemetry.names()
    );
    assert_eq!(
        telemetry.task_ids("task.submit.accepted"),
        vec![TASK_ID.to_string()],
        "accepted event must carry the TaskId"
    );
    let accepted = telemetry
        .events()
        .into_iter()
        .find(|event| event.name.as_deref() == Some("task.submit.accepted"))
        .expect("accepted event");
    assert_eq!(
        accepted.request_id.as_deref(),
        Some("parent-request-1"),
        "caller requestId must correlate"
    );

    let cancel = cancel_request(&task_ref(TASK_ID));
    let cancel_bytes = encode_task_cancel_request_frame(&cancel).expect("encode cancel");
    sink.handle(&session(), &cancel_bytes)
        .expect("handle cancel");
    poll_frames(&writer, 2).await;
    assert!(
        telemetry
            .names()
            .iter()
            .any(|name| name == "task.cancel.canceled"),
        "canceled event missing: {:?}",
        telemetry.names()
    );

    // Quota rejection emits task.submit.rejected with the TaskId.
    let quota_writer = Arc::new(FakeWriter::default());
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        store.clone(),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let quota_sink = Arc::new(DurableTaskFrameSink::new(
        store,
        scheduler,
        Arc::new(FakeImageSource::known()),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        quota_writer.clone() as Arc<dyn WsSessionWriter>,
        Arc::new(TaskControlCounters::default()),
        telemetry.clone(),
        2,
    ));
    let rejected_bytes =
        encode_task_submit_request_frame(&header, &[1, 2, 3]).expect("encode rejected submit");
    quota_sink
        .handle(&session(), &rejected_bytes)
        .expect("handle rejected submit");
    poll_frames(&quota_writer, 1).await;
    assert!(
        telemetry
            .names()
            .iter()
            .any(|name| name == "task.submit.rejected"),
        "rejected event missing: {:?}",
        telemetry.names()
    );
    assert_eq!(
        telemetry.task_ids("task.submit.rejected"),
        vec![TASK_ID.to_string()],
        "rejected event must carry the TaskId"
    );
}

#[tokio::test]
async fn task_sink_cancel_unknown_owner_emits_not_found() {
    let telemetry = RecordingTelemetry::new();
    let writer = Arc::new(FakeWriter::default());
    let store = Arc::new(MemoryTaskStore::new()) as Arc<dyn TaskStore>;
    let scheduler = Arc::new(Scheduler::new(
        store.clone(),
        Arc::new(NoopAdmission),
        Arc::new(skiff_task_control::SystemClock),
        SchedulerConfig::default(),
        RetryBackoffPolicy::default(),
    ));
    let sink = Arc::new(DurableTaskFrameSink::new(
        store,
        scheduler,
        Arc::new(FakeImageSource::unknown()),
        Arc::new(NoopTaskSubmitParentResolver)
            as Arc<dyn skiff_router::task::TaskSubmitParentResolver>,
        None,
        writer.clone() as Arc<dyn WsSessionWriter>,
        Arc::new(TaskControlCounters::default()),
        telemetry.clone(),
        4096,
    ));
    let cancel = cancel_request(&task_ref(TASK_ID));
    let bytes = encode_task_cancel_request_frame(&cancel).expect("encode cancel");
    sink.handle(&session(), &bytes).expect("handle cancel");
    poll_frames(&writer, 1).await;
    assert!(
        telemetry
            .names()
            .iter()
            .any(|name| name == "task.cancel.notFound"),
        "notFound event missing: {:?}",
        telemetry.names()
    );
}

#[tokio::test]
async fn scheduler_observation_emits_task_events() {
    let telemetry = RecordingTelemetry::new();
    let observation = RouterTaskSchedulerObservation::new(telemetry.clone());
    let now = DurableUtcTimestamp::from_millis(1_700_000_000_000);
    let record = task_record(DurableUtcTimestamp::from_millis(1_699_999_900_000));

    observation.on_due_ready(&record, now);
    observation.on_claim(&record, now);
    observation.on_claim_duplicate(&record.task_id, &ClaimRejection::AlreadyLeased);
    observation.on_renewed(&record.task_id, &LeaseId::new("lease-1"), now);
    observation.on_renew_lost(
        &record.task_id,
        &LeaseId::new("lease-1"),
        RenewRejection::StaleLease,
    );
    observation.on_recover(&record.task_id, &LeaseId::new("lease-1"));
    observation.on_release(
        &record.task_id,
        &LeaseId::new("lease-1"),
        DurableUtcTimestamp::from_millis(1_700_000_100_000),
    );

    let names = telemetry.names();
    for expected in [
        "task.duplicate.absorbed",
        "task.lease.lost",
        "task.recovered",
        "task.lease.released",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing {expected}: {names:?}"
        );
    }
    for absent in ["task.ready", "task.claim", "task.lease.renewed"] {
        assert!(
            !names.iter().any(|name| name == absent),
            "{absent} must not be emitted (health counters cover trends)"
        );
    }
    let lost = telemetry
        .events()
        .into_iter()
        .find(|event| event.name.as_deref() == Some("task.lease.lost"))
        .expect("lease lost event");
    let attrs = lost.attrs.expect("lease lost attrs");
    assert_eq!(attrs["taskId"], serde_json::json!(TASK_ID));
    assert_eq!(attrs["leaseId"], serde_json::json!("lease-1"));
    assert!(attrs["reason"].is_string());
}

#[test]
fn producer_batches_submit_events() {
    let mut config = health_common::config(std::path::Path::new("."));
    config.telemetry = Some(skiff_router::config::TelemetryConfig {
        enabled: true,
        endpoint: "ws://127.0.0.1:9/telemetry".to_string(),
        protocol: "skiff-telemetry-v1".to_string(),
        queue_max_events: 100,
        batch_max_events: 64,
        batch_max_bytes: 262_144,
        flush_interval_ms: 1_000,
        file_path: None,
        file_max_bytes: None,
        file_max_files: None,
    });
    let producer = RouterTelemetryProducer::new(&config).expect("producer");
    let event = task_event(
        "task.submit.accepted",
        Some(TASK_ID),
        Default::default(),
    );
    for _ in 0..3 {
        assert!(producer.emit(event.clone()));
    }

    let batches = producer.drain_batches();
    assert_eq!(batches.len(), 1, "all events fit one batch");
    let events = batches
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert!(events
        .iter()
        .all(|event| event.name.as_deref() == Some("task.submit.accepted")));
}

#[test]
fn noop_sink_accepts_any_event() {
    use skiff_router::telemetry::NoopTaskTelemetrySink;
    let sink = NoopTaskTelemetrySink;
    assert!(!sink.emit(task_event(
        "task.submit.accepted",
        Some(TASK_ID),
        Default::default(),
    )));
}

// ---------------------------------------------------------------------------
// File sink (default when telemetry.endpoint is empty)
// ---------------------------------------------------------------------------

static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn temp_telemetry_dir(label: &str) -> std::path::PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "skiff-router-telemetry-{label}-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp telemetry dir");
    dir
}

fn remove_temp_telemetry_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

fn read_jsonl(path: &std::path::Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(path).expect("read telemetry jsonl");
    text.lines()
        .map(|line| serde_json::from_str(line).expect("jsonl line must parse"))
        .collect()
}

fn telemetry_config(
    endpoint: &str,
    file_path: Option<std::path::PathBuf>,
    file_max_bytes: Option<u64>,
    file_max_files: Option<u64>,
) -> skiff_router::config::TelemetryConfig {
    skiff_router::config::TelemetryConfig {
        enabled: true,
        endpoint: endpoint.to_string(),
        protocol: "skiff-telemetry-v1".to_string(),
        queue_max_events: 100,
        batch_max_events: 64,
        batch_max_bytes: 262_144,
        flush_interval_ms: 1_000,
        file_path,
        file_max_bytes,
        file_max_files,
    }
}

fn sample_event() -> TelemetryEvent {
    task_event(
        "task.submit.accepted",
        Some(TASK_ID),
        Default::default(),
    )
}

fn assert_file_header(line: &serde_json::Value) {
    let object = line.as_object().expect("header must be an object");
    assert_eq!(object["type"], serde_json::json!("fileHeader"));
    assert_eq!(object["protocol"], serde_json::json!("skiff-telemetry-v1"));
    assert_eq!(object["producerId"], serde_json::json!("router:dev"));
    assert_eq!(object["source"], serde_json::json!("router"));
    assert!(object["createdAt"].as_str().is_some());
}

#[test]
fn file_sink_default_path_writes_header_then_one_event_per_line() {
    let temp = temp_telemetry_dir("default-path");
    let artifacts = temp.join("artifacts");
    let mut config = health_common::config(&artifacts);
    config.telemetry = Some(telemetry_config("", None, None, None));
    let producer = RouterTelemetryProducer::new(&config).expect("producer");
    let sink = RouterTelemetryFileSink::new(producer);
    assert!(sink.producer().emit(sample_event()));
    assert!(sink.producer().emit(sample_event()));
    sink.drain_once_to_file().expect("flush to file");

    // Default path: <artifacts_path.parent()>/logs/telemetry/<producer_id>.jsonl
    let path = temp.join("logs/telemetry/router:dev.jsonl");
    assert!(path.exists(), "default JSONL path missing: {}", path.display());
    let lines = read_jsonl(&path);
    assert_eq!(lines.len(), 3, "header + two events, no batch wrapper");
    assert_file_header(&lines[0]);
    for line in &lines[1..] {
        let object = line.as_object().expect("event must be an object");
        assert!(object.get("envelope_type").is_none(), "no batch envelope");
        assert!(object.get("producerId").is_none(), "no batch envelope");
        assert!(object.get("seq").is_none(), "no batch envelope");
        assert_eq!(object["name"], serde_json::json!("task.submit.accepted"));
        assert_eq!(object["attrs"]["taskId"], serde_json::json!(TASK_ID));
    }
    remove_temp_telemetry_dir(&temp);
}

#[test]
fn file_sink_file_path_override_absolute() {
    let temp = temp_telemetry_dir("override");
    let artifacts = temp.join("artifacts");
    let override_path = temp.join("override").join("custom.jsonl");
    let mut config = health_common::config(&artifacts);
    config.telemetry = Some(telemetry_config(
        "",
        Some(override_path.clone()),
        None,
        None,
    ));
    let producer = RouterTelemetryProducer::new(&config).expect("producer");
    let sink = RouterTelemetryFileSink::new(producer);
    assert!(sink.producer().emit(sample_event()));
    sink.drain_once_to_file().expect("flush to file");

    assert!(override_path.exists(), "override path missing");
    let default_path = temp.join("logs/telemetry/router:dev.jsonl");
    assert!(
        !default_path.exists(),
        "override must replace the default path"
    );
    let lines = read_jsonl(&override_path);
    assert_eq!(lines.len(), 2);
    assert_file_header(&lines[0]);
    remove_temp_telemetry_dir(&temp);
}

#[test]
fn file_sink_file_path_override_relative_to_default_root() {
    let temp = temp_telemetry_dir("relative");
    let artifacts = temp.join("artifacts");
    let mut config = health_common::config(&artifacts);
    config.telemetry = Some(telemetry_config(
        "",
        Some(std::path::PathBuf::from("nested/rel.jsonl")),
        None,
        None,
    ));
    let producer = RouterTelemetryProducer::new(&config).expect("producer");
    let sink = RouterTelemetryFileSink::new(producer);
    assert!(sink.producer().emit(sample_event()));
    sink.drain_once_to_file().expect("flush to file");

    let path = temp.join("logs/telemetry/nested/rel.jsonl");
    assert!(path.exists(), "relative override under default root missing");
    let lines = read_jsonl(&path);
    assert_eq!(lines.len(), 2);
    assert_file_header(&lines[0]);
    remove_temp_telemetry_dir(&temp);
}

#[test]
fn file_sink_rotates_by_size_and_retains_max_files_with_header_rewrite() {
    let temp = temp_telemetry_dir("rotation");
    let artifacts = temp.join("artifacts");
    let mut config = health_common::config(&artifacts);
    config.telemetry = Some(telemetry_config("", None, Some(128), Some(2)));
    let producer = RouterTelemetryProducer::new(&config).expect("producer");
    let sink = RouterTelemetryFileSink::new(producer);
    for _ in 0..3 {
        assert!(sink.producer().emit(sample_event()));
        sink.drain_once_to_file().expect("flush to file");
    }

    let path = temp.join("logs/telemetry/router:dev.jsonl");
    let rotated_1 = temp.join("logs/telemetry/router:dev.jsonl.1");
    let rotated_2 = temp.join("logs/telemetry/router:dev.jsonl.2");
    let rotated_3 = temp.join("logs/telemetry/router:dev.jsonl.3");
    assert!(path.exists());
    assert!(rotated_1.exists(), "first rotation missing");
    assert!(rotated_2.exists(), "second rotation must shift .1 -> .2");
    assert!(
        !rotated_3.exists(),
        "max_files=2 must drop rotations beyond .2"
    );
    // Current file (and every rotated file) starts with a rewritten header.
    assert_file_header(&read_jsonl(&path)[0]);
    assert_file_header(&read_jsonl(&rotated_1)[0]);
    assert_file_header(&read_jsonl(&rotated_2)[0]);
    remove_temp_telemetry_dir(&temp);
}
