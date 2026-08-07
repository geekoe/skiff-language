//! JSON-RPC 2.0 text numeric id lexical corpus (C-model-connection §5.2):
//! 22 frozen cases consumed through the production `JsonRpc20TextProfile`
//! classifier and the production lane (canonical peer keys, canonical
//! terminal frame bytes, platform error frames, close codes).

mod ws_harness;

use std::sync::Arc;

use serde::Deserialize;
use skiff_router::ws::{
    AttachMeta, InboundDispatchResult, JsonRpc20TextProfile, PeerWriter, WebSocketLane,
    WebSocketLaneOptions,
};
use skiff_runtime_transport::connection_protocol::{
    OpaquePeerId, ProfileAction, WebSocketRpcProfile,
};

use ws_harness::{
    FakeDispatchInbound, FakeMethodCatalog, FakePeerWriter, FakeRuntimeResponder,
    FakeRuntimeViolationSink,
};

const ID_CORPUS: &str = include_str!("../../runtime/transport/testdata/client-ws/jsonrpc-ids.json");

#[derive(Debug, Clone, Deserialize)]
struct IdCase {
    name: String,
    frame: String,
    kind: String,
    #[serde(rename = "idKind")]
    id_kind: Option<String>,
    id: Option<String>,
    #[serde(rename = "peerKey")]
    peer_key: Option<String>,
    #[serde(rename = "errorKind")]
    error_kind: Option<String>,
    code: Option<u16>,
}

#[derive(Debug, Clone, Deserialize)]
struct IdCorpus {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    cases: Vec<IdCase>,
}

fn profile() -> JsonRpc20TextProfile {
    JsonRpc20TextProfile::default()
}

fn lane() -> (
    Arc<WebSocketLane>,
    Arc<FakeDispatchInbound>,
    Arc<FakeRuntimeResponder>,
    Arc<FakeRuntimeViolationSink>,
) {
    let dispatch = Arc::new(FakeDispatchInbound::new());
    let responder = Arc::new(FakeRuntimeResponder::new());
    let violations = Arc::new(FakeRuntimeViolationSink::new());
    let lane = WebSocketLane::new(
        WebSocketLaneOptions {
            index: skiff_router::ws::ClientConnectionIndexOptions {
                connection_limit: 8,
                slow_client_budget_bytes: 1024 * 1024,
                high_water_capacity: 8,
            },
            ..Default::default()
        },
        Arc::new(FakeMethodCatalog::new()),
        Arc::new(skiff_router::ws::NoopNotificationObserver),
        violations.clone(),
        dispatch.clone(),
    );
    (lane, dispatch, responder, violations)
}

fn attach_probe(lane: &Arc<WebSocketLane>, id: &str) -> Arc<FakePeerWriter> {
    let fake = Arc::new(FakePeerWriter::new());
    lane.reserve(id).expect("reserve");
    let _ = lane.admit(
        id,
        None,
        None,
        1,
        skiff_router::ws::OverflowPolicy::CloseOldest,
    );
    let writer: Arc<dyn PeerWriter> = fake.clone();
    let _ = lane
        .attach(
            id,
            1,
            "g1".to_string(),
            ws_harness::runtime_session("r1"),
            writer,
            AttachMeta {
                service_id: "example.com/chat".to_string(),
                websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64)),
                profile: WebSocketRpcProfile::JsonRpc2_0Text,
            },
        )
        .expect("attach");
    fake
}

