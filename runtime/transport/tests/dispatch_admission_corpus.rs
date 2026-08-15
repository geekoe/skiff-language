//! Reference-model corpus verifier for C-dispatch
//! (`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-dispatch-contract.md`):
//! routing epoch capture → candidate query → reserve → revalidate → enqueue
//! → terminal release, pending/terminal semantics, and function-spawn
//! correlation with actor-method spawn routed to the actor lane.
//!
//! TEST-ONLY reference model. Not production code; W-dispatch must implement
//! the frozen semantics and consume the same fixtures.

// This standalone integration-test crate is compiled only as a test target;
// wrapping the whole file in `cfg(test)` would add indentation without scope.
#![allow(clippy::tests_outside_test_module)]

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

const REQUIRED_SCENARIOS: [&str; 19] = [
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
    "function-spawn-derived-pending",
    "actor-method-spawn-actor-lane",
    "spawn-ambiguous-parent-rejected",
];

fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "unary-completed-releases-permit",
            include_str!(
                "../testdata/dispatch-admission/scenarios/01-unary-completed-releases-permit.json"
            ),
        ),
        (
            "unary-response-error-failed",
            include_str!(
                "../testdata/dispatch-admission/scenarios/02-unary-response-error-failed.json"
            ),
        ),
        (
            "stream-start-chunk-end-completed",
            include_str!(
                "../testdata/dispatch-admission/scenarios/03-stream-start-chunk-end-completed.json"
            ),
        ),
        (
            "stream-protocol-error-terminates-and-cancels",
            include_str!(
                "../testdata/dispatch-admission/scenarios/04-stream-protocol-error-terminates-and-cancels.json"
            ),
        ),
        (
            "queue-full-fail-closed",
            include_str!(
                "../testdata/dispatch-admission/scenarios/05-queue-full-fail-closed.json"
            ),
        ),
        (
            "request-id-duplicate-fail-closed",
            include_str!(
                "../testdata/dispatch-admission/scenarios/06-request-id-duplicate-fail-closed.json"
            ),
        ),
        (
            "no-candidate-fail-closed",
            include_str!(
                "../testdata/dispatch-admission/scenarios/07-no-candidate-fail-closed.json"
            ),
        ),
        (
            "revalidate-fail-cancelled-reselect",
            include_str!(
                "../testdata/dispatch-admission/scenarios/08-revalidate-fail-cancelled-reselect.json"
            ),
        ),
        (
            "revalidate-fail-stale-revision-reselect",
            include_str!(
                "../testdata/dispatch-admission/scenarios/09-revalidate-fail-stale-revision-reselect.json"
            ),
        ),
        (
            "selection-cursor-round-robin",
            include_str!(
                "../testdata/dispatch-admission/scenarios/10-selection-cursor-round-robin.json"
            ),
        ),
        (
            "timeout-terminates-and-cancels",
            include_str!(
                "../testdata/dispatch-admission/scenarios/11-timeout-terminates-and-cancels.json"
            ),
        ),
        (
            "runtime-cancel-no-cancel-frame",
            include_str!(
                "../testdata/dispatch-admission/scenarios/12-runtime-cancel-no-cancel-frame.json"
            ),
        ),
        (
            "client-abort-cancels",
            include_str!(
                "../testdata/dispatch-admission/scenarios/13-client-abort-cancels.json"
            ),
        ),
        (
            "runtime-disconnect-terminates-all-pending",
            include_str!(
                "../testdata/dispatch-admission/scenarios/14-runtime-disconnect-terminates-all-pending.json"
            ),
        ),
        (
            "replacement-terminates-old-pending",
            include_str!(
                "../testdata/dispatch-admission/scenarios/15-replacement-terminates-old-pending.json"
            ),
        ),
        (
            "shutdown-terminates-all-pending",
            include_str!(
                "../testdata/dispatch-admission/scenarios/16-shutdown-terminates-all-pending.json"
            ),
        ),
        (
            "function-spawn-derived-pending",
            include_str!(
                "../testdata/dispatch-admission/scenarios/17-function-spawn-derived-pending.json"
            ),
        ),
        (
            "actor-method-spawn-actor-lane",
            include_str!(
                "../testdata/dispatch-admission/scenarios/18-actor-method-spawn-actor-lane.json"
            ),
        ),
        (
            "spawn-ambiguous-parent-rejected",
            include_str!(
                "../testdata/dispatch-admission/scenarios/19-spawn-ambiguous-parent-rejected.json"
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
struct ActorParent {
    session: String,
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
    #[serde(rename = "spawnFunction")]
    SpawnFunction {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "parentRequestId")]
        parent_request_id: String,
        target: Option<String>,
    },
    #[serde(rename = "spawnActorMethod")]
    SpawnActorMethod {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "parentRequestId")]
        parent_request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelFrame {
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
    cancel_frames: Vec<CancelFrame>,
    #[serde(rename = "permitsHeld")]
    permits_held: usize,
    releases: u64,
    #[serde(rename = "actorLaneSpawns")]
    actor_lane_spawns: u64,
    #[serde(rename = "derivedSpawns")]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Unary,
    Stream,
    DerivedSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    WaitingStart,
    Streaming,
}

