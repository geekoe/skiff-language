//! HTTP control entry for assembly activation (plan §4.1 step 0; TS parity
//! with `router/src/router/assemblyControlPlane.ts`).
//!
//! The runtime/control listener routes requests on
//! [`ASSEMBLY_ACTIVATION_CONTROL_PATH`] to [`ActivationHttpHandler`]. The
//! handler strictly decodes the frozen `AssemblyActivationRequest`
//! (`skiff_artifact_model` strict Deserialize + validate), starts a live
//! transaction through the coordinator handle, waits for the terminal
//! outcome, and returns the TS-parity JSON response / error classification.
//!
//! This module is the activation lane's only production HTTP surface; it owns
//! no session/dispatcher state and never holds another owner across awaits.

use std::fmt;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{json, Value};
use skiff_artifact_model::AssemblyActivationRequest;
use skiff_deployment::activation_state::CommittedActivation;

use super::coordinator::{ActivationCoordinatorHandle, ActivationPhase, CoordinatorError};
use crate::http::error::HttpError;

/// Canonical control endpoint for whole-assembly activation
/// (`POST /__skiff/activate-assembly`).
pub const ASSEMBLY_ACTIVATION_CONTROL_PATH: &str = "/__skiff/activate-assembly";

/// Body cap for the activation request (TS `readBody` parity: 1 MiB).
pub const ACTIVATION_REQUEST_BODY_CAP: usize = 1024 * 1024;

/// HTTP-level safety deadline. The coordinator's own ACK deadline and the
/// bounded loader deadline normally terminal the transaction first; this is
/// only the outer HTTP guard so a stuck coordinator can never hang a control
/// connection forever.
const DEFAULT_ACTIVATION_HTTP_DEADLINE: Duration = Duration::from_secs(120);

const BODY_TOO_LARGE_MESSAGE: &str = "assembly activation request body exceeds 1 MiB";

pub type ActivationHttpResponse = Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>;

/// Production activation HTTP handler (activation lane).
#[derive(Clone)]
pub struct ActivationHttpHandler {
    coordinator: ActivationCoordinatorHandle,
    deadline: Duration,
}

impl fmt::Debug for ActivationHttpHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActivationHttpHandler")
            .field("deadline", &self.deadline)
            .finish_non_exhaustive()
    }
}

impl ActivationHttpHandler {
    pub fn new(coordinator: ActivationCoordinatorHandle) -> Self {
        Self::with_deadline(coordinator, DEFAULT_ACTIVATION_HTTP_DEADLINE)
    }

    pub fn with_deadline(coordinator: ActivationCoordinatorHandle, deadline: Duration) -> Self {
        Self {
            coordinator,
            deadline,
        }
    }

    /// Routes one control request (listener already matched the canonical
    /// path). Reads the body with the 1 MiB cap, then performs the live
    /// activation and returns the TS-parity JSON outcome.
    pub async fn handle(&self, request: Request<Incoming>) -> ActivationHttpResponse {
        let method = request.method().clone();
        if method != Method::POST {
            return method_not_allowed();
        }
        let body = match read_body_capped(request.into_body(), ACTIVATION_REQUEST_BODY_CAP).await {
            Ok(body) => body,
            Err(message) => {
                return activation_json_error(
                    classify_activation_error(&message),
                    "AssemblyActivationRejected",
                    message,
                )
            }
        };
        self.handle_parts(&method, &body).await
    }