async fn wait_for_finalizers(lane: &Arc<WebSocketLane>) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if lane.snapshot().finalizer_pending == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "finalizer did not drain"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonrpc_id_corpus_classifies_through_production_profile() {
        let corpus: IdCorpus = serde_json::from_str(ID_CORPUS).expect("id corpus must parse");
        assert_eq!(corpus.schema_version, 1);
        assert_eq!(corpus.corpus, "jsonrpc-peer-id-v1");
        assert_eq!(corpus.cases.len(), 22, "frozen id corpus has 22 cases");
        let profile = profile();
        for case in &corpus.cases {
            let action = profile.classify_text(case.frame.as_bytes());
            match case.kind.as_str() {
                "request" => {
                    let ProfileAction::Request { id, method } = action else {
                        panic!("{}: expected request, got {action:?}", case.name);
                    };
                    assert_eq!(method, "status.get", "{}", case.name);
                    match case.id_kind.as_deref() {
                        Some("string") => {
                            assert_eq!(
                                id,
                                OpaquePeerId::String(case.id.clone().expect("id")),
                                "{}",
                                case.name
                            );
                        }
                        Some("safeInteger") => {
                            let OpaquePeerId::SafeInteger(value) = id else {
                                panic!("{}: expected safe integer id", case.name);
                            };
                            assert_eq!(
                                value.to_string(),
                                case.id.clone().expect("id"),
                                "{}",
                                case.name
                            );
                        }
                        other => panic!("{}: unknown idKind {other:?}", case.name),
                    }
                    assert_eq!(
                        id.canonical_key(),
                        case.peer_key.clone().expect("peerKey"),
                        "{}",
                        case.name
                    );
                }
                "notification" => {
                    assert_eq!(
                        action,
                        ProfileAction::Notification {
                            method: "chat.event".to_string()
                        },
                        "{}",
                        case.name
                    );
                }
                "response" => {
                    let ProfileAction::Response { id } = action else {
                        panic!("{}: expected response, got {action:?}", case.name);
                    };
                    assert_eq!(id, case.id.clone().expect("id"), "{}", case.name);
                }
                "platformError" => {
                    let ProfileAction::PlatformError { kind } = action else {
                        panic!("{}: expected platformError, got {action:?}", case.name);
                    };
                    let expected = match case.error_kind.as_deref() {
                        Some("parse") => skiff_router::ws::PlatformErrorKind::Parse,
                        Some("invalidRequest") => {
                            skiff_router::ws::PlatformErrorKind::InvalidRequest
                        }
                        other => panic!("{}: unknown errorKind {other:?}", case.name),
                    };
                    assert_eq!(
                        skiff_router::ws::PlatformErrorKind::from(kind),
                        expected,
                        "{}",
                        case.name
                    );
                }
                "close" => {
                    let ProfileAction::Close { code } = action else {
                        panic!("{}: expected close, got {action:?}", case.name);
                    };
                    assert_eq!(code, case.code.expect("code"), "{}", case.name);
                }
                other => panic!("{}: unknown kind {other}", case.name),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn jsonrpc_id_corpus_roundtrip_through_lane() {
        let corpus: IdCorpus = serde_json::from_str(ID_CORPUS).expect("id corpus must parse");
        for case in &corpus.cases {
            let (lane, dispatch, responder, violations) = lane();
            let writer = attach_probe(&lane, "c1");
            match case.kind.as_str() {
                "request" => {
                    let _ = lane.handle_peer_text("c1", case.frame.as_bytes());
                    let action = dispatch
                        .actions()
                        .pop()
                        .unwrap_or_else(|| panic!("{}: request must dispatch", case.name));
                    assert_eq!(
                        action.peer_id.canonical_key(),
                        case.peer_key.clone().expect("peerKey"),
                        "{}",
                        case.name
                    );
                    let _ = lane.complete_inbound(
                        &action.execution_token,
                        InboundDispatchResult::Success {
                            result: br#"{"ok":1}"#.to_vec(),
                        },
                    );
                    let writes = writer.writes();
                    assert_eq!(writes.len(), 1, "{}: terminal frame written", case.name);
                    let expected_id = case.id.clone().expect("id");
                    let canonical_id = if case.id_kind.as_deref() == Some("string") {
                        format!("\"{expected_id}\"")
                    } else {
                        expected_id
                    };
                    assert!(
                        writes[0].contains(&format!("\"id\":{canonical_id}")),
                        "{}: terminal frame {:#?} must use canonical id",
                        case.name,
                        writes[0]
                    );
                }
                "notification" => {
                    let _ = lane.handle_peer_text("c1", case.frame.as_bytes());
                    assert!(
                        dispatch.actions().is_empty(),
                        "{}: notification must not dispatch",
                        case.name
                    );
                    assert!(writer.writes().is_empty(), "{}: no terminal", case.name);
                }
                "response" => {
                    // String-id response against an active outbound request.
                    let owner = lane.broker.owner_token("c1").expect("owner").0;
                    let source = ws_harness::runtime_session("r1");
                    let request = skiff_router::ws::RuntimeRequest {
                        request_id: "probe-req".to_string(),
                        service_id: "example.com/chat".to_string(),
                        websocket_entry_id: format!(
                            "skiff-websocket-entry-v1:sha256:{}",
                            "b".repeat(64)
                        ),
                        owner_token: owner,
                        profile: WebSocketRpcProfile::JsonRpc2_0Text,
                        method: "chat.send".to_string(),
                        payload: br#"{"n":1}"#.to_vec(),
                        deadline: None,
                    };
                    let responder_arc: Arc<dyn skiff_router::ws::RuntimeResponder> =
                        responder.clone();
                    let source = skiff_router::ws::BrokerRuntimeSource {
                        sender: source,
                        session_token: "session-r1".to_string(),
                        respond: responder_arc,
                    };
                    let outcome = lane.handle_runtime_request("c1", &source, &request);
                    assert_eq!(
                        outcome,
                        skiff_router::ws::RuntimeRequestOutcome::Success,
                        "{}: runtime request violation={:?}",
                        case.name,
                        violations.violations()
                    );
                    // The outbound peer id is `<socketGeneration>:<seq>`; the
                    // classifier-level string-id acceptance is asserted by the
                    // first test. Settlement uses the real generated id.
                    let peer_id = "g1:0";
                    let frame = format!(r#"{{"jsonrpc":"2.0","id":"{peer_id}","result":null}}"#);
                    let _ = lane.handle_peer_text("c1", frame.as_bytes());
                    let responses = responder.responses();
                    assert_eq!(responses.len(), 1, "{}: response settled", case.name);
                    assert_eq!(responses[0].request_id, "probe-req");
                    assert_eq!(
                        responses[0].outcome,
                        skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::Success
                    );
                }
                "platformError" => {
                    let _ = lane.handle_peer_text("c1", case.frame.as_bytes());
                    let writes = writer.writes();
                    assert_eq!(writes.len(), 1, "{}: error frame written", case.name);
                    let (code, message) = match case.error_kind.as_deref() {
                        Some("parse") => (-32700, "Parse error"),
                        _ => (-32600, "Invalid Request"),
                    };
                    assert_eq!(
                        writes[0],
                        format!(
                            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{code},"message":"{message}"}}}}"#
                        ),
                        "{}",
                        case.name
                    );
                }
                "close" => {
                    let _ = lane.handle_peer_text("c1", case.frame.as_bytes());
                    wait_for_finalizers(&lane).await;
                    assert_eq!(
                        lane.index.connection_terminal("c1").as_deref(),
                        Some("ProtocolClose"),
                        "{}",
                        case.name
                    );
                    assert_eq!(
                        writer.close().map(|(code, _)| code),
                        case.code,
                        "{}",
                        case.name
                    );
                }
                _ => {}
            }
        }
    }
}
