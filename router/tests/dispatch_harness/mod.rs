//! Shared fake seams for W-dispatch tests (C-dispatch §7.7).
//!
//! The production dispatcher is driven through the typed ports exactly as the
//! frozen contract defines them; these fakes mirror the reference-machine
//! fixtures used by `runtime/transport/tests/dispatch_admission_corpus.rs`.

// Shared test-fixture module compiled into multiple integration-test crates;
// each binary exercises a different subset of the seams.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{AssemblyIdentity, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef};
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity, ServiceDeploymentRef,
};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::RoutingEpoch;
use skiff_router::dispatch::{
    capabilities_from_wire_names, CandidateViewSource, DispatchSubmit, LeaseRevalidate,
    RequestAuthority, RevalidateOutcome, RoutingEpochSource, RuntimePeer, SessionAbortControl,
    TaskAttemptSubmit,
};
use skiff_router::routing::{CandidateDirectoryView, CandidateSession, RegisteredSessionLease};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
    RuntimeAssemblyRequestIngressFrameHeader, RuntimeAssemblyRequestIngressProtocol,
    RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader, RuntimeAssemblyTaskAttemptFrameHeader,
    RuntimeAssemblyTaskInvocationFrameHeader, RuntimeAssemblyTaskRequestCallerFrameHeader,
    RuntimeAssemblyTaskRequestRoutingFrameHeader, RuntimeAssemblyTaskRequestStartFrameHeader,
};

/// One live session fact in the fake directory view.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub id: String,
    pub epoch: RuntimeSessionEpoch,
    pub revision: u64,
    pub cancelled: bool,
    pub tuple: RegisteredAssemblyTuple,
    pub capabilities: Vec<String>,
}

/// Fixed captured epoch (C-routing-query whole-epoch lease seam).
#[derive(Debug)]
pub struct FakeEpochSource {
    pub epoch: Option<Arc<RoutingEpoch>>,
}

impl RoutingEpochSource for FakeEpochSource {
    fn capture(&self) -> Option<Arc<RoutingEpoch>> {
        self.epoch.clone()
    }
}

/// Fake directory view source over a mutable session-state view.
///
/// Disconnect/replacement events update the view through
/// [`FakeCandidateViewSource::mark_cancelled`], mirroring how the real
/// directory cancels an exact session before the new session becomes current.
/// The canonical [`RuntimeCandidateQuery`] projection then runs over this
/// view, exactly as it will in production.
#[derive(Debug, Clone)]
pub struct FakeCandidateViewSource {
    pub sessions: Arc<Mutex<Vec<SessionState>>>,
}

impl FakeCandidateViewSource {
    pub fn new(sessions: Vec<SessionState>) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(sessions)),
        }
    }

    pub fn mark_cancelled(&self, id: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .iter_mut()
            .find(|session| session.id == id)
            .expect("unknown session");
        session.cancelled = true;
    }
}

impl CandidateViewSource for FakeCandidateViewSource {
    fn view(&self) -> CandidateDirectoryView {
        let sessions = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .map(|session| CandidateSession {
                session_epoch: session.epoch.clone(),
                registered: true,
                registered_tuple: Some(session.tuple.clone()),
                registration_revision: session.revision,
                cancelled: session.cancelled,
                capabilities: capabilities_from_wire_names(&session.capabilities),
            })
            .collect::<Vec<_>>();
        CandidateDirectoryView {
            revision: Some(1),
            sessions,
        }
    }
}

/// Fake atomic revalidation (plan §3.3 step 5).
///
/// Per-request injection covers the corpus `revalidateOutcome` events;
/// un-injected requests pass.
#[derive(Debug, Default)]
pub struct FakeRevalidateState {
    pub injected: HashMap<String, RevalidateOutcome>,
}

#[derive(Debug, Clone)]
pub struct FakeLeaseRevalidate {
    pub state: Arc<Mutex<FakeRevalidateState>>,
}

impl FakeLeaseRevalidate {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeRevalidateState::default())),
        }
    }
}

impl LeaseRevalidate for FakeLeaseRevalidate {
    fn revalidate(&self, request_id: &str, _lease: &RegisteredSessionLease) -> RevalidateOutcome {
        self.state
            .lock()
            .unwrap()
            .injected
            .get(request_id)
            .copied()
            .unwrap_or(RevalidateOutcome::Ok)
    }
}

/// Fake per-session writer (C-dispatch §7.7 `FakeRuntimePeer`).
#[derive(Debug, Default)]
pub struct PeerRecord {
    pub starts: Vec<String>,
    pub cancels: Vec<(String, String)>,
    pub attempts: Vec<String>,
    pub attempt_headers: Vec<RuntimeAssemblyTaskRequestStartFrameHeader>,
    pub fail_start: bool,
    pub fail_cancel: bool,
    pub fail_attempt: bool,
}

