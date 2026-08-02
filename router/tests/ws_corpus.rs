//! W-WebSocket corpus verifier: the production `WebSocketLane`
//! (`ClientConnectionIndex` + `RuntimeGenerationPinLedger` +
//! `WebSocketRequestBroker`) driven through the C-ws fake seams, asserting
//! the same observable results as the frozen reference machine
//! (`runtime/transport/tests/client_ws_corpus.rs`, 23 scenarios).

mod ws_harness;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use skiff_router::ws::{
    AttachMeta, BrokerRuntimeSource, BusinessKey, ClientTerminal, InboundDispatchResult,
    OverflowPolicy, PeerWriter, WebSocketLane, WebSocketLaneOptions,
};
use skiff_runtime_transport::connection_protocol::WebSocketRpcProfile;
use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::websocket_generation_lifecycle::{
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleOperation,
    WebSocketGenerationLifecycleSender,
};

use ws_harness::{
    acquire_control, pin_tuple, runtime_session, FakeDispatchInbound, FakeMethodCatalog,
    FakePeerWriter, FakeRuntimePeer, FakeRuntimeResponder, FakeRuntimeSessionClose,
    FakeRuntimeViolationSink,
};

const REQUIRED_SCENARIOS: [&str; 23] = [
    "01-accept-and-rpc-roundtrip",
    "02-peer-close-terminal",
    "03-business-replacement-close-oldest",
    "04-ranked-replacement-supersedes",
    "05-reject-new-preserves-existing",
    "06-runtime-disconnect-terminal",
    "07-shutdown-drains-finalizers",
    "08-slow-client-saturation",
    "09-captured-writer-stale-write-fence",
    "10-outbound-deadline-terminal",
    "11-broker-tombstone-late-response-isolation",
    "12-four-way-replacement-then-peer-close",
    "13-four-way-peer-close-then-replacement",
    "14-four-way-replacement-then-runtime-disconnect",
    "15-four-way-runtime-disconnect-then-replacement",
    "16-four-way-peer-close-then-shutdown",
    "17-four-way-shutdown-then-peer-close",
    "18-four-way-runtime-disconnect-then-shutdown",
    "19-release-timeout-terminal",
    "20-inbound-deadline-terminal",
    "21-broker-outbound-capacity-resource-limit",
    "22-duplicate-peer-request-id",
    "23-runtime-cancel-outbound",
];

fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("01-accept-and-rpc-roundtrip", include_str!("../../runtime/transport/testdata/client-ws/scenarios/01-accept-and-rpc-roundtrip.json")),
        ("02-peer-close-terminal", include_str!("../../runtime/transport/testdata/client-ws/scenarios/02-peer-close-terminal.json")),
        ("03-business-replacement-close-oldest", include_str!("../../runtime/transport/testdata/client-ws/scenarios/03-business-replacement-close-oldest.json")),
        ("04-ranked-replacement-supersedes", include_str!("../../runtime/transport/testdata/client-ws/scenarios/04-ranked-replacement-supersedes.json")),
        ("05-reject-new-preserves-existing", include_str!("../../runtime/transport/testdata/client-ws/scenarios/05-reject-new-preserves-existing.json")),
        ("06-runtime-disconnect-terminal", include_str!("../../runtime/transport/testdata/client-ws/scenarios/06-runtime-disconnect-terminal.json")),
        ("07-shutdown-drains-finalizers", include_str!("../../runtime/transport/testdata/client-ws/scenarios/07-shutdown-drains-finalizers.json")),
        ("08-slow-client-saturation", include_str!("../../runtime/transport/testdata/client-ws/scenarios/08-slow-client-saturation.json")),
        ("09-captured-writer-stale-write-fence", include_str!("../../runtime/transport/testdata/client-ws/scenarios/09-captured-writer-stale-write-fence.json")),
        ("10-outbound-deadline-terminal", include_str!("../../runtime/transport/testdata/client-ws/scenarios/10-outbound-deadline-terminal.json")),
        ("11-broker-tombstone-late-response-isolation", include_str!("../../runtime/transport/testdata/client-ws/scenarios/11-broker-tombstone-late-response-isolation.json")),
        ("12-four-way-replacement-then-peer-close", include_str!("../../runtime/transport/testdata/client-ws/scenarios/12-four-way-replacement-then-peer-close.json")),
        ("13-four-way-peer-close-then-replacement", include_str!("../../runtime/transport/testdata/client-ws/scenarios/13-four-way-peer-close-then-replacement.json")),
        ("14-four-way-replacement-then-runtime-disconnect", include_str!("../../runtime/transport/testdata/client-ws/scenarios/14-four-way-replacement-then-runtime-disconnect.json")),
        ("15-four-way-runtime-disconnect-then-replacement", include_str!("../../runtime/transport/testdata/client-ws/scenarios/15-four-way-runtime-disconnect-then-replacement.json")),
        ("16-four-way-peer-close-then-shutdown", include_str!("../../runtime/transport/testdata/client-ws/scenarios/16-four-way-peer-close-then-shutdown.json")),
        ("17-four-way-shutdown-then-peer-close", include_str!("../../runtime/transport/testdata/client-ws/scenarios/17-four-way-shutdown-then-peer-close.json")),
        ("18-four-way-runtime-disconnect-then-shutdown", include_str!("../../runtime/transport/testdata/client-ws/scenarios/18-four-way-runtime-disconnect-then-shutdown.json")),
        ("19-release-timeout-terminal", include_str!("../../runtime/transport/testdata/client-ws/scenarios/19-release-timeout-terminal.json")),
        ("20-inbound-deadline-terminal", include_str!("../../runtime/transport/testdata/client-ws/scenarios/20-inbound-deadline-terminal.json")),
        ("21-broker-outbound-capacity-resource-limit", include_str!("../../runtime/transport/testdata/client-ws/scenarios/21-broker-outbound-capacity-resource-limit.json")),
        ("22-duplicate-peer-request-id", include_str!("../../runtime/transport/testdata/client-ws/scenarios/22-duplicate-peer-request-id.json")),
        ("23-runtime-cancel-outbound", include_str!("../../runtime/transport/testdata/client-ws/scenarios/23-runtime-cancel-outbound.json")),
    ]
}

#[derive(Debug, Clone, Deserialize)]
struct LimitsValue {
    #[serde(rename = "connectionLimit", default = "default_connection_limit")]
    connection_limit: usize,
    #[serde(
        rename = "slowClientBudgetBytes",
        default = "default_slow_client_budget"
    )]
    slow_client_budget_bytes: u64,
    #[serde(
        rename = "perGenerationCapacity",
        default = "default_per_generation_capacity"
    )]
    per_generation_capacity: usize,
}

fn default_connection_limit() -> usize {
    5000
}

fn default_slow_client_budget() -> u64 {
    16 * 1024 * 1024
}