    /// Pure request handling over an already-read body (unit-test seam; the
    /// wire path additionally enforces the cap while reading).
    pub async fn handle_parts(&self, method: &Method, body: &[u8]) -> ActivationHttpResponse {
        if method != &Method::POST {
            return method_not_allowed();
        }
        if body.len() > ACTIVATION_REQUEST_BODY_CAP {
            return activation_json_error(
                classify_activation_error(BODY_TOO_LARGE_MESSAGE),
                "AssemblyActivationRejected",
                BODY_TOO_LARGE_MESSAGE,
            );
        }
        let activation: AssemblyActivationRequest = match serde_json::from_slice(body) {
            Ok(request) => request,
            Err(error) => {
                let message = format!("invalid assembly activation request: {error}");
                return activation_json_error(
                    classify_activation_error(&message),
                    "AssemblyActivationRejected",
                    message,
                );
            }
        };
        if let Err(error) = self.coordinator.start_live(activation.clone()) {
            return activation_error_from_coordinator(error);
        }

        // The coordinator serializes transactions and keeps terminal phases
        // (and the last `tx`/activationId) until the next start. A phase-diff
        // wait alone can never observe a back-to-back commit's own terminal
        // (the phase stays Committed), so the exact activationId match is the
        // back-to-back discriminator; the phase-diff keeps pre-`tx` failures
        // (for example a stale expected generation) settling fast.
        let initial_phase = self.coordinator.phase();
        let terminal_health = tokio::time::timeout(
            self.deadline,
            self.coordinator.wait_until_health(|health| {
                matches!(
                    health.phase,
                    ActivationPhase::Committed
                        | ActivationPhase::Aborted
                        | ActivationPhase::Failed
                        | ActivationPhase::Shutdown
                        | ActivationPhase::Exited
                ) && (health.phase != initial_phase
                    || health.activation_id.as_deref() == Some(activation.activation_id.as_str()))
            }),
        )
        .await;

        match terminal_health {
            Err(_) => activation_json_error(
                504,
                "AssemblyActivationRejected",
                "assembly activation timed out",
            ),
            Ok(health)
                if health.phase == ActivationPhase::Committed
                    && health.activation_id.as_deref()
                        == Some(activation.activation_id.as_str()) =>
            {
                committed_response(&activation)
            }
            Ok(health) => {
                let message = health
                    .last_failure
                    .unwrap_or_else(|| "assembly activation was not committed".to_string());
                let status = classify_activation_error(&message);
                let code = if status == 503 {
                    "AssemblyParticipantsUnavailable"
                } else {
                    "AssemblyActivationRejected"
                };
                activation_json_error(status, code, message)
            }
        }
    }
}

fn committed_response(request: &AssemblyActivationRequest) -> ActivationHttpResponse {
    let committed = CommittedActivation {
        generation: request
            .expected_generation
            .checked_add(1)
            .expect("candidate generation was validated by the coordinator"),
        assembly: request.assembly.clone(),
        config_snapshot: request.config_snapshot.clone(),
    };
    let committed = serde_json::to_value(&committed).expect("committed DTO serializes");
    let body = json!({
        "ok": true,
        "committed": committed,
        "activeAssembly": {
            "environment": request.environment.as_str(),
            "generation": request.expected_generation + 1,
            "assemblyIdentity": request.assembly.assembly_identity.as_str(),
            "configSnapshotId": request.config_snapshot.snapshot_id.to_string(),
        },
        // TS parity shape: the Rust composition does not yet expose a replica
        // snapshot projection, so the control response keeps an empty list.
        "replicas": [],
    });
    json_response(StatusCode::OK, &body)
}

fn method_not_allowed() -> ActivationHttpResponse {
    let body = json!({
        "error": {
            "code": "MethodNotAllowed",
            "message": "assembly activation requires POST",
        }
    });
    json_response_with_headers(StatusCode::METHOD_NOT_ALLOWED, &body, &[("allow", "POST")])
}

fn activation_error_from_coordinator(error: CoordinatorError) -> ActivationHttpResponse {
    match error {
        CoordinatorError::InvalidRequest(message) => activation_json_error(
            classify_activation_error(&message),
            "AssemblyActivationRejected",
            message,
        ),
        CoordinatorError::TransactionInProgress => activation_json_error(
            409,
            "AssemblyActivationRejected",
            "a live or recovery activation transaction is already in progress",
        ),
        CoordinatorError::MailboxFull => activation_json_error(
            503,
            "AssemblyParticipantsUnavailable",
            "activation coordinator mailbox is full",
        ),
        CoordinatorError::Shutdown => activation_json_error(
            503,
            "AssemblyParticipantsUnavailable",
            "activation coordinator is shut down",
        ),
    }
}