#[derive(Debug, Clone)]
pub struct FakeRuntimePeer {
    pub record: Arc<Mutex<PeerRecord>>,
}

impl FakeRuntimePeer {
    pub fn new() -> Self {
        Self {
            record: Arc::new(Mutex::new(PeerRecord::default())),
        }
    }
}

impl RuntimePeer for FakeRuntimePeer {
    fn send_request_start(
        &self,
        _session: &RuntimeSessionEpoch,
        request: &DispatchSubmit,
    ) -> Result<(), String> {
        let mut record = self.record.lock().unwrap();
        if record.fail_start {
            return Err("writer queue full".to_string());
        }
        record.starts.push(request.request_id().to_string());
        Ok(())
    }

    fn send_request_cancel(
        &self,
        _session: &RuntimeSessionEpoch,
        request_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        let mut record = self.record.lock().unwrap();
        if record.fail_cancel {
            return Err("writer queue full".to_string());
        }
        record
            .cancels
            .push((request_id.to_string(), reason.to_string()));
        Ok(())
    }

    fn send_task_attempt_start(
        &self,
        _session: &RuntimeSessionEpoch,
        attempt: &TaskAttemptSubmit,
    ) -> Result<(), String> {
        let mut record = self.record.lock().unwrap();
        if record.fail_attempt {
            return Err("writer queue full".to_string());
        }
        record.attempts.push(attempt.request_id().to_string());
        record.attempt_headers.push(attempt.header.clone());
        Ok(())
    }
}

/// Fake session abort control (C-dispatch §7.4).
#[derive(Debug, Default)]
pub struct AbortRecord {
    pub sessions: Vec<RuntimeSessionEpoch>,
}

#[derive(Debug, Clone)]
pub struct FakeSessionAbort {
    pub record: Arc<Mutex<AbortRecord>>,
}

impl FakeSessionAbort {
    pub fn new() -> Self {
        Self {
            record: Arc::new(Mutex::new(AbortRecord::default())),
        }
    }
}

impl SessionAbortControl for FakeSessionAbort {
    fn abort_session(&self, session: &RuntimeSessionEpoch) {
        self.record.lock().unwrap().sessions.push(session.clone());
    }
}

/// Builds a real immutable `RoutingEpoch` from corpus scenario fields.
///
/// The fixture assembly's identity is replaced with the scenario value so
/// `epoch.registered_tuple()` projects the exact corpus tuple (the same way
/// the frozen routing-query reference derives the expected tuple).
pub fn build_epoch(
    profile: &str,
    generation: u64,
    assembly_identity: &str,
    config_snapshot_id: &str,
    deployment: ServiceDeploymentRef,
) -> Arc<RoutingEpoch> {
    build_epoch_with_actor_methods(
        profile,
        generation,
        assembly_identity,
        config_snapshot_id,
        deployment,
        Vec::new(),
    )
}

/// Builds a real immutable `RoutingEpoch` with an explicit actor routing
/// projection (E2b actor-method task tests).
pub fn build_epoch_with_actor_methods(
    profile: &str,
    generation: u64,
    assembly_identity: &str,
    config_snapshot_id: &str,
    deployment: ServiceDeploymentRef,
    methods: Vec<ActorRoutingMethod>,
) -> Arc<RoutingEpoch> {
    let mut assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    assembly.assembly_identity = AssemblyIdentity::new(assembly_identity);
    assembly.resolved_deployments = vec![deployment];
    let snapshot = Arc::new(
        RuntimeConfigSnapshot::new(
            profile,
            RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(config_snapshot_id)
                    .expect("config snapshot id"),
            },
            Vec::new(),
        )
        .expect("snapshot fixture"),
    );
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(
        ActorRoutingProjection::new(ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(), methods)
            .expect("empty projection"),
    )));
    Arc::new(
        RoutingEpoch::new(profile, generation, Arc::new(assembly), snapshot, catalog)
            .expect("epoch fixture"),
    )
}

pub const CORPUS_PROFILE: &str = "prod";
pub const CORPUS_GENERATION: u64 = 42;
pub const CORPUS_ASSEMBLY_IDENTITY: &str =
    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const CORPUS_CONFIG_SNAPSHOT_ID: &str =
    "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const CORPUS_SERVICE_ID: &str = "example.com/service-1";
pub const CORPUS_CONTRACT_VERSION: &str = "1.0.0";
pub const CORPUS_DEPLOYMENT_REVISION: &str = "deployment-1";
pub const CORPUS_DEPLOYMENT_ARTIFACT_IDENTITY: &str =
    "skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

/// Fixed corpus-shaped epoch for invariant tests.
pub fn corpus_epoch() -> Arc<RoutingEpoch> {
    build_epoch(
        CORPUS_PROFILE,
        CORPUS_GENERATION,
        CORPUS_ASSEMBLY_IDENTITY,
        CORPUS_CONFIG_SNAPSHOT_ID,
        corpus_deployment_ref(),
    )
}

