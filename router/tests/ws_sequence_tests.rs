//! Direct sequence tests for the W-WebSocket owners: ledger
//! (`RuntimeGenerationPinLedger`), broker (`WebSocketRequestBroker`) and the
//! index/lane finalizer (four-way races, captured writer fence, slow-client
//! budget, release timeout, protocol/binary closes, shutdown drain).

mod ws_harness;

use std::sync::Arc;
use std::time::Duration;

use skiff_router::ws::{
    AttachMeta, BrokerConnectionGeneration, BrokerRuntimeSource, BusinessKey, ClientTerminal,
    InboundDispatchResult, OverflowPolicy, PeerWriter, RuntimeRequest, WebSocketLane,
    WebSocketLaneOptions, WebSocketRequestBroker, WebSocketRequestBrokerOptions,
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

fn lane() -> Arc<WebSocketLane> {
    WebSocketLane::new(
        WebSocketLaneOptions {
            index: skiff_router::ws::ClientConnectionIndexOptions {
                connection_limit: 8,
                slow_client_budget_bytes: 1024,
                high_water_capacity: 8,
            },
            broker: WebSocketRequestBrokerOptions {
                outbound_per_generation_capacity: 2,
                inbound_per_generation_capacity: 2,
                tombstone_capacity: 16,
                ..Default::default()
            },
            ..Default::default()
        },
        Arc::new(FakeRuntimePeer::new()),
        Arc::new(FakeRuntimeSessionClose::new()),
        Arc::new(skiff_router::ws::AllowAnyPendingAdmission),
        Arc::new(FakeMethodCatalog::new()),
        Arc::new(skiff_router::ws::NoopNotificationObserver),
        Arc::new(FakeRuntimeViolationSink::new()),
        Arc::new(FakeDispatchInbound::new()),
    )
}

fn source(responder: &Arc<FakeRuntimeResponder>, runtime_display: &str) -> BrokerRuntimeSource {
    BrokerRuntimeSource {
        sender: runtime_session(runtime_display),
        session_token: format!("session-{runtime_display}"),
        respond: responder.clone(),
    }
}

fn request(request_id: &str, owner_token: u64, deadline_ms: Option<u64>) -> RuntimeRequest {
    RuntimeRequest {
        request_id: request_id.to_string(),
        service_id: "example.com/chat".to_string(),
        websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64)),
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
    }
}

fn response_frame(peer_id: &str, result: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":"{peer_id}","result":{result}}}"#)
}

fn attach_connection(lane: &Arc<WebSocketLane>, id: &str, display: &str, runtime: &str) {
    let _ = attach_connection_with_writer(lane, id, display, runtime);
}