fn activation_json_error(
    status: u16,
    code: &str,
    message: impl Into<String>,
) -> ActivationHttpResponse {
    let error = HttpError::platform(status, code, message, None);
    let bytes = serde_json::to_vec(&error.json_body())
        .unwrap_or_else(|_| b"{\"error\":{\"code\":\"AssemblyActivationRejected\",\"message\":\"error body failed to serialize\"}}".to_vec());
    let body: Value = serde_json::from_slice(&bytes).expect("static error body is valid JSON");
    json_response(
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        &body,
    )
}

fn json_response(status: StatusCode, body: &Value) -> ActivationHttpResponse {
    json_response_with_headers(status, body, &[])
}

fn json_response_with_headers(
    status: StatusCode,
    body: &Value,
    headers: &[(&str, &str)],
) -> ActivationHttpResponse {
    let bytes = serde_json::to_vec(body).expect("response body serializes");
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = builder
        .body(
            Full::new(Bytes::from(bytes))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("static JSON response is valid");
    Ok(response)
}

/// TS `classifyActivationError` parity (assemblyControlPlane.ts): substring
/// classification used for the control response status.
fn classify_activation_error(message: &str) -> u16 {
    if message.contains("healthy participant") || message.contains("disconnected") {
        503
    } else if message.contains("timed out") {
        504
    } else if message.contains("invalid") || message.contains("must be") || message.contains("JSON")
    {
        400
    } else {
        409
    }
}

/// Frame-by-frame body read with the activation request cap (same mechanism
/// as `router/src/http/server.rs::read_request_body`; no production pipeline
/// is copied).
async fn read_body_capped(mut body: Incoming, limit: usize) -> Result<Bytes, String> {
    let mut chunks = Vec::new();
    let mut total = 0usize;
    loop {
        let Some(frame) = body.frame().await else {
            break;
        };
        let frame =
            frame.map_err(|_| "assembly activation request body read failed".to_string())?;
        if let Ok(data) = frame.into_data() {
            total = total
                .checked_add(data.len())
                .ok_or_else(|| BODY_TOO_LARGE_MESSAGE.to_string())?;
            if total > limit {
                return Err(BODY_TOO_LARGE_MESSAGE.to_string());
            }
            chunks.push(data);
        }
    }
    Ok(chunks.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use crate::activation::{
        memory::MemoryActivationStateRepository, ActivationCoordinator,
        ActivationCoordinatorOptions, ActivationCoordinatorPorts, ActivationParticipantBinding,
        ActivationRevalidateOutcome, ActivationStateRepository, BlockingLoaderPort,
        CandidateEpochRefs, CandidateLoadError, EnqueueResult, EpochStorePublishPort,
        NoopHealthSink, PublishCommittedEpochPort, RuntimeCandidateQueryPort, SessionEnqueuePort,
    };
    use crate::artifact::ActorRoutingCatalog;
    use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
    use crate::routing::{DispatchCapabilities, RegisteredSessionLease, SessionCancellation};
    use crate::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
    use async_trait::async_trait;
    use skiff_artifact_model::{
        AssemblyActivationControl, AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly,
        RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
        ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };
    use skiff_deployment::activation_state::{
        EnvironmentActivationState, ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION,
    };
    use skiff_deployment::projection::actor_routing::{
        ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    };
    use skiff_deployment::storage::CommittedActivation;
    use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

    use super::*;

    const ASSEMBLY: &str =
        "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SNAPSHOT: &str = "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn assembly_ref(identity: &str) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(identity),
        }
    }

    fn config_ref(id: &str) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(id).expect("snapshot id"),
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

    fn tuple(environment: &str, generation: u64) -> RegisteredAssemblyTuple {
        RegisteredAssemblyTuple {
            environment: environment.to_string(),
            generation,
            assembly: assembly_ref(ASSEMBLY),
            config_snapshot: config_ref(SNAPSHOT),
        }
    }

    fn initial_state(environment: &str, generation: u64) -> EnvironmentActivationState {
        EnvironmentActivationState {
            schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
            environment: environment.to_string(),
            committed: CommittedActivation {
                generation,
                assembly: assembly_ref(ASSEMBLY),
                config_snapshot: config_ref(SNAPSHOT),
            },
            pending: None,
        }
    }

    fn request(
        environment: &str,
        activation_id: &str,
        expected_generation: u64,
    ) -> AssemblyActivationRequest {
        AssemblyActivationRequest {
            schema_version: ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION.to_string(),
            environment: environment.to_string(),
            activation_id: activation_id.to_string(),
            expected_generation,
            assembly: assembly_ref(ASSEMBLY),
            config_snapshot: config_ref(SNAPSHOT),
        }
    }

    fn request_json(environment: &str, activation_id: &str, expected_generation: u64) -> String {
        serde_json::to_string(&request(environment, activation_id, expected_generation))
            .expect("request serializes")
    }

    /// The Runtime replies `Prepared` with the same frozen transaction tuple
    /// (the coordinator rejects a `Prepare` echoed back as an ACK).
    fn prepared_control(prepare: &AssemblyActivationControl) -> AssemblyActivationControl {
        match prepare {
            AssemblyActivationControl::Prepare {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                ..
            } => AssemblyActivationControl::Prepared {
                environment: environment.clone(),
                activation_id: activation_id.clone(),
                expected_generation: *expected_generation,
                candidate_generation: *candidate_generation,
                assembly: assembly.clone(),
                config_snapshot: config_snapshot.clone(),
                replica_id: replica_id.clone(),
            },
            other => panic!("expected Prepare control, got {other:?}"),
        }
    }

    #[derive(Debug)]
    struct OkLoader;

    #[async_trait]
    impl BlockingLoaderPort for OkLoader {
        async fn load_candidate(
            &self,
            refs: &CandidateEpochRefs,
        ) -> Result<Arc<RoutingEpoch>, CandidateLoadError> {
            Ok(epoch(
                &refs.environment,
                refs.generation,
                refs.assembly.clone(),
                refs.config_snapshot.clone(),
            ))
        }
    }

    #[derive(Debug)]
    struct ScriptedCandidates {
        leases: StdMutex<Vec<RegisteredSessionLease>>,
    }

    impl ScriptedCandidates {
        fn new(tuple: RegisteredAssemblyTuple, replica_ids: &[(&str, u64)]) -> Self {
            let leases = replica_ids
                .iter()
                .map(|(replica_id, generation)| RegisteredSessionLease {
                    session_epoch: session(replica_id, *generation),
                    registration_revision: 1,
                    exact_registered_tuple: tuple.clone(),
                    cancellation: SessionCancellation { cancelled: false },
                    capabilities: DispatchCapabilities {
                        unary: true,
                        server_stream: true,
                    },
                })
                .collect();
            Self {
                leases: StdMutex::new(leases),
            }
        }

        fn set_tuple(&self, tuple: RegisteredAssemblyTuple) {
            let mut leases = self.leases.lock().expect("leases lock");
            for lease in leases.iter_mut() {
                lease.exact_registered_tuple = tuple.clone();
            }
        }
    }

    impl RuntimeCandidateQueryPort for ScriptedCandidates {
        fn freeze(
            &self,
            _environment: &str,
        ) -> Result<Vec<RegisteredSessionLease>, crate::activation::ActivationCandidateError>
        {
            Ok(self.leases.lock().expect("leases lock").clone())
        }

        fn revalidate(
            &self,
            _activation_id: &str,
            _frozen: &[ActivationParticipantBinding],
        ) -> ActivationRevalidateOutcome {
            ActivationRevalidateOutcome::Ok
        }
    }

    #[derive(Debug, Default)]
    struct RecordingSessions {
        controls: StdMutex<Vec<(String, AssemblyActivationControl)>>,
    }

    impl SessionEnqueuePort for RecordingSessions {
        fn enqueue_prepare(
            &self,
            _binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("prepare".to_string(), control.clone()));
            EnqueueResult::Ok
        }

        fn enqueue_commit(
            &self,
            _binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("commit".to_string(), control.clone()));
            EnqueueResult::Ok
        }

        fn enqueue_abort(
            &self,
            _binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("abort".to_string(), control.clone()));
            EnqueueResult::Ok
        }

        fn abort_session(&self, _session: &RuntimeSessionEpoch) {}
    }

    struct Harness {
        repo: Arc<MemoryActivationStateRepository>,
        handle: crate::activation::ActivationCoordinatorHandle,
        sessions: Arc<RecordingSessions>,
        candidates: Arc<ScriptedCandidates>,
    }

    async fn harness(environment: &str, committed_generation: u64) -> Harness {
        let repo = Arc::new(MemoryActivationStateRepository::new());
        repo.initialize(&initial_state(environment, committed_generation))
            .await
            .expect("initialize");
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple(environment, committed_generation),
            &[("runtime-a", 1)],
        ));
        let sessions = Arc::new(RecordingSessions::default());
        let ports = ActivationCoordinatorPorts {
            repository: Arc::clone(&repo) as Arc<dyn ActivationStateRepository>,
            loader: Arc::new(OkLoader) as Arc<dyn BlockingLoaderPort>,
            candidates: Arc::clone(&candidates) as Arc<dyn RuntimeCandidateQueryPort>,
            sessions: Arc::clone(&sessions) as Arc<dyn SessionEnqueuePort>,
            publish: Arc::new(EpochStorePublishPort::new(Arc::new(
                ActiveRoutingEpochStore::new(),
            ))) as Arc<dyn PublishCommittedEpochPort>,
            health: Arc::new(NoopHealthSink),
        };
        let handle = ActivationCoordinator::spawn(
            ports,
            ActivationCoordinatorOptions {
                mailbox_capacity: 64,
                ack_deadline: Duration::from_secs(5),
                service_db_mongo_url: None,
            },
        );
        Harness {
            repo,
            handle,
            sessions,
            candidates,
        }
    }

    async fn respond(
        handler: &ActivationHttpHandler,
        method: &Method,
        body: impl Into<Bytes>,
    ) -> (u16, Value) {
        let response = handler
            .handle_parts(method, &body.into())
            .await
            .expect("handler response");
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let value: Value = serde_json::from_slice(&bytes).expect("JSON response body");
        (status, value)
    }

    #[tokio::test]
    async fn rejects_non_post_with_405_and_allow_header() {
        let harness = harness("prod", 7).await;
        let handler = ActivationHttpHandler::new(harness.handle.clone());
        let response = handler
            .handle_parts(&Method::GET, b"{}")
            .await
            .expect("handler response");
        assert_eq!(response.status().as_u16(), 405);
        assert_eq!(
            response
                .headers()
                .get("allow")
                .and_then(|v| v.to_str().ok()),
            Some("POST")
        );
    }

    #[tokio::test]
    async fn rejects_malformed_and_unknown_field_bodies_with_400() {
        let harness = harness("prod", 7).await;
        let handler = ActivationHttpHandler::new(harness.handle.clone());

        let (status, body) = respond(&handler, &Method::POST, "{not json").await;
        assert_eq!(status, 400, "malformed JSON must classify as 400");
        assert_eq!(body["error"]["code"], "AssemblyActivationRejected");

        let mut unknown = request_json("prod", "activation-8", 7);
        unknown = unknown.replace(
            r#""configSnapshot""#,
            r#""unexpectedField":true,"configSnapshot""#,
        );
        let (status, body) = respond(&handler, &Method::POST, unknown).await;
        assert_eq!(status, 400, "unknown field must fail strict decode");
        assert!(body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unknown field")));

        let (status, _) = respond(
            &handler,
            &Method::POST,
            request_json("prod", "activation-8", 7)
                .replace("skiff-assembly-activation-request-v2", "skiff-legacy-v1"),
        )
        .await;
        assert_eq!(status, 400, "schema version mismatch must fail validate");
    }

    #[tokio::test]
    async fn rejects_body_over_cap_with_ts_classification() {
        let harness = harness("prod", 7).await;
        let handler = ActivationHttpHandler::new(harness.handle.clone());
        let oversized = vec![b' '; ACTIVATION_REQUEST_BODY_CAP + 1];
        let (status, body) = respond(&handler, &Method::POST, oversized).await;
        // TS classifyActivationError maps the 1 MiB message to 409.
        assert_eq!(status, 409);
        assert!(body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("1 MiB")));
    }

    #[tokio::test]
    async fn rejects_concurrent_transaction_with_409() {
        let harness = harness("prod", 7).await;
        let handler = ActivationHttpHandler::new(harness.handle.clone());
        // First transaction is enqueued and the actor has left Idle.
        handler
            .coordinator
            .start_live(request("prod", "activation-8", 7))
            .expect("first start");
        handler
            .coordinator
            .wait_for_phase(|phase| phase != ActivationPhase::Idle)
            .await;
        let (status, body) = respond(
            &handler,
            &Method::POST,
            request_json("prod", "activation-8b", 7),
        )
        .await;
        assert_eq!(status, 409);
        assert!(body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("already in progress")));
    }

    #[tokio::test]
    async fn pre_decision_disconnect_durably_aborts_and_response_conflicts() {
        let harness = harness("prod", 7).await;
        let handler = ActivationHttpHandler::new(harness.handle.clone());
        handler
            .coordinator
            .start_live(request("prod", "activation-8", 7))
            .expect("start live");
        handler
            .coordinator
            .wait_for_phase(|phase| phase == ActivationPhase::Prepared)
            .await;
        handler
            .coordinator
            .notify_session_closed(&session("runtime-a", 1))
            .expect("disconnect");
        handler
            .coordinator
            .wait_for_phase(|phase| phase == ActivationPhase::Aborted)
            .await;
        let durable = harness.repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 7);
        assert!(
            durable.pending.is_none(),
            "pre-decision disconnect must abort"
        );

        // A follow-up request with a stale expected generation fails fast with
        // the TS 409 classification (no keyword in the message).
        let (status, body) = respond(
            &handler,
            &Method::POST,
            request_json("prod", "activation-8b", 8),
        )
        .await;
        assert_eq!(status, 409);
        assert!(body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("does not match")));
    }

    #[tokio::test]
    async fn full_live_transaction_returns_committed_json() {
        let harness = harness("prod", 7).await;
        let Harness {
            repo,
            handle,
            sessions,
            ..
        } = harness;
        let handler = ActivationHttpHandler::new(handle.clone());
        let body = request_json("prod", "activation-8", 7);

        // Deliver the Prepared ACK as soon as the transaction is staged.
        let ack_handle = handle.clone();
        let ack_session = session("runtime-a", 1);
        let ack_task = tokio::spawn(async move {
            ack_handle
                .wait_for_phase(|phase| {
                    matches!(
                        phase,
                        ActivationPhase::Prepared
                            | ActivationPhase::Committed
                            | ActivationPhase::Failed
                            | ActivationPhase::Aborted
                    )
                })
                .await;
            let controls = sessions.controls.lock().expect("controls lock").clone();
            let prepare = controls
                .iter()
                .find(|(kind, _)| kind == "prepare")
                .map(|(_, control)| control.clone())
                .expect("prepare control was enqueued");
            ack_handle
                .deliver_ack(&ack_session, prepared_control(&prepare))
                .expect("deliver prepared ack");
        });

        let (status, body) = respond(&handler, &Method::POST, body).await;
        ack_task.await.expect("ack task");

        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["committed"]["generation"], 8);
        assert_eq!(body["committed"]["assembly"]["assemblyIdentity"], ASSEMBLY);
        assert_eq!(body["committed"]["configSnapshot"]["snapshotId"], SNAPSHOT);
        assert_eq!(body["activeAssembly"]["environment"], "prod");
        assert_eq!(body["activeAssembly"]["generation"], 8);
        assert_eq!(body["activeAssembly"]["assemblyIdentity"], ASSEMBLY);
        assert_eq!(body["activeAssembly"]["configSnapshotId"], SNAPSHOT);
        assert_eq!(body["replicas"], serde_json::json!([]));

        let durable = repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 8);
        assert!(durable.pending.is_none());
    }

    #[tokio::test]
    async fn back_to_back_commits_each_settle_the_exact_control_request() {
        let harness = harness("prod", 7).await;
        let Harness {
            repo,
            handle,
            sessions,
            candidates,
        } = harness;
        let handler = ActivationHttpHandler::new(handle.clone());

        let commit_once = |activation_id: String, expected_generation: u64| {
            let handle = handle.clone();
            let sessions = Arc::clone(&sessions);
            let handler = handler.clone();
            async move {
                let ack_session = session("runtime-a", 1);
                let ack_sessions = Arc::clone(&sessions);
                let wait_activation_id = activation_id.clone();
                let ack_task = tokio::spawn(async move {
                    handle
                        .wait_until_health(|health| {
                            health.activation_id.as_deref() == Some(wait_activation_id.as_str())
                                && health.phase == ActivationPhase::Prepared
                        })
                        .await;
                    let controls = ack_sessions.controls.lock().expect("controls lock").clone();
                    let prepare = controls
                        .iter()
                        .rev()
                        .find(|(kind, _)| kind == "prepare")
                        .map(|(_, control)| control.clone())
                        .expect("latest prepare control was enqueued");
                    handle
                        .deliver_ack(&ack_session, prepared_control(&prepare))
                        .expect("deliver prepared ack");
                });
                let body = request_json("prod", &activation_id, expected_generation);
                let (status, value) = respond(&handler, &Method::POST, body).await;
                ack_task.await.expect("ack task");
                assert_eq!(status, 200, "{value}");
                value
            }
        };

        let first = commit_once("activation-8".to_string(), 7).await;
        assert_eq!(first["committed"]["generation"], 8);
        let durable = repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 8);

        // The coordinator keeps the terminal Committed phase after the first
        // commit; a second control request must still settle its own exact
        // transaction (regression: phase-diff wait could never observe it).
        candidates.set_tuple(tuple("prod", 8));
        let second = commit_once("activation-9".to_string(), 8).await;
        assert_eq!(second["committed"]["generation"], 9);
        let durable = repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 9);
        assert!(durable.pending.is_none());
    }

    #[test]
    fn classify_matches_ts_substring_rules() {
        assert_eq!(classify_activation_error("healthy participant"), 503);
        assert_eq!(classify_activation_error("runtime disconnected"), 503);
        assert_eq!(classify_activation_error("activation timed out"), 504);
        assert_eq!(classify_activation_error("invalid request"), 400);
        assert_eq!(
            classify_activation_error("environment must be canonical"),
            400
        );
        assert_eq!(classify_activation_error("strict JSON parse failed"), 400);
        assert_eq!(classify_activation_error("generation mismatch"), 409);
    }

    #[test]
    fn body_cap_constant_matches_wire_limit() {
        assert_eq!(ACTIVATION_REQUEST_BODY_CAP, 1024 * 1024);
    }
}