/// Fixed corpus-shaped deployment reference.
pub fn corpus_deployment_ref() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: CORPUS_SERVICE_ID.to_string(),
        contract_version: CORPUS_CONTRACT_VERSION.to_string(),
        deployment_revision: DeploymentRevision::new(CORPUS_DEPLOYMENT_REVISION),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            CORPUS_DEPLOYMENT_ARTIFACT_IDENTITY,
        ),
    }
}

/// Fixed corpus-shaped session fact.
pub fn session_state(id: &str, replica_id: &str, connection_generation: u64) -> SessionState {
    SessionState {
        id: id.to_string(),
        epoch: RuntimeSessionEpoch {
            replica_id: replica_id.to_string(),
            connection_generation,
        },
        revision: 1,
        cancelled: false,
        tuple: RegisteredAssemblyTuple {
            profile: CORPUS_PROFILE.to_string(),
            generation: CORPUS_GENERATION,
            assembly: skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(CORPUS_ASSEMBLY_IDENTITY),
            },
            config_snapshot: RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(CORPUS_CONFIG_SNAPSHOT_ID)
                    .expect("snapshot id"),
            },
        },
        capabilities: vec!["unary".to_string(), "serverStream".to_string()],
    }
}

/// Fixed corpus-shaped `request.start` header.
pub fn request_header(request_id: &str, mode: &str) -> RuntimeAssemblyRequestStartFrameHeader {
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: "skiff-runtime-frame-v4".to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: mode.to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: AssemblyIdentity::new(CORPUS_ASSEMBLY_IDENTITY),
            assembly_generation: CORPUS_GENERATION,
            deployment: ServiceDeploymentRef {
                service_id: CORPUS_SERVICE_ID.to_string(),
                contract_version: CORPUS_CONTRACT_VERSION.to_string(),
                deployment_revision: DeploymentRevision::new(CORPUS_DEPLOYMENT_REVISION),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    CORPUS_DEPLOYMENT_ARTIFACT_IDENTITY,
                ),
            },
            build_id: Some(CORPUS_DEPLOYMENT_ARTIFACT_IDENTITY.to_string()),
            gateway_entry_identity: GatewayEntryIdentity::parse(
                "skiff-gateway-entry-v2:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            )
            .expect("gateway entry identity"),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: "POST".to_string(),
                path: "/".to_string(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: "trace".to_string(),
            span_id: "span".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method: "POST".to_string(),
            url: "http://example.test/".to_string(),
            path: "/".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    }
}

/// Fixed corpus-shaped request (no deadline, no preference).
pub fn request(request_id: &str, mode: &str) -> DispatchSubmit {
    DispatchSubmit {
        header: request_header(request_id, mode),
        payload_bytes: Vec::new(),
        prefer_session: None,
    }
}

/// Fixed corpus-shaped task parent authority.
pub fn authority_for_session(session: &RuntimeSessionEpoch) -> RequestAuthority {
    RequestAuthority {
        assembly_identity: CORPUS_ASSEMBLY_IDENTITY.to_string(),
        assembly_generation: CORPUS_GENERATION,
        deployment: corpus_deployment_ref(),
        session_epoch: session.clone(),
    }
}

/// Fixed corpus-shaped durable task attempt request (function target).
pub fn task_attempt(
    request_id: &str,
    task_id: &str,
    attempt_id: &str,
    lease_id: &str,
) -> TaskAttemptSubmit {
    TaskAttemptSubmit {
        header: RuntimeAssemblyTaskRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: request_id.to_string(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyTaskRequestCallerFrameHeader {
                kind: "service".to_string(),
            },
            routing: RuntimeAssemblyTaskRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: AssemblyIdentity::new(CORPUS_ASSEMBLY_IDENTITY.to_string()),
                assembly_generation: CORPUS_GENERATION,
                deployment: corpus_deployment_ref(),
                build_id: Some(corpus_deployment_ref().deployment_artifact_identity.to_string()),
            },
            invocation: RuntimeAssemblyTaskInvocationFrameHeader {
                kind: "task".to_string(),
                target_kind: "function".to_string(),
                target: "example.com/service-1:fn".to_string(),
            },
            deadline: None,
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: "trace-task".to_string(),
                span_id: "span-task".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            test_effects_enabled: false,
            test_case_capability: None,
            task_attempt: Some(RuntimeAssemblyTaskAttemptFrameHeader {
                task_id: task_id.to_string(),
                attempt_id: attempt_id.to_string(),
                lease_id: lease_id.to_string(),
            }),
        },
        payload: Vec::new(),
        task_id: task_id.to_string(),
        attempt_id: attempt_id.to_string(),
        lease_id: lease_id.to_string(),
        prefer_session: None,
    }
}
