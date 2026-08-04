//! W-dispatch corpus verifier: the production `RequestDispatcher` +
//! `RuntimeAdmissionPool` driven through the C-dispatch fake seams, asserting
//! the same observable results as the frozen reference machine
//! (`runtime/transport/tests/dispatch_admission_corpus.rs`).

mod dispatch_harness;

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_router::dispatch::{
    CancelFrame, DispatchSubmit, PendingTerminal, RequestDispatcher, RevalidateOutcome,
    RuntimeDispatcherOptions, RuntimeResponseFrame, SubmitResult,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
    RuntimeAssemblyRequestIngressFrameHeader, RuntimeAssemblyRequestIngressProtocol,
    RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader,
};

use dispatch_harness::{
    build_epoch, FakeCandidateViewSource, FakeEpochSource, FakeLeaseRevalidate, FakeRuntimePeer,
    FakeSessionAbort, SessionState,
};

const REQUIRED_SCENARIOS: [&str; 16] = [
    "unary-completed-releases-permit",
    "unary-response-error-failed",
    "stream-start-chunk-end-completed",
    "stream-protocol-error-terminates-and-cancels",
    "queue-full-fail-closed",
    "request-id-duplicate-fail-closed",
    "no-candidate-fail-closed",
    "revalidate-fail-cancelled-reselect",
    "revalidate-fail-stale-revision-reselect",
    "selection-cursor-round-robin",
    "timeout-terminates-and-cancels",
    "runtime-cancel-no-cancel-frame",
    "client-abort-cancels",
    "runtime-disconnect-terminates-all-pending",
    "replacement-terminates-old-pending",
    "shutdown-terminates-all-pending",
];

fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "unary-completed-releases-permit",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/01-unary-completed-releases-permit.json"
            ),
        ),
        (
            "unary-response-error-failed",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/02-unary-response-error-failed.json"
            ),
        ),
        (
            "stream-start-chunk-end-completed",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/03-stream-start-chunk-end-completed.json"
            ),
        ),
        (
            "stream-protocol-error-terminates-and-cancels",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/04-stream-protocol-error-terminates-and-cancels.json"
            ),
        ),
        (
            "queue-full-fail-closed",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/05-queue-full-fail-closed.json"
            ),
        ),
        (
            "request-id-duplicate-fail-closed",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/06-request-id-duplicate-fail-closed.json"
            ),
        ),
        (
            "no-candidate-fail-closed",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/07-no-candidate-fail-closed.json"
            ),
        ),
        (
            "revalidate-fail-cancelled-reselect",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/08-revalidate-fail-cancelled-reselect.json"
            ),
        ),
        (
            "revalidate-fail-stale-revision-reselect",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/09-revalidate-fail-stale-revision-reselect.json"
            ),
        ),
        (
            "selection-cursor-round-robin",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/10-selection-cursor-round-robin.json"
            ),
        ),
        (
            "timeout-terminates-and-cancels",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/11-timeout-terminates-and-cancels.json"
            ),
        ),
        (
            "runtime-cancel-no-cancel-frame",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/12-runtime-cancel-no-cancel-frame.json"
            ),
        ),
        (
            "client-abort-cancels",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/13-client-abort-cancels.json"
            ),
        ),
        (
            "runtime-disconnect-terminates-all-pending",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/14-runtime-disconnect-terminates-all-pending.json"
            ),
        ),
        (
            "replacement-terminates-old-pending",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/15-replacement-terminates-old-pending.json"
            ),
        ),
        (
            "shutdown-terminates-all-pending",
            include_str!(
                "../../runtime/transport/testdata/dispatch-admission/scenarios/16-shutdown-terminates-all-pending.json"
            ),
        ),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tuple {
    profile: String,
    generation: u64,
    assembly: String,
    #[serde(rename = "configSnapshot")]
    config_snapshot: String,
}