fn default_per_generation_capacity() -> usize {
    128
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum EventValue {
    AcceptConnection {
        connection: String,
        #[serde(rename = "businessKey")]
        business_key: Option<String>,
        rank: Option<u64>,
        #[serde(rename = "maxConnections", default = "default_max_connections")]
        max_connections: usize,
        #[serde(default = "default_overflow")]
        overflow: String,
    },
    Attach {
        connection: String,
        #[serde(rename = "socketGeneration")]
        socket_generation: String,
        runtime: String,
    },
    AcquirePin {
        connection: String,
        runtime: String,
    },
    ReleasePin {
        connection: String,
        mode: String,
    },
    PeerClose {
        connection: String,
    },
    RuntimeDisconnect {
        runtime: String,
    },
    Shutdown,
    SlowClient {
        connection: String,
        bytes: u64,
    },
    CapturedWrite {
        connection: String,
        #[serde(rename = "socketGeneration")]
        socket_generation: String,
        bytes: u64,
    },
    RuntimeRequest {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deadlineMs")]
        deadline_ms: Option<u64>,
    },
    Deadline {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    PeerResponse {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerRequest {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    InboundDispatch {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
        result: String,
    },
    RuntimeCancel {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    LateResponse {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
}

fn default_max_connections() -> usize {
    1
}

fn default_overflow() -> String {
    "close-oldest".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectValue {
    terminals: HashMap<String, String>,
    #[serde(rename = "connectionCount", default)]
    connection_count: usize,
    #[serde(rename = "generationCount", default)]
    generation_count: usize,
    #[serde(rename = "outboundPending", default)]
    outbound_pending: usize,
    #[serde(rename = "inboundPending", default)]
    inbound_pending: usize,
    #[serde(default)]
    tombstones: usize,
    #[serde(rename = "pinsAcquired", default)]
    pins_acquired: usize,
    #[serde(rename = "pinsPendingRelease", default)]
    pins_pending_release: usize,
    #[serde(rename = "releaseAcks", default)]
    release_acks: u64,
    #[serde(rename = "finalizerCount", default)]
    finalizer_count: u64,
    #[serde(rename = "runtimeClosed", default)]
    runtime_closed: bool,
    #[serde(rename = "failStop", default)]
    fail_stop: bool,
    #[serde(rename = "openConnections", default)]
    open_connections: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ScenarioFile {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    limits: LimitsValue,
    events: Vec<EventValue>,
    expect: ExpectValue,
}

struct Driver {
    lane: Arc<WebSocketLane>,
    responder: Arc<FakeRuntimeResponder>,
    dispatch: Arc<FakeDispatchInbound>,
    writers: HashMap<String, Arc<dyn PeerWriter>>,
    runtime_by_connection: HashMap<String, String>,
    owner_token_by_connection: HashMap<String, u64>,
    generation_by_connection: HashMap<String, String>,
    expected_pins: std::collections::HashSet<String>,
    acquire_sequence: u64,
}

impl Driver {
    fn new(limits: &LimitsValue) -> Self {
        let runtime_peer = Arc::new(FakeRuntimePeer::new());
        let runtime_close = Arc::new(FakeRuntimeSessionClose::new());
        let responder = Arc::new(FakeRuntimeResponder::new());
        let dispatch = Arc::new(FakeDispatchInbound::new());
        let options = WebSocketLaneOptions {
            index: skiff_router::ws::ClientConnectionIndexOptions {
                connection_limit: limits.connection_limit,
                slow_client_budget_bytes: limits.slow_client_budget_bytes,
                high_water_capacity: limits.connection_limit,
            },
            broker: skiff_router::ws::WebSocketRequestBrokerOptions {
                outbound_per_generation_capacity: limits.per_generation_capacity,
                inbound_per_generation_capacity: limits.per_generation_capacity,
                ..Default::default()
            },
            ..Default::default()
        };
        let lane = WebSocketLane::new(
            options,
            runtime_peer,
            runtime_close,
            Arc::new(skiff_router::ws::AllowAnyPendingAdmission),
            Arc::new(FakeMethodCatalog::new()),
            Arc::new(skiff_router::ws::NoopNotificationObserver),
            Arc::new(FakeRuntimeViolationSink::new()),
            dispatch.clone(),
        );
        Self {
            lane,
            responder,
            dispatch,
            writers: HashMap::new(),
            runtime_by_connection: HashMap::new(),
            owner_token_by_connection: HashMap::new(),
            generation_by_connection: HashMap::new(),
            expected_pins: std::collections::HashSet::new(),
            acquire_sequence: 0,
        }
    }

    fn source(&self, runtime_display: &str) -> BrokerRuntimeSource {
        BrokerRuntimeSource {
            sender: runtime_session(runtime_display),
            session_token: format!("session-{runtime_display}"),
            respond: self.responder.clone(),
        }
    }

    fn run(&mut self, event: &EventValue) {
        match event {
            EventValue::AcceptConnection {
                connection,
                business_key,
                rank,
                max_connections,
                overflow,
            } => {
                self.lane.reserve(connection).expect("reserve");
                let overflow = if overflow == "reject-new" {
                    OverflowPolicy::RejectNew
                } else {
                    OverflowPolicy::CloseOldest
                };
                let _ = self.lane.admit(
                    connection,
                    business_key.as_ref().map(BusinessKey::from_raw),
                    *rank,
                    *max_connections,
                    overflow,
                );
            }
            EventValue::Attach {
                connection,
                socket_generation,
                runtime,
            } => {
                let generation_number = socket_generation
                    .trim_start_matches('g')
                    .parse::<u64>()
                    .unwrap_or(1);
                let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
                let captured = self
                    .lane
                    .attach(
                        connection,
                        generation_number,
                        socket_generation.clone(),
                        runtime_session(runtime),
                        writer,
                        AttachMeta {
                            service_id: "example.com/chat".to_string(),
                            websocket_entry_id: format!(
                                "skiff-websocket-entry-v1:sha256:{}",
                                "b".repeat(64)
                            ),
                            profile: WebSocketRpcProfile::JsonRpc2_0Text,
                        },
                    )
                    .expect("attach");
                self.writers.insert(connection.clone(), captured);
                self.runtime_by_connection
                    .insert(connection.clone(), runtime.clone());
                self.owner_token_by_connection.insert(
                    connection.clone(),
                    self.lane
                        .broker
                        .owner_token(connection)
                        .expect("owner token")
                        .0,
                );
                self.generation_by_connection
                    .insert(connection.clone(), socket_generation.clone());
            }
            EventValue::AcquirePin {
                connection,
                runtime,
            } => {
                let tuple = pin_tuple(connection, runtime);
                if self.expected_pins.insert(connection.clone()) {
                    self.lane
                        .ledger
                        .expect_connection(tuple.clone())
                        .expect("expect");
                }
                self.acquire_sequence += 1;
                let decision = self.lane.ledger.handle_acquire(
                    &runtime_session(runtime),
                    &acquire_control(&format!("acquire-{}", self.acquire_sequence), &tuple),
                );
                assert!(
                    matches!(decision, skiff_router::ws::AcquireDecision::Ack(_)),
                    "acquire must ack for {connection}"
                );
            }
            EventValue::ReleasePin { connection, mode } => match mode.as_str() {
                "initiate" => {
                    let _ = self
                        .lane
                        .ledger
                        .release_connection(connection, true)
                        .expect("release initiate");
                }
                "ack" => {
                    let request_id = self
                        .lane
                        .ledger
                        .pending_release_request_id(connection)
                        .expect("pending release request id");
                    let runtime_display = self
                        .runtime_by_connection
                        .get(connection)
                        .cloned()
                        .unwrap_or_else(|| "r1".to_string());
                    let tuple = pin_tuple(connection, &runtime_display);
                    let ack = WebSocketGenerationLifecycleControl::Ack {
                        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                        frame_type: "websocket.generation.lifecycle".to_string(),
                        operation: WebSocketGenerationLifecycleOperation::Release,
                        request_id,
                        sender: WebSocketGenerationLifecycleSender::Runtime,
                        tuple,
                    };
                    self.lane
                        .ledger
                        .handle_release_response(&runtime_session(&runtime_display), &ack)
                        .expect("release ack");
                }
                "timeout" => {
                    let request_id = self
                        .lane
                        .ledger
                        .pending_release_request_id(connection)
                        .expect("pending release request id");
                    let _ = self.lane.ledger.fire_release_timeout(&request_id);
                }
                other => panic!("unknown release mode {other}"),
            },
            EventValue::PeerClose { connection } => {
                let _ = self.lane.handle_peer_disconnect(connection);
            }
            EventValue::RuntimeDisconnect { runtime } => {
                let _ = self.lane.runtime_disconnected(&runtime_session(runtime));
            }
            EventValue::Shutdown => {
                let _ = self.lane.shutdown();
            }
            EventValue::SlowClient { connection, bytes } => {
                if self.lane.index.slow_client_write(connection, *bytes)
                    == skiff_router::ws::WriteBudget::OverBudget
                {
                    let _ = self
                        .lane
                        .finish(connection, ClientTerminal::SlowClient, None);
                }
            }
            EventValue::CapturedWrite {
                connection,
                socket_generation,
                bytes,
            } => {
                let writer = self
                    .writers
                    .get(connection)
                    .cloned()
                    .expect("captured writer");
                let frame = "x".repeat(*bytes as usize);
                let result = writer.write_text(frame);
                assert!(
                    result.is_err(),
                    "captured writer for {connection} generation {socket_generation} must be fenced"
                );
            }
            EventValue::RuntimeRequest {
                connection,
                request_id,
                deadline_ms,
            } => {
                let runtime_display = self
                    .runtime_by_connection
                    .get(connection)
                    .cloned()
                    .expect("runtime");
                let owner_token = self
                    .owner_token_by_connection
                    .get(connection)
                    .copied()
                    .expect("owner token");
                let request = skiff_router::ws::RuntimeRequest {
                    request_id: request_id.clone(),
                    service_id: "example.com/chat".to_string(),
                    websocket_entry_id: format!(
                        "skiff-websocket-entry-v1:sha256:{}",
                        "b".repeat(64)
                    ),
                    owner_token,
                    profile: WebSocketRpcProfile::JsonRpc2_0Text,
                    method: "chat.send".to_string(),
                    payload: br#"{"n":1}"#.to_vec(),
                    deadline: deadline_ms.map(|timeout_ms| {
                        skiff_runtime_transport::protocol::RuntimeDeadlineFrameHeader {
                            timeout_ms,
                            expires_at: "2026-08-02T00:00:00Z".to_string(),
                        }
                    }),
                };
                let _outcome = self.lane.handle_runtime_request(
                    connection,
                    &self.source(&runtime_display),
                    &request,
                );
            }
            EventValue::Deadline {
                connection,
                request_id,
            } => {
                assert!(
                    self.lane.fire_deadline(connection, request_id),
                    "deadline must hit {connection}/{request_id}"
                );
            }
            EventValue::PeerResponse {
                connection,
                peer_id,
            } => {
                let frame = format!(r#"{{"jsonrpc":"2.0","id":"{peer_id}","result":null}}"#);
                let _ = self.lane.handle_peer_text(connection, frame.as_bytes());
            }
            EventValue::PeerRequest {
                connection,
                peer_id,
            } => {
                let frame = format!(
                    r#"{{"jsonrpc":"2.0","id":"{peer_id}","method":"chat.send","params":{{}}}}"#
                );
                let _ = self.lane.handle_peer_text(connection, frame.as_bytes());
            }
            EventValue::InboundDispatch {
                connection,
                peer_id,
                result,
            } => {
                let action = self
                    .dispatch
                    .actions()
                    .into_iter()
                    .find(|action| {
                        action.connection_id == *connection
                            && action.peer_id.canonical_key() == format!("s:{peer_id}")
                    })
                    .expect("dispatched inbound action");
                let result = match result.as_str() {
                    "success" => InboundDispatchResult::Success {
                        result: br#"{"ok":1}"#.to_vec(),
                    },
                    "invalidParams" => InboundDispatchResult::InvalidParams,
                    "internalError" => InboundDispatchResult::InternalError,
                    "runtimeUnavailable" => InboundDispatchResult::RuntimeUnavailable,
                    "deadlineExceeded" => InboundDispatchResult::DeadlineExceeded,
                    other => panic!("unknown inbound result {other}"),
                };
                let _ = self.lane.complete_inbound(&action.execution_token, result);
            }
            EventValue::RuntimeCancel {
                connection,
                request_id,
            } => {
                let runtime_display = self
                    .runtime_by_connection
                    .get(connection)
                    .cloned()
                    .expect("runtime");
                assert!(
                    self.lane
                        .handle_runtime_cancel(&self.source(&runtime_display), request_id),
                    "runtime cancel must hit {connection}/{request_id}"
                );
            }
            EventValue::LateResponse {
                connection,
                peer_id,
            } => {
                let frame = format!(r#"{{"jsonrpc":"2.0","id":"{peer_id}","result":null}}"#);
                let _ = self.lane.handle_peer_text(connection, frame.as_bytes());
            }
        }
    }
}

async fn wait_for_finalizers(lane: &Arc<WebSocketLane>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if lane.snapshot().finalizer_pending == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "finalizer barrier did not drain"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_corpus_matches_frozen_semantics() {
        let scenarios = scenario_files();
        let names = scenarios.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.contains(&required),
                "required scenario {required} missing"
            );
        }
        for (name, text) in scenarios {
            let scenario: ScenarioFile = serde_json::from_str(text).expect("scenario must parse");
            assert_eq!(scenario.schema_version, 1);
            assert_eq!(scenario.scenario, name);
            let mut driver = Driver::new(&scenario.limits);
            for event in &scenario.events {
                driver.run(event);
            }
            wait_for_finalizers(&driver.lane).await;

            let index_snap = driver.lane.index.snapshot();
            let ledger_snap = driver.lane.ledger.snapshot();
            let broker_snap = driver.lane.broker.snapshot();
            let health = driver.lane.snapshot();

            for (id, expected_terminal) in &scenario.expect.terminals {
                let actual = driver
                    .lane
                    .index
                    .connection_terminal(id)
                    .unwrap_or_else(|| panic!("scenario {name}: unknown connection {id}"));
                assert_eq!(
                    actual, *expected_terminal,
                    "scenario {name}: terminal for {id}"
                );
            }
            assert_eq!(
                health.connection_count, scenario.expect.connection_count,
                "scenario {name}: connectionCount"
            );
            assert_eq!(
                health.open_connections, scenario.expect.open_connections,
                "scenario {name}: openConnections"
            );
            assert_eq!(
                health.generation_count, scenario.expect.generation_count,
                "scenario {name}: generationCount"
            );
            assert_eq!(
                health.outbound_pending, scenario.expect.outbound_pending,
                "scenario {name}: outboundPending"
            );
            assert_eq!(
                health.inbound_pending, scenario.expect.inbound_pending,
                "scenario {name}: inboundPending"
            );
            assert_eq!(
                health.tombstones, scenario.expect.tombstones,
                "scenario {name}: tombstones"
            );
            assert_eq!(
                health.pins_acquired, scenario.expect.pins_acquired,
                "scenario {name}: pinsAcquired"
            );
            assert_eq!(
                health.pins_pending_release, scenario.expect.pins_pending_release,
                "scenario {name}: pinsPendingRelease"
            );
            assert_eq!(
                health.release_acks, scenario.expect.release_acks,
                "scenario {name}: releaseAcks"
            );
            assert_eq!(
                health.finalizer_count, scenario.expect.finalizer_count,
                "scenario {name}: finalizerCount"
            );
            assert_eq!(
                !health.runtime_closed.is_empty(),
                scenario.expect.runtime_closed,
                "scenario {name}: runtimeClosed"
            );
            assert!(
                !scenario.expect.fail_stop && health.fail_stop_reason.is_none(),
                "scenario {name}: failStop must be false for the frozen corpus"
            );
            assert_eq!(
                index_snap.finalizer_pending, 0,
                "scenario {name}: finalizer residue"
            );
            assert_eq!(
                ledger_snap.pins_pending_release, scenario.expect.pins_pending_release,
                "scenario {name}: ledger pending release"
            );
            assert_eq!(
                broker_snap.outbound_pending, health.outbound_pending,
                "scenario {name}: broker outbound index equals pending"
            );
            assert_eq!(
                broker_snap.inbound_pending, health.inbound_pending,
                "scenario {name}: broker inbound index equals pending"
            );
        }
    }
}