#[derive(Debug)]
struct Pending {
    kind: PendingKind,
    session_id: String,
    stream_phase: Option<StreamPhase>,
    next_seq: u64,
}

#[derive(Debug)]
struct SessionState {
    id: String,
    cancelled: bool,
    revision: u64,
    tuple: Tuple,
    capabilities: Vec<String>,
    in_flight: usize,
}

struct DispatchMachine {
    max_concurrency: usize,
    epoch: Epoch,
    sessions: Vec<SessionState>,
    actor_parents: HashMap<String, String>,
    pending: BTreeMap<String, Pending>,
    cursor: usize,
    stopped: bool,
    releases: u64,
    cancel_frames: Vec<(String, String)>,
    actor_lane_spawns: u64,
    derived_spawns: u64,
    outcomes: HashMap<String, String>,
    reject_reasons: HashMap<String, String>,
    terminal_sources: HashMap<String, String>,
    session_bindings: HashMap<String, String>,
}

impl DispatchMachine {
    fn new(scenario: &Scenario) -> Self {
        Self {
            max_concurrency: scenario.max_concurrency,
            epoch: scenario.epoch.clone(),
            sessions: scenario
                .sessions
                .iter()
                .map(|session| SessionState {
                    id: session.id.clone(),
                    cancelled: session.cancelled,
                    revision: session.revision,
                    tuple: session.tuple.clone(),
                    capabilities: session.capabilities.clone(),
                    in_flight: 0,
                })
                .collect(),
            actor_parents: scenario
                .actor_invocation_parents
                .iter()
                .map(|(id, parent)| (id.clone(), parent.session.clone()))
                .collect(),
            pending: BTreeMap::new(),
            cursor: 0,
            stopped: false,
            releases: 0,
            cancel_frames: Vec::new(),
            actor_lane_spawns: 0,
            derived_spawns: 0,
            outcomes: HashMap::new(),
            reject_reasons: HashMap::new(),
            terminal_sources: HashMap::new(),
            session_bindings: HashMap::new(),
        }
    }

    fn epoch_tuple(&self) -> Tuple {
        Tuple {
            profile: self.epoch.profile.clone(),
            generation: self.epoch.generation,
            assembly: self.epoch.assembly_identity.clone(),
            config_snapshot: self.epoch.config_snapshot_id.clone(),
        }
    }