fn attach_connection_with_writer(
    lane: &Arc<WebSocketLane>,
    id: &str,
    display: &str,
    runtime: &str,
) -> Arc<FakePeerWriter> {
    let fake = Arc::new(FakePeerWriter::new());
    lane.reserve(id).expect("reserve");
    let _ = lane.admit(id, None, None, 1, OverflowPolicy::CloseOldest);
    let writer: Arc<dyn PeerWriter> = fake.clone();
    let generation = display.trim_start_matches('g').parse::<u64>().unwrap_or(1);
    lane.attach(
        id,
        generation,
        display.to_string(),
        runtime_session(runtime),
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if lane.snapshot().finalizer_pending == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "finalizer did not drain"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

// ---------------------------------------------------------------------------
// Broker sequence tests
// ---------------------------------------------------------------------------

fn broker() -> (
    Arc<WebSocketRequestBroker>,
    Arc<FakeRuntimeResponder>,
    Arc<FakeDispatchInbound>,
    Arc<FakeRuntimeViolationSink>,
) {
    let responder = Arc::new(FakeRuntimeResponder::new());
    let dispatch = Arc::new(FakeDispatchInbound::new());
    let violations = Arc::new(FakeRuntimeViolationSink::new());
    let broker = WebSocketRequestBroker::new(
        Arc::new(FakeMethodCatalog::new()),
        Arc::new(skiff_router::ws::NoopNotificationObserver),
        violations.clone(),
        dispatch.clone(),
        WebSocketRequestBrokerOptions {
            outbound_per_generation_capacity: 2,
            inbound_per_generation_capacity: 2,
            tombstone_capacity: 16,
            ..Default::default()
        },
    );
    (Arc::new(broker), responder, dispatch, violations)
}

fn broker_attach(
    broker: &Arc<WebSocketRequestBroker>,
    connection_id: &str,
    display: &str,
    writer: &Arc<dyn PeerWriter>,
) -> u64 {
    broker
        .attach_generation(
            BrokerConnectionGeneration {
                connection_id: connection_id.to_string(),
                socket_generation: display.to_string(),
                service_id: "example.com/chat".to_string(),
                websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64)),
                profile: WebSocketRpcProfile::JsonRpc2_0Text,
            },
            writer.clone(),
            1,
        )
        .expect("broker attach")
        .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_router::ws::{AcquireDecision, RuntimeRequestOutcome, WriteBudget};

    #[test]
    fn broker_outbound_roundtrip_settles_exact_runtime_source() {
        let (broker, responder, _, _) = broker();
        let fake = Arc::new(FakePeerWriter::new());
        let writer: Arc<dyn PeerWriter> = fake.clone();
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::Success
        );
        let writes = fake.writes();
        assert_eq!(writes.len(), 1);
        assert!(writes[0].contains("\"id\":\"g1:0\""));
        assert_eq!(broker.snapshot().outbound_pending, 1);

        assert_eq!(
            broker.handle_peer_text("c1", response_frame("g1:0", "{\"ok\":1}").as_bytes()),
            skiff_router::ws::PeerTextOutcome::Ok
        );
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert_eq!(broker.snapshot().outbound_tombstones, 1);
        let responses = responder.responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].request_id, "req-1");
        assert_eq!(
            responses[0].outcome,
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::Success
        );
        assert_eq!(responses[0].payload, br#"{"ok":1}"#);
    }

    #[test]
    fn broker_out_of_order_responses_and_late_isolation() {
        let (broker, responder, _, _) = broker();
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-2", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:1", "1").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:0", "2").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:1", "late").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert_eq!(broker.snapshot().generation_count, 1);
    }

    #[test]
    fn broker_deadline_wins_exactly_once() {
        let (broker, responder, _, _) = broker();
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, Some(100)),),
            RuntimeRequestOutcome::Success
        );
        assert!(broker.fire_deadline("c1", "req-1"));
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert_eq!(broker.snapshot().outbound_tombstones, 1);
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:0", "1").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert_eq!(
            responder.responses()[0].outcome,
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::DeadlineExceeded
        );
    }

    #[test]
    fn broker_runtime_cancel_detaches_without_peer_write() {
        let (broker, responder, _, _) = broker();
        let fake = Arc::new(FakePeerWriter::new());
        let writer: Arc<dyn PeerWriter> = fake.clone();
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert!(broker.handle_runtime_cancel(&runtime, "req-1"));
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert_eq!(broker.snapshot().outbound_tombstones, 1);
        assert_eq!(fake.writes().len(), 1, "cancel must not write a peer frame");
    }

    #[test]
    fn broker_runtime_disconnect_detaches_only_that_session() {
        let (broker, responder, _, _) = broker();
        let writer_a: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let writer_b: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let owner_a = broker_attach(&broker, "c1", "g1", &writer_a);
        let owner_b = broker_attach(&broker, "c2", "g2", &writer_b);
        let runtime_a = source(&responder, "r1");
        let runtime_b = source(&responder, "r2");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime_a, &request("req-a", owner_a, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request("c2", &runtime_b, &request("req-b", owner_b, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(broker.handle_runtime_disconnect(&runtime_a), 1);
        assert_eq!(broker.snapshot().outbound_pending, 1);
        assert!(broker
            .handle_peer_text("c2", response_frame("g2:0", "1").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert_eq!(broker.snapshot().outbound_pending, 0);
    }

    #[test]
    fn broker_capacity_and_duplicate_runtime_key_fail_closed() {
        let (broker, responder, _, violations) = broker();
        let fake = Arc::new(FakePeerWriter::new());
        let writer: Arc<dyn PeerWriter> = fake.clone();
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-2", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-3", owner, None)),
            RuntimeRequestOutcome::ResourceLimit
        );
        assert_eq!(fake.writes().len(), 2, "capacity rejection must not write");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::ProtocolError
        );
        assert_eq!(violations.violations().len(), 1);
    }

    #[test]
    fn broker_duplicate_inbound_id_and_unknown_response_close() {
        let (broker_a, _responder, dispatch, _) = broker();
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        broker_attach(&broker_a, "c1", "g1", &writer);
        let frame = r#"{"jsonrpc":"2.0","id":"p1","method":"chat.send","params":{}}"#;
        assert!(broker_a
            .handle_peer_text("c1", frame.as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        let action = dispatch.actions().pop().expect("dispatch action");
        assert!(broker_a
            .complete_inbound(
                &action.execution_token,
                InboundDispatchResult::Success {
                    result: br#"{"ok":1}"#.to_vec(),
                },
            )
            .eq(&skiff_router::ws::InboundCompletionOutcome::Completed));
        assert_eq!(broker_a.snapshot().inbound_tombstones, 1);
        assert!(matches!(
            broker_a.handle_peer_text("c1", frame.as_bytes()),
            skiff_router::ws::PeerTextOutcome::Close(close) if close.code == 1002
        ));

        let (broker2, _, _, _) = broker();
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        broker_attach(&broker2, "c1", "g1", &writer);
        assert!(matches!(
            broker2.handle_peer_text("c1", response_frame("unknown:0", "1").as_bytes()),
            skiff_router::ws::PeerTextOutcome::Close(close) if close.code == 1002
        ));
    }

    #[test]
    fn broker_peer_disconnect_settles_all_pending_and_aborts_inbound() {
        let (broker, responder, dispatch, _) = broker();
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::Success
        );
        let frame = r#"{"jsonrpc":"2.0","id":"p1","method":"chat.send","params":{}}"#;
        assert!(broker
            .handle_peer_text("c1", frame.as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        let action = dispatch.actions().pop().expect("dispatch action");
        let _ = broker.handle_peer_disconnect("c1");
        assert_eq!(broker.snapshot().generation_count, 0);
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert_eq!(broker.snapshot().inbound_pending, 0);
        assert!(
            *action.cancel.borrow(),
            "inbound must be aborted on peer close"
        );
        assert_eq!(
            responder.responses()[0].outcome,
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::TransportUnavailable
        );
        assert_eq!(broker.snapshot().outbound_tombstones, 0);
    }

    #[test]
    fn broker_writer_failure_fences_only_the_exact_request() {
        let (broker, responder, _, _) = broker();
        let writer = Arc::new(FakePeerWriter::new());
        writer.fail_next();
        let writer: Arc<dyn PeerWriter> = writer;
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-1", owner, None)),
            RuntimeRequestOutcome::TransportUnavailable
        );
        assert_eq!(broker.snapshot().outbound_pending, 0);
        assert_eq!(
            responder.responses()[0].outcome,
            skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::TransportUnavailable
        );
        assert_eq!(
            broker.handle_runtime_request("c1", &runtime, &request("req-2", owner, None)),
            RuntimeRequestOutcome::Success
        );
        assert_eq!(broker.snapshot().outbound_pending, 1);
    }

    #[test]
    fn broker_tombstone_fifo_eviction_permits_reuse_but_keeps_active_fence() {
        let responder = Arc::new(FakeRuntimeResponder::new());
        let broker = WebSocketRequestBroker::new(
            Arc::new(FakeMethodCatalog::new()),
            Arc::new(skiff_router::ws::NoopNotificationObserver),
            Arc::new(FakeRuntimeViolationSink::new()),
            Arc::new(FakeDispatchInbound::new()),
            WebSocketRequestBrokerOptions {
                outbound_per_generation_capacity: 3,
                inbound_per_generation_capacity: 2,
                tombstone_capacity: 2,
                ..Default::default()
            },
        );
        let broker = Arc::new(broker);
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let owner = broker_attach(&broker, "c1", "g1", &writer);
        let runtime = source(&responder, "r1");
        for request_id in ["req-1", "req-2", "req-3"] {
            assert_eq!(
                broker.handle_runtime_request("c1", &runtime, &request(request_id, owner, None)),
                RuntimeRequestOutcome::Success
            );
        }
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:0", "1").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:1", "2").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert!(broker
            .handle_peer_text("c1", response_frame("g1:2", "3").as_bytes())
            .eq(&skiff_router::ws::PeerTextOutcome::Ok));
        assert_eq!(broker.snapshot().outbound_tombstones, 2);
        assert!(
            matches!(
                broker.handle_peer_text("c1", response_frame("g1:0", "null").as_bytes()),
                skiff_router::ws::PeerTextOutcome::Close(close) if close.code == 1002
            ),
            "the first tombstone was evicted by FIFO; its late response is unknown"
        );
        assert_eq!(broker.snapshot().outbound_pending, 0);
    }

    // -----------------------------------------------------------------------
    // Ledger sequence tests
    // -----------------------------------------------------------------------

    #[test]
    fn ledger_exact_acquire_ack_and_cached_dedupe_keep_single_pin() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &tuple),
            ),
            AcquireDecision::Ack(_)
        ));
        assert_eq!(lane.ledger.snapshot().pins_acquired, 1);
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &tuple),
            ),
            AcquireDecision::Ack(_)
        ));
        assert_eq!(lane.ledger.snapshot().pins_acquired, 1);
        let mut conflict = tuple.clone();
        conflict.build_id = format!("skiff-service-deployment-v2:sha256:{}", "b".repeat(64));
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &conflict),
            ),
            AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                if code == skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::RequestConflict
        ));
        assert_eq!(lane.ledger.snapshot().pins_acquired, 1);
    }

    #[test]
    fn ledger_acquire_rejection_codes_are_exact() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &tuple),
            ),
            AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                if code == skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::NotAcquired
        ));
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let mut mismatch = tuple.clone();
        mismatch.build_id = format!("skiff-service-deployment-v2:sha256:{}", "b".repeat(64));
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &mismatch),
            ),
            AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                if code == skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::TupleMismatch
        ));
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &tuple),
            ),
            AcquireDecision::Ack(_)
        ));
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r2"),
                &acquire_control("acquire-2", &tuple),
            ),
            AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                if code == skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::SenderMismatch
        ));
        let other = pin_tuple("c2", "r1");
        let mut other = other.clone();
        other.router_session_id = "session-other".to_string();
        lane.ledger
            .expect_connection(other.clone())
            .expect("expect");
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-3", &other),
            ),
            AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                if code == skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::SenderMismatch
        ));
        assert!(lane.ledger.expect_connection(tuple.clone()).is_err());
        assert!(lane.ledger.fail_stop_reason().is_some());
    }

    #[test]
    fn ledger_release_pending_dedupe_ack_and_reject_paths() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let first = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        let second = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        assert_eq!(first, second, "release dedupe must share the pending");
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 1);
        let ack = WebSocketGenerationLifecycleControl::Ack {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            operation: WebSocketGenerationLifecycleOperation::Release,
            request_id: first,
            sender: WebSocketGenerationLifecycleSender::Runtime,
            tuple: tuple.clone(),
        };
        lane.ledger
            .handle_release_response(&runtime_session("r1"), &ack)
            .expect("ack");
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 0);
        assert_eq!(lane.ledger.snapshot().release_acks, 1);
        assert_eq!(lane.ledger.snapshot().pins_acquired, 0);

        let tuple = pin_tuple("c2", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-2", &tuple),
        );
        let request_id = match lane.ledger.release_connection("c2", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        let reject = WebSocketGenerationLifecycleControl::Reject {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            operation: WebSocketGenerationLifecycleOperation::Release,
            request_id,
            sender: WebSocketGenerationLifecycleSender::Runtime,
            tuple: tuple.clone(),
            code: skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleRejectionCode::GenerationUnavailable,
            reason: "generation gone".to_string(),
        };
        lane.ledger
            .handle_release_response(&runtime_session("r1"), &reject)
            .expect("reject handled");
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 0);
        assert_eq!(lane.ledger.snapshot().pins_acquired, 0);
        assert_eq!(lane.ledger.snapshot().release_failures.len(), 1);
        assert_eq!(lane.ledger.snapshot().runtime_closed.len(), 1);
    }

    #[test]
    fn ledger_release_timeout_never_retains_pin() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let request_id = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        assert!(lane.ledger.fire_release_timeout(&request_id).is_some());
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 0);
        assert_eq!(lane.ledger.snapshot().pins_acquired, 0);
        assert_eq!(lane.ledger.snapshot().release_failures.len(), 1);
        assert_eq!(lane.ledger.snapshot().runtime_closed.len(), 1);
    }

    #[test]
    fn ledger_disconnect_clears_cached_acquires_and_pending() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let _request_id = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        lane.ledger.runtime_disconnected(&runtime_session("r1"));
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 0);
        assert_eq!(lane.ledger.snapshot().pins_acquired, 0);
        assert_eq!(lane.ledger.snapshot().cached_acquire_count, 0);
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        assert!(matches!(
            lane.ledger.handle_acquire(
                &runtime_session("r1"),
                &acquire_control("acquire-1", &tuple),
            ),
            AcquireDecision::Ack(_)
        ));
        assert_eq!(lane.ledger.snapshot().pins_acquired, 1);
    }

    #[test]
    fn ledger_send_failure_does_not_silently_retain_the_pin() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let fake_peer = Arc::new(FakeRuntimePeer::new());
        let _ = fake_peer;
        let _request_id = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 1);
        assert_eq!(lane.ledger.snapshot().release_failures.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ledger_flush_aggregates_release_failures() {
        let lane = lane();
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let request_id = match lane.ledger.release_connection("c1", true).expect("release") {
            skiff_router::ws::ReleaseOutcome::Pending(handle) => handle.request_id,
            _ => panic!("pending release expected"),
        };
        assert!(lane.ledger.fire_release_timeout(&request_id).is_some());
        let result = lane.ledger.flush().await;
        assert!(result.is_err(), "flush must surface the release failure");
        assert_eq!(lane.ledger.snapshot().pins_pending_release, 0);
        assert_eq!(lane.ledger.snapshot().pins_acquired, 0);
    }

    // -----------------------------------------------------------------------
    // Index / lane finalizer tests
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_captured_writer_is_fenced_after_replacement() {
        let lane = lane();
        lane.reserve("c1").expect("reserve");
        let _ = lane.admit(
            "c1",
            Some(BusinessKey::from_raw("b1")),
            None,
            1,
            OverflowPolicy::CloseOldest,
        );
        let writer_c1: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        let captured = lane
            .attach(
                "c1",
                1,
                "g1".to_string(),
                runtime_session("r1"),
                writer_c1,
                AttachMeta {
                    service_id: "example.com/chat".to_string(),
                    websocket_entry_id: format!(
                        "skiff-websocket-entry-v1:sha256:{}",
                        "b".repeat(64)
                    ),
                    profile: WebSocketRpcProfile::JsonRpc2_0Text,
                },
            )
            .expect("attach c1");
        lane.reserve("c2").expect("reserve c2");
        let _ = lane.admit(
            "c2",
            Some(BusinessKey::from_raw("b1")),
            None,
            1,
            OverflowPolicy::CloseOldest,
        );
        assert_eq!(
            lane.index.connection_terminal("c1").as_deref(),
            Some("Replacement")
        );
        assert!(captured.write_text("stale".to_string()).is_err());
        let fake_c2 = Arc::new(FakePeerWriter::new());
        let writer_c2: Arc<dyn PeerWriter> = fake_c2.clone();
        let _ = lane
            .attach(
                "c2",
                2,
                "g2".to_string(),
                runtime_session("r1"),
                writer_c2,
                AttachMeta {
                    service_id: "example.com/chat".to_string(),
                    websocket_entry_id: format!(
                        "skiff-websocket-entry-v1:sha256:{}",
                        "b".repeat(64)
                    ),
                    profile: WebSocketRpcProfile::JsonRpc2_0Text,
                },
            )
            .expect("attach c2");
        wait_for_finalizers(&lane).await;
        assert_eq!(lane.snapshot().generation_count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_slow_client_budget_finishes_with_1011() {
        let lane = lane();
        attach_connection(&lane, "c1", "g1", "r1");
        assert_eq!(
            lane.index.slow_client_write("c1", 2048),
            WriteBudget::OverBudget
        );
        let _ = lane.finish("c1", ClientTerminal::SlowClient, None);
        wait_for_finalizers(&lane).await;
        assert_eq!(
            lane.index.connection_terminal("c1").as_deref(),
            Some("SlowClient")
        );
        assert_eq!(lane.snapshot().slow_client_count, 1);
        assert_eq!(lane.snapshot().connection_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_release_timeout_completes_client_terminal_and_closes_runtime() {
        let lane = lane();
        attach_connection(&lane, "c1", "g1", "r1");
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let _ = lane.handle_peer_disconnect("c1");
        let request_id = lane
            .ledger
            .pending_release_request_id("c1")
            .expect("pending release");
        assert!(lane.ledger.fire_release_timeout(&request_id).is_some());
        wait_for_finalizers(&lane).await;
        assert_eq!(
            lane.index.connection_terminal("c1").as_deref(),
            Some("PeerClose")
        );
        assert_eq!(lane.snapshot().pins_acquired, 0);
        assert_eq!(lane.snapshot().pins_pending_release, 0);
        assert!(!lane.snapshot().runtime_closed.is_empty());
        assert_eq!(lane.snapshot().finalizer_failures.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_shutdown_drains_finalizers_and_uses_1001() {
        let lane = lane();
        attach_connection(&lane, "c1", "g1", "r1");
        let tuple = pin_tuple("c1", "r1");
        lane.ledger
            .expect_connection(tuple.clone())
            .expect("expect");
        let _ = lane.ledger.handle_acquire(
            &runtime_session("r1"),
            &acquire_control("acquire-1", &tuple),
        );
        let _ = lane.shutdown();
        let request_id = lane
            .ledger
            .pending_release_request_id("c1")
            .expect("pending release");
        let ack = WebSocketGenerationLifecycleControl::Ack {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            operation: WebSocketGenerationLifecycleOperation::Release,
            request_id,
            sender: WebSocketGenerationLifecycleSender::Runtime,
            tuple,
        };
        lane.ledger
            .handle_release_response(&runtime_session("r1"), &ack)
            .expect("ack");
        wait_for_finalizers(&lane).await;
        assert_eq!(
            lane.index.connection_terminal("c1").as_deref(),
            Some("Shutdown")
        );
        assert_eq!(lane.snapshot().release_acks, 1);
        assert_eq!(lane.snapshot().connection_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_peer_close_writes_no_close_frame() {
        let lane = lane();
        let fake = Arc::new(FakePeerWriter::new());
        let writer: Arc<dyn PeerWriter> = fake.clone();
        lane.reserve("c1").expect("reserve");
        let _ = lane.admit("c1", None, None, 1, OverflowPolicy::CloseOldest);
        let _ = lane
            .attach(
                "c1",
                1,
                "g1".to_string(),
                runtime_session("r1"),
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
        let _ = lane.handle_peer_disconnect("c1");
        wait_for_finalizers(&lane).await;
        assert!(
            fake.terminated(),
            "peer close must terminate without close frame"
        );
        assert!(fake.close().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_protocol_close_and_binary_close_codes() {
        let lane_a = lane();
        let fake = attach_connection_with_writer(&lane_a, "c1", "g1", "r1");
        let frame = r#"{"jsonrpc":"2.0","id":"p1","method":"chat.send","params":{}}"#;
        let _ = lane_a.handle_peer_text("c1", frame.as_bytes());
        let _ = lane_a.handle_peer_text("c1", frame.as_bytes());
        wait_for_finalizers(&lane_a).await;
        assert_eq!(
            lane_a.index.connection_terminal("c1").as_deref(),
            Some("ProtocolClose")
        );
        assert_eq!(fake.close().map(|(code, _)| code), Some(1002));

        let lane2 = lane();
        let fake = attach_connection_with_writer(&lane2, "c2", "g1", "r1");
        let _ = lane2.handle_peer_binary("c2");
        wait_for_finalizers(&lane2).await;
        assert_eq!(
            lane2.index.connection_terminal("c2").as_deref(),
            Some("ProtocolClose")
        );
        assert_eq!(fake.close().map(|(code, _)| code), Some(1003));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lane_runtime_disconnect_finishes_attached_connections_with_1011() {
        let lane = lane();
        attach_connection(&lane, "c1", "g1", "r1");
        attach_connection(&lane, "c2", "g2", "r1");
        attach_connection(&lane, "c3", "g3", "r2");
        let _ = lane.runtime_disconnected(&runtime_session("r1"));
        wait_for_finalizers(&lane).await;
        assert_eq!(
            lane.index.connection_terminal("c1").as_deref(),
            Some("RuntimeDisconnect")
        );
        assert_eq!(
            lane.index.connection_terminal("c2").as_deref(),
            Some("RuntimeDisconnect")
        );
        assert_eq!(
            lane.index.connection_terminal("c3").as_deref(),
            Some("None")
        );
        assert_eq!(lane.snapshot().generation_count, 1);
    }
}