impl Tuple {
    fn to_registered(&self) -> RegisteredAssemblyTuple {
        RegisteredAssemblyTuple {
            profile: self.profile.clone(),
            generation: self.generation,
            assembly: RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(self.assembly.clone()),
            },
            config_snapshot: RuntimeConfigSnapshotRef {
                snapshot_id: RuntimeConfigSnapshotId::parse(self.config_snapshot.clone())
                    .expect("snapshot id"),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Session {
    id: String,
    #[serde(rename = "replicaId")]
    replica_id: String,
    #[serde(rename = "connectionGeneration")]
    connection_generation: u64,
    revision: u64,
    cancelled: bool,
    tuple: Tuple,
    capabilities: Vec<String>,
}

/// Frozen corpus parent facts (reference-machine shape; D2 keeps parsing the
/// shared testdata but no longer resolves task parents in the dispatcher).
#[derive(Debug, Clone, Deserialize)]
struct ActorParent {
    session: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Deployment {
    #[serde(rename = "serviceId")]
    service_id: String,
    #[serde(rename = "contractVersion")]
    contract_version: String,
    #[serde(rename = "deploymentRevision")]
    deployment_revision: String,
    #[serde(rename = "deploymentArtifactIdentity")]
    deployment_artifact_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Epoch {
    profile: String,
    generation: u64,
    #[serde(rename = "assemblyIdentity")]
    assembly_identity: String,
    #[serde(rename = "configSnapshotId")]
    config_snapshot_id: String,
    deployment: Deployment,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Event {
    Request {
        #[serde(rename = "requestId")]
        request_id: String,
        mode: String,
        #[serde(rename = "preferSession")]
        prefer_session: Option<String>,
        #[serde(rename = "revalidateOutcome")]
        revalidate_outcome: Option<String>,
    },
    #[serde(rename = "responseStart")]
    ResponseStart {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "responseChunk")]
    ResponseChunk {
        #[serde(rename = "requestId")]
        request_id: String,
        seq: u64,
    },
    #[serde(rename = "responseEnd")]
    ResponseEnd {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "payloadPresent")]
        payload_present: Option<bool>,
    },
    #[serde(rename = "responseError")]
    ResponseError {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "runtimeCancel")]
    RuntimeCancel {
        #[serde(rename = "requestId")]
        request_id: String,
        reason: Option<String>,
    },
    Timeout {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    #[serde(rename = "clientAbort")]
    ClientAbort {
        #[serde(rename = "requestId")]
        request_id: String,
    },
    Disconnect {
        session: String,
    },
    Replacement {
        #[serde(rename = "oldSession")]
        old_session: String,
        #[serde(rename = "newSession")]
        new_session: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelFrameExpect {
    #[serde(rename = "requestId")]
    request_id: String,
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expect {
    #[serde(rename = "requestOutcomes")]
    request_outcomes: HashMap<String, String>,
    #[serde(rename = "rejectReasons")]
    reject_reasons: HashMap<String, String>,
    #[serde(rename = "terminalSources")]
    terminal_sources: HashMap<String, String>,
    #[serde(rename = "sessionBindings")]
    session_bindings: HashMap<String, String>,
    #[serde(rename = "cancelFrames")]
    cancel_frames: Vec<CancelFrameExpect>,
    #[serde(rename = "permitsHeld")]
    permits_held: usize,
    releases: u64,
    // Frozen reference-machine counters kept for shared-testdata parsing;
    // D2 removed the volatile task path, so they are no longer asserted.
    #[serde(rename = "actorLaneSpawns")]
    #[allow(dead_code)]
    actor_lane_spawns: u64,
    #[serde(rename = "derivedSpawns")]
    #[allow(dead_code)]
    derived_spawns: u64,
    #[serde(rename = "failStop")]
    fail_stop: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    #[serde(rename = "maxConcurrency")]
    max_concurrency: usize,
    epoch: Epoch,
    sessions: Vec<Session>,
    #[serde(rename = "actorInvocationParents")]
    actor_invocation_parents: HashMap<String, ActorParent>,
    events: Vec<Event>,
    expect: Expect,
}

fn scenario_deployment_ref(epoch: &Epoch) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: epoch.deployment.service_id.clone(),
        contract_version: epoch.deployment.contract_version.clone(),
        deployment_revision: DeploymentRevision::new(epoch.deployment.deployment_revision.clone()),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            epoch.deployment.deployment_artifact_identity.clone(),
        ),
    }
}

struct Harness {
    scenario: Scenario,
    dispatcher: RequestDispatcher,
    abort: FakeSessionAbort,
    candidate: FakeCandidateViewSource,
    revalidate: FakeLeaseRevalidate,
    session_epochs: HashMap<String, RuntimeSessionEpoch>,
    session_ids: HashMap<RuntimeSessionEpoch, String>,
    outcomes: HashMap<String, String>,
    reject_reasons: HashMap<String, String>,
    terminal_sources: HashMap<String, String>,
    session_bindings: HashMap<String, String>,
    cancel_frames: Vec<CancelFrame>,
}

impl Harness {
    fn new(scenario: Scenario) -> Self {
        let deployment = scenario_deployment_ref(&scenario.epoch);
        let epoch = build_epoch(
            &scenario.epoch.profile,
            scenario.epoch.generation,
            &scenario.epoch.assembly_identity,
            &scenario.epoch.config_snapshot_id,
            deployment,
        );
        let mut session_epochs = HashMap::new();
        let mut session_ids = HashMap::new();
        let sessions = scenario
            .sessions
            .iter()
            .map(|session| {
                let epoch_identity = RuntimeSessionEpoch {
                    replica_id: session.replica_id.clone(),
                    connection_generation: session.connection_generation,
                };
                session_epochs.insert(session.id.clone(), epoch_identity.clone());
                session_ids.insert(epoch_identity.clone(), session.id.clone());
                SessionState {
                    id: session.id.clone(),
                    epoch: epoch_identity,
                    revision: session.revision,
                    cancelled: session.cancelled,
                    tuple: session.tuple.to_registered(),
                    capabilities: session.capabilities.clone(),
                }
            })
            .collect();
        let candidate = FakeCandidateViewSource::new(sessions);
        let peer = FakeRuntimePeer::new();
        for parent in scenario.actor_invocation_parents.values() {
            assert!(
                session_epochs.contains_key(&parent.session),
                "actor invocation parent session {} must exist",
                parent.session
            );
        }
        let abort = FakeSessionAbort::new();
        let revalidate = FakeLeaseRevalidate::new();
        let options = RuntimeDispatcherOptions::new(
            scenario.max_concurrency,
            Arc::new(FakeEpochSource { epoch: Some(epoch) }),
            Arc::new(candidate.clone()),
            Arc::new(revalidate.clone()),
            Arc::new(peer.clone()),
            Arc::new(abort.clone()),
        )
        .expect("options");
        let dispatcher = RequestDispatcher::new(options).expect("dispatcher");
        Self {
            scenario,
            dispatcher,
            abort,
            candidate,
            revalidate,
            session_epochs,
            session_ids,
            outcomes: HashMap::new(),
            reject_reasons: HashMap::new(),
            terminal_sources: HashMap::new(),
            session_bindings: HashMap::new(),
            cancel_frames: Vec::new(),
        }
    }

    fn session(&self, id: &str) -> RuntimeSessionEpoch {
        self.session_epochs
            .get(id)
            .unwrap_or_else(|| panic!("unknown session {id}"))
            .clone()
    }

    fn session_of(&self, request_id: &str) -> RuntimeSessionEpoch {
        let session_id = self
            .session_bindings
            .get(request_id)
            .unwrap_or_else(|| panic!("request {request_id} is not bound to a session"));
        self.session(session_id)
    }

    fn record_rejected(&mut self, request_id: &str, reason: &str) {
        self.reject_reasons
            .insert(request_id.to_string(), reason.to_string());
        self.outcomes
            .entry(request_id.to_string())
            .or_insert_with(|| "rejected".to_string());
    }

    fn record_terminal(&mut self, terminal: PendingTerminal) {
        self.outcomes.insert(
            terminal.request_id.clone(),
            terminal.outcome.as_str().to_string(),
        );
        self.terminal_sources.insert(
            terminal.request_id.clone(),
            terminal.source.as_str().to_string(),
        );
        if let Some(cancel_frame) = terminal.cancel_frame {
            self.cancel_frames.push(cancel_frame);
        }
    }

    fn request(
        &mut self,
        request_id: &str,
        mode: &str,
        prefer: Option<&str>,
        revalidate: Option<&str>,
    ) {
        if let Some(outcome) = revalidate {
            let outcome = match outcome {
                "ok" => RevalidateOutcome::Ok,
                "fail-cancelled" => RevalidateOutcome::Cancelled,
                "fail-stale-revision" => RevalidateOutcome::StaleRevision,
                other => panic!("unknown revalidate outcome {other}"),
            };
            self.revalidate
                .state
                .lock()
                .unwrap()
                .injected
                .insert(request_id.to_string(), outcome);
        }
        let prefer_epoch = prefer.map(|id| self.session(id));
        let request = self.build_request(request_id, mode, prefer_epoch);
        match self.dispatcher.submit(request) {
            SubmitResult::Accepted {
                request_id,
                session_epoch,
            } => {
                let session_id = self
                    .session_ids
                    .get(&session_epoch)
                    .expect("bound session must exist")
                    .clone();
                self.session_bindings.insert(request_id.clone(), session_id);
            }
            SubmitResult::Rejected { request_id, reason } => {
                self.record_rejected(&request_id, reason.as_str());
            }
        }
    }

    fn build_request(
        &self,
        request_id: &str,
        mode: &str,
        prefer_session: Option<RuntimeSessionEpoch>,
    ) -> DispatchSubmit {
        let epoch = &self.scenario.epoch;
        let deployment = scenario_deployment_ref(epoch);
        DispatchSubmit {
            header: RuntimeAssemblyRequestStartFrameHeader {
                schema_version: "skiff-runtime-frame-v4".to_string(),
                frame_type: "request.start".to_string(),
                request_id: request_id.to_string(),
                mode: mode.to_string(),
                caller: RuntimeAssemblyRequestCallerFrameHeader {
                    kind: "gateway".to_string(),
                },
                routing: RuntimeAssemblyRequestRoutingFrameHeader {
                    kind: "runtimeAssembly".to_string(),
                    assembly_identity: AssemblyIdentity::new(epoch.assembly_identity.clone()),
                    assembly_generation: epoch.generation,
                    deployment,
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
            },
            payload_bytes: Vec::new(),
            prefer_session,
        }
    }
}

impl Harness {
    fn record_outcome(&mut self, outcome: skiff_router::dispatch::FrameOutcome) {
        for terminal in outcome.terminals {
            self.record_terminal(terminal);
        }
    }

    fn dispatch_frame(&mut self, session: &RuntimeSessionEpoch, frame: RuntimeResponseFrame) {
        let outcome = self.dispatcher.on_frame(session, frame);
        self.record_outcome(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_admission_scenarios_match_reference_machine() {
        for (name, json) in scenario_files() {
            let scenario: Scenario = serde_json::from_str(json)
                .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
            assert_eq!(scenario.schema_version, 1, "{name}");
            assert_eq!(scenario.scenario, name, "{name}");
            assert!(
                REQUIRED_SCENARIOS.contains(&name),
                "{name} must be a required scenario"
            );
            assert!(scenario.max_concurrency >= 1, "{name}");
            let mut session_ids = std::collections::HashSet::new();
            for session in &scenario.sessions {
                assert!(
                    session_ids.insert(session.id.as_str()),
                    "{name} duplicate session id {}",
                    session.id
                );
            }

            let mut harness = Harness::new(scenario.clone());
            for event in &scenario.events {
                match event {
                    Event::Request {
                        request_id,
                        mode,
                        prefer_session,
                        revalidate_outcome,
                    } => harness.request(
                        request_id,
                        mode,
                        prefer_session.as_deref(),
                        revalidate_outcome.as_deref(),
                    ),
                    Event::ResponseStart { request_id } => {
                        let session = harness.session_of(request_id);
                        harness.dispatch_frame(
                            &session,
                            RuntimeResponseFrame::Start {
                                request_id: request_id.clone(),
                            },
                        );
                    }
                    Event::ResponseChunk { request_id, seq } => {
                        let session = harness.session_of(request_id);
                        harness.dispatch_frame(
                            &session,
                            RuntimeResponseFrame::Chunk {
                                request_id: request_id.clone(),
                                seq: *seq,
                                payload: Vec::new(),
                            },
                        );
                    }
                    Event::ResponseEnd {
                        request_id,
                        payload_present,
                    } => {
                        let session = harness.session_of(request_id);
                        harness.dispatch_frame(
                            &session,
                            RuntimeResponseFrame::End {
                                request_id: request_id.clone(),
                                payload_present: payload_present.unwrap_or(false),
                                payload: Vec::new(),
                            },
                        );
                    }
                    Event::ResponseError { request_id } => {
                        let session = harness.session_of(request_id);
                        harness.dispatch_frame(&session, RuntimeResponseFrame::Error {
                        request_id: request_id.clone(),
                        error: skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(
                            skiff_runtime_transport::protocol::RuntimeErrorFramePayload {
                                code: "test".to_string(),
                                message: "test".to_string(),
                                status: Some(503),
                                details: None,
                            },
                        ),
                    });
                    }
                    Event::RuntimeCancel { request_id, reason } => {
                        assert!(
                            reason.as_ref().is_some_and(|reason| !reason.is_empty()),
                            "{name} runtimeCancel requires a reason"
                        );
                        let session = harness.session_of(request_id);
                        harness.dispatch_frame(
                            &session,
                            RuntimeResponseFrame::Cancel {
                                request_id: request_id.clone(),
                                reason: reason.clone().expect("reason"),
                            },
                        );
                    }
                    Event::Timeout { request_id } => {
                        if let Some(terminal) = harness.dispatcher.timeout(request_id) {
                            harness.record_terminal(terminal);
                        }
                    }
                    Event::ClientAbort { request_id } => {
                        if let Some(terminal) = harness.dispatcher.caller_abort(request_id, None) {
                            harness.record_terminal(terminal);
                        }
                    }
                    Event::Disconnect { session } => {
                        harness.candidate.mark_cancelled(session);
                        let session_epoch = harness.session(session);
                        for terminal in harness.dispatcher.on_session_closed(&session_epoch) {
                            harness.record_terminal(terminal);
                        }
                    }
                    Event::Replacement {
                        old_session,
                        new_session,
                    } => {
                        harness.candidate.mark_cancelled(old_session);
                        let old_epoch = harness.session(old_session);
                        for terminal in harness.dispatcher.on_session_closed(&old_epoch) {
                            harness.record_terminal(terminal);
                        }
                        let _ = new_session;
                    }
                    Event::Shutdown => {
                        for terminal in harness.dispatcher.shutdown() {
                            harness.record_terminal(terminal);
                        }
                    }
                }
            }

            let actual_cancel_frames: Vec<CancelFrameExpect> = harness
                .cancel_frames
                .iter()
                .map(|frame| CancelFrameExpect {
                    request_id: frame.request_id.clone(),
                    reason: frame.reason.clone(),
                })
                .collect();
            assert_eq!(harness.outcomes, scenario.expect.request_outcomes, "{name}");
            assert_eq!(
                harness.reject_reasons, scenario.expect.reject_reasons,
                "{name} reject reasons"
            );
            assert_eq!(
                harness.terminal_sources, scenario.expect.terminal_sources,
                "{name} terminal sources"
            );
            assert_eq!(
                harness.session_bindings, scenario.expect.session_bindings,
                "{name} session bindings"
            );
            assert_eq!(
                actual_cancel_frames, scenario.expect.cancel_frames,
                "{name} cancel frames"
            );

            let health = harness.dispatcher.health();
            assert_eq!(
                health.admission.permits_held, scenario.expect.permits_held,
                "{name} permits held"
            );
            assert_eq!(
                health.admission.releases, scenario.expect.releases,
                "{name} releases"
            );
            assert!(!scenario.expect.fail_stop, "{name} failStop");
            assert!(
                harness.abort.record.lock().unwrap().sessions.is_empty(),
                "{name} must not abort a session"
            );

            // Pending/permit zero invariants: no scenario leaves a pending or a
            // held per-session permit behind.
            assert_eq!(harness.dispatcher.pending_count(), 0, "{name} pending zero");
            let ledger = harness.dispatcher.permit_ledger();
            assert_eq!(ledger.permits_held, scenario.expect.permits_held, "{name}");
            assert!(
                ledger.per_session.is_empty(),
                "{name} per-session permits must return to zero: {ledger:?}"
            );
        }
    }

    #[test]
    fn scenarios_cover_every_required_dispatch_scenario() {
        let names: std::collections::HashSet<&str> =
            scenario_files().iter().map(|(name, _)| *name).collect();
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.contains(required),
                "required scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }
}