    fn candidate_indices(&self, mode: &str) -> Vec<usize> {
        let expected = self.epoch_tuple();
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                !session.cancelled
                    && session.revision == 1
                    && session.tuple == expected
                    && session
                        .capabilities
                        .iter()
                        .any(|capability| capability == mode)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn has_capacity(&self, index: usize) -> bool {
        self.sessions[index].in_flight < self.max_concurrency
    }

    fn index_of_session(&self, id: &str) -> usize {
        self.sessions
            .iter()
            .position(|session| session.id == id)
            .unwrap_or_else(|| panic!("unknown session {id}"))
    }

    fn enqueue(&mut self, request_id: &str, mode: &str, session_index: usize) {
        let kind = match mode {
            "unary" => PendingKind::Unary,
            "serverStream" => PendingKind::Stream,
            other => panic!("unknown mode {other}"),
        };
        let session_id = self.sessions[session_index].id.clone();
        self.pending.insert(
            request_id.to_string(),
            Pending {
                kind,
                stream_phase: (kind == PendingKind::Stream).then_some(StreamPhase::WaitingStart),
                next_seq: 0,
                session_id: session_id.clone(),
            },
        );
        self.session_bindings
            .insert(request_id.to_string(), session_id);
    }

    fn reject(&mut self, request_id: &str, reason: &str) {
        self.reject_reasons
            .insert(request_id.to_string(), reason.to_string());
        self.outcomes
            .insert(request_id.to_string(), "rejected".to_string());
    }

    fn request(
        &mut self,
        request_id: &str,
        mode: &str,
        prefer_session: Option<&str>,
        revalidate_outcome: Option<&str>,
    ) {
        if self.stopped {
            self.reject(request_id, "shutdown");
            return;
        }
        if self.pending.contains_key(request_id) {
            self.reject(request_id, "duplicate");
            return;
        }
        let candidates = self.candidate_indices(mode);
        if candidates.is_empty() {
            self.reject(request_id, "no_candidate");
            return;
        }

        let mut pick = None;
        if let Some(preferred) = prefer_session {
            if let Some(&index) = candidates
                .iter()
                .find(|&&index| self.sessions[index].id == preferred)
            {
                if self.has_capacity(index) {
                    pick = Some(index);
                }
            }
        }
        if pick.is_none() {
            for offset in 0..candidates.len() {
                let index = candidates[(self.cursor + offset) % candidates.len()];
                if self.has_capacity(index) {
                    pick = Some(index);
                    break;
                }
            }
        }
        let Some(selected) = pick else {
            self.reject(request_id, "queue_full");
            return;
        };

        self.sessions[selected].in_flight += 1;
        if matches!(
            revalidate_outcome,
            Some("fail-cancelled" | "fail-stale-revision")
        ) {
            self.sessions[selected].in_flight -= 1;
            self.releases += 1;
            let mut reselect = None;
            for &index in &candidates {
                if index == selected {
                    continue;
                }
                if self.has_capacity(index) {
                    reselect = Some(index);
                    break;
                }
            }
            match reselect {
                Some(index) => {
                    self.sessions[index].in_flight += 1;
                    self.enqueue(request_id, mode, index);
                }
                None => self.reject(request_id, "revalidate_fail_closed"),
            }
            return;
        }

        let position = candidates
            .iter()
            .position(|&index| index == selected)
            .expect("selected must be a candidate");
        self.cursor = (position + 1) % candidates.len();
        self.enqueue(request_id, mode, selected);
    }

    fn terminal(
        &mut self,
        request_id: &str,
        outcome: &str,
        source: &str,
        cancel_reason: Option<&str>,
    ) {
        let Some(pending) = self.pending.remove(request_id) else {
            return;
        };
        self.outcomes
            .insert(request_id.to_string(), outcome.to_string());
        self.terminal_sources
            .insert(request_id.to_string(), source.to_string());
        let session_index = self.index_of_session(&pending.session_id);
        self.sessions[session_index].in_flight -= 1;
        self.releases += 1;
        if let Some(reason) = cancel_reason {
            self.cancel_frames
                .push((request_id.to_string(), reason.to_string()));
        }
    }

    fn protocol_error(&mut self, request_id: &str) {
        self.terminal(
            request_id,
            "protocolError",
            "protocol_error",
            Some("protocol_error"),
        );
    }

    fn response_start(&mut self, request_id: &str) {
        let mut protocol = false;
        if let Some(pending) = self.pending.get_mut(request_id) {
            if pending.kind == PendingKind::Stream
                && pending.stream_phase == Some(StreamPhase::WaitingStart)
            {
                pending.stream_phase = Some(StreamPhase::Streaming);
                return;
            }
            protocol = true;
        }
        if protocol {
            self.protocol_error(request_id);
        }
    }

    fn response_chunk(&mut self, request_id: &str, seq: u64) {
        let mut protocol = false;
        if let Some(pending) = self.pending.get_mut(request_id) {
            if pending.kind == PendingKind::Stream
                && pending.stream_phase == Some(StreamPhase::Streaming)
                && seq == pending.next_seq
            {
                pending.next_seq += 1;
                return;
            }
            protocol = true;
        }
        if protocol {
            self.protocol_error(request_id);
        }
    }

    fn response_end(&mut self, request_id: &str, payload_present: Option<bool>) {
        let mut complete = false;
        let mut protocol = false;
        if let Some(pending) = self.pending.get_mut(request_id) {
            match pending.kind {
                PendingKind::Stream => {
                    if pending.stream_phase == Some(StreamPhase::Streaming)
                        && payload_present == Some(false)
                    {
                        complete = true;
                    } else {
                        protocol = true;
                    }
                }
                PendingKind::DerivedSpawn => {
                    if payload_present == Some(false) {
                        complete = true;
                    } else {
                        protocol = true;
                    }
                }
                PendingKind::Unary => complete = true,
            }
        } else {
            return;
        }
        if protocol {
            self.protocol_error(request_id);
            return;
        }
        if complete {
            self.terminal(request_id, "completed", "runtime_response_end", None);
        }
    }

    fn response_error(&mut self, request_id: &str) {
        self.terminal(request_id, "failed", "runtime_response_error", None);
    }

    fn disconnect_session(&mut self, session_id: &str) {
        let index = self.index_of_session(session_id);
        self.sessions[index].cancelled = true;
        let pending_ids: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for request_id in pending_ids {
            self.terminal(&request_id, "cancelled", "runtime_disconnect", None);
        }
    }

    fn shutdown(&mut self) {
        self.stopped = true;
        let pending_ids: Vec<String> = self.pending.keys().cloned().collect();
        for request_id in pending_ids {
            self.terminal(
                &request_id,
                "cancelled",
                "router_shutdown",
                Some("router_shutdown"),
            );
        }
    }

    fn spawn_function(&mut self, request_id: &str, parent_request_id: &str) {
        if self.stopped {
            self.reject(request_id, "shutdown");
            return;
        }
        let request_parent = self.pending.contains_key(parent_request_id);
        let actor_parent = self.actor_parents.contains_key(parent_request_id);
        if request_parent && actor_parent {
            self.reject(request_id, "ambiguous");
            return;
        }
        if !request_parent && !actor_parent {
            self.reject(request_id, "no_parent");
            return;
        }
        if !request_parent {
            self.reject(request_id, "wrong_parent_kind");
            return;
        }
        if self.pending.contains_key(request_id) {
            self.reject(request_id, "duplicate");
            return;
        }
        let parent_session = self
            .pending
            .get(parent_request_id)
            .expect("request parent exists")
            .session_id
            .clone();
        let session_index = self.index_of_session(&parent_session);
        if self.sessions[session_index].cancelled {
            self.reject(request_id, "parent_terminal");
            return;
        }
        if !self.has_capacity(session_index) {
            self.reject(request_id, "queue_full");
            return;
        }
        self.sessions[session_index].in_flight += 1;
        self.pending.insert(
            request_id.to_string(),
            Pending {
                kind: PendingKind::DerivedSpawn,
                stream_phase: None,
                next_seq: 0,
                session_id: parent_session.clone(),
            },
        );
        self.session_bindings
            .insert(request_id.to_string(), parent_session);
        self.derived_spawns += 1;
    }

    fn spawn_actor_method(&mut self, request_id: &str, parent_request_id: &str) {
        if self.stopped {
            self.reject(request_id, "shutdown");
            return;
        }
        let request_parent = self.pending.contains_key(parent_request_id);
        let actor_parent = self.actor_parents.contains_key(parent_request_id);
        if request_parent && actor_parent {
            self.reject(request_id, "ambiguous");
            return;
        }
        if !request_parent && !actor_parent {
            self.reject(request_id, "no_parent");
            return;
        }
        self.actor_lane_spawns += 1;
    }

    fn permits_held(&self) -> usize {
        self.sessions.iter().map(|session| session.in_flight).sum()
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
            assert!(!scenario.epoch.deployment.service_id.is_empty(), "{name}");
            assert!(
                !scenario.epoch.deployment.contract_version.is_empty()
                    && !scenario.epoch.deployment.deployment_revision.is_empty()
                    && !scenario
                        .epoch
                        .deployment
                        .deployment_artifact_identity
                        .is_empty(),
                "{name} deployment coordinates"
            );
            assert!(scenario.max_concurrency >= 1, "{name}");
            let mut session_ids = HashSet::new();
            for session in &scenario.sessions {
                assert!(!session.replica_id.is_empty(), "{name}");
                assert!(session.connection_generation >= 1, "{name}");
                assert!(session_ids.insert(session.id.as_str()), "{name}");
            }

            let mut machine = DispatchMachine::new(&scenario);
            for event in &scenario.events {
                match event {
                    Event::Request {
                        request_id,
                        mode,
                        prefer_session,
                        revalidate_outcome,
                    } => machine.request(
                        request_id,
                        mode,
                        prefer_session.as_deref(),
                        revalidate_outcome.as_deref(),
                    ),
                    Event::ResponseStart { request_id } => machine.response_start(request_id),
                    Event::ResponseChunk { request_id, seq } => {
                        machine.response_chunk(request_id, *seq)
                    }
                    Event::ResponseEnd {
                        request_id,
                        payload_present,
                    } => machine.response_end(request_id, *payload_present),
                    Event::ResponseError { request_id } => machine.response_error(request_id),
                    Event::RuntimeCancel { request_id, reason } => {
                        assert!(
                            reason.as_ref().is_some_and(|reason| !reason.is_empty()),
                            "{name} runtimeCancel requires a reason"
                        );
                        machine.terminal(request_id, "cancelled", "runtime_request_cancel", None);
                    }
                    Event::Timeout { request_id } => {
                        machine.terminal(request_id, "cancelled", "timeout", Some("timeout"))
                    }
                    Event::ClientAbort { request_id } => machine.terminal(
                        request_id,
                        "cancelled",
                        "caller_abort",
                        Some("caller_cancel"),
                    ),
                    Event::Disconnect { session } => machine.disconnect_session(session),
                    Event::Replacement {
                        old_session,
                        new_session,
                    } => {
                        machine.disconnect_session(old_session);
                        let _ = new_session;
                    }
                    Event::Shutdown => machine.shutdown(),
                    Event::SpawnFunction {
                        request_id,
                        parent_request_id,
                        target,
                    } => {
                        assert!(
                            target.as_ref().is_some_and(|target| !target.is_empty()),
                            "{name} spawnFunction requires a target"
                        );
                        machine.spawn_function(request_id, parent_request_id);
                    }
                    Event::SpawnActorMethod {
                        request_id,
                        parent_request_id,
                    } => machine.spawn_actor_method(request_id, parent_request_id),
                }
            }

            let actual_cancel_frames: Vec<CancelFrame> = machine
                .cancel_frames
                .iter()
                .map(|(request_id, reason)| CancelFrame {
                    request_id: request_id.clone(),
                    reason: reason.clone(),
                })
                .collect();
            assert_eq!(machine.outcomes, scenario.expect.request_outcomes, "{name}");
            assert_eq!(
                machine.reject_reasons, scenario.expect.reject_reasons,
                "{name} reject reasons"
            );
            assert_eq!(
                machine.terminal_sources, scenario.expect.terminal_sources,
                "{name} terminal sources"
            );
            assert_eq!(
                machine.session_bindings, scenario.expect.session_bindings,
                "{name} session bindings"
            );
            assert_eq!(
                actual_cancel_frames, scenario.expect.cancel_frames,
                "{name} cancel frames"
            );
            assert_eq!(
                machine.permits_held(),
                scenario.expect.permits_held,
                "{name} permits held"
            );
            assert_eq!(
                machine.releases, scenario.expect.releases,
                "{name} releases"
            );
            assert_eq!(
                machine.actor_lane_spawns, scenario.expect.actor_lane_spawns,
                "{name} actor lane spawns"
            );
            assert_eq!(
                machine.derived_spawns, scenario.expect.derived_spawns,
                "{name} derived spawns"
            );
            assert!(
                !scenario.expect.fail_stop,
                "{name} reference model has no fail-stop path"
            );
        }
    }

    #[test]
    fn scenarios_cover_every_required_dispatch_rule() {
        let names: HashSet<&str> = scenario_files().iter().map(|(name, _)| *name).collect();
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.contains(required),
                "required scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }
}
