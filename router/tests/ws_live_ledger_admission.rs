//! E-ws gate production small-fix tests (`router-live:ws`):
//!
//! 1. `RuntimeGenerationPinLedger` admission expectation uses TS
//!    `matchesExpectation` parity (5 fields, ignoring the Runtime-minted
//!    `routerSessionId`), while cached/acquired checks keep full tuple
//!    equality (`tuplesEqual` parity).
//! 2. `WsDispatchStore`/`WsPendingAdmissionSender` answer the Runtime
//!    generation `Acquire` while the websocketConnect dispatch is still
//!    pending (before `register_connection`), matching the real Runtime
//!    ordering (acquire is sent before `response.end`).

mod ws_harness;

use skiff_artifact_model::{
    AssemblyIdentity, DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity,
    ServiceDeploymentRef,
};
use skiff_router::dispatch::RuntimeAdmissionPool;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::supervisor::ws::{
    ConnectOutcome, WsBinding, WsConnectMetadata, WsConnectionRecord, WsDispatchStore,
    WsLaneHandle, WsPendingAdmissionSender, WsSessionWriter,
};
use skiff_router::ws::{
    AcquireDecision, AllowAnyPendingAdmission, ClientConnectionIndexOptions, DispatchInbound,
    LedgerOptions, NoopNotificationObserver, OverflowPolicy, PeerWriter, PendingAdmissionSender,
    RuntimeGenerationPinLedger, WebSocketLane, WebSocketLaneOptions, WebSocketRequestBrokerOptions,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleRejectionCode,
    WebSocketGenerationLifecycleTuple,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ws_harness::{
    acquire_control, pin_tuple, runtime_session, FakeDispatchInbound, FakeMethodCatalog,
    FakePeerWriter, FakeRuntimePeer, FakeRuntimeSessionClose, FakeRuntimeViolationSink,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn websocket_entry() -> String {
        format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64))
    }

    fn entry_identity(tag: &str) -> GatewayEntryIdentity {
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", sha_digest(tag)))
            .expect("gateway entry identity")
    }

    fn sha_digest(seed: &str) -> String {
        let mut digest = String::new();
        for byte in seed.bytes().chain(std::iter::repeat(0)) {
            digest.push_str(&format!("{byte:02x}"));
            if digest.len() >= 64 {
                break;
            }
        }
        while digest.len() < 64 {
            digest.push('0');
        }
        digest
    }

    fn assembly_identity() -> AssemblyIdentity {
        AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            sha_digest("assembly")
        ))
    }

    fn binding() -> WsBinding {
        WsBinding {
            service_id: "example.com/chat".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: "example.com/chat".to_string(),
                contract_version: "example.com/chat@1".to_string(),
                deployment_revision: DeploymentRevision::new("1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                    "skiff-deployment-artifact-v4:sha256:{}",
                    sha_digest("deployment")
                )),
            },
            gateway_entry_identity: entry_identity("connect"),
            websocket_entry_id: websocket_entry(),
            path: "/ws".to_string(),
            connect_handler: true,
            methods: BTreeMap::from([(
                "chat.send".to_string(),
                skiff_router::supervisor::ws::WsMethodBinding {
                    method: "chat.send".to_string(),
                    gateway_entry_identity: entry_identity("chat.send"),
                },
            )]),
        }
    }

    fn record(connection_id: &str, runtime: &RuntimeSessionEpoch) -> WsConnectionRecord {
        WsConnectionRecord {
            connection_id: connection_id.to_string(),
            runtime: runtime.clone(),
            binding: binding(),
            business_identity: None,
            assembly_identity: assembly_identity(),
            assembly_generation: 7,
        }
    }

    fn router_side_tuple(
        connection_id: &str,
        runtime: &RuntimeSessionEpoch,
    ) -> WebSocketGenerationLifecycleTuple {
        let mut tuple = pin_tuple(connection_id, &runtime.replica_id);
        tuple.router_session_id =
            format!("{}#{}", runtime.replica_id, runtime.connection_generation);
        tuple.assembly_identity = assembly_identity();
        tuple.assembly_generation = 7;
        tuple.websocket_entry_id = websocket_entry();
        tuple
    }

    fn runtime_minted_tuple(
        connection_id: &str,
        runtime: &RuntimeSessionEpoch,
    ) -> WebSocketGenerationLifecycleTuple {
        let mut tuple = router_side_tuple(connection_id, runtime);
        tuple.router_session_id = format!("skiff-router-session-v1:opaque:{}", "c".repeat(36));
        tuple
    }

    #[derive(Debug, Default)]
    struct FakeWsSessionWriter {
        frames: Arc<Mutex<Vec<(RuntimeSessionEpoch, Vec<u8>)>>>,
    }

    impl WsSessionWriter for FakeWsSessionWriter {
        fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
            self.frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((runtime.clone(), bytes));
            Ok(())
        }
    }

    fn lane_with_pending_admission(store: &Arc<WsDispatchStore>) -> Arc<WebSocketLane> {
        WebSocketLane::new(
            WebSocketLaneOptions {
                index: ClientConnectionIndexOptions {
                    connection_limit: 8,
                    slow_client_budget_bytes: 1024 * 1024,
                    high_water_capacity: 8,
                },
                broker: WebSocketRequestBrokerOptions {
                    inbound_timeout_ms: 1000,
                    ..Default::default()
                },
                ..Default::default()
            },
            Arc::new(FakeRuntimePeer::new()),
            Arc::new(FakeRuntimeSessionClose::new()),
            Arc::new(WsPendingAdmissionSender::new(Arc::clone(store))),
            Arc::new(FakeMethodCatalog::new()),
            Arc::new(NoopNotificationObserver),
            Arc::new(FakeRuntimeViolationSink::new()),
            Arc::new(FakeDispatchInbound::new()),
        )
    }

    fn store_with(writer: &Arc<FakeWsSessionWriter>) -> Arc<WsDispatchStore> {
        WsDispatchStore::new(
            WsLaneHandle::new(),
            writer.clone(),
            RuntimeAdmissionPool::new(4),
            1000,
        )
    }

    fn lane_with_dispatch(dispatch: Arc<dyn DispatchInbound>) -> Arc<WebSocketLane> {
        WebSocketLane::new(
            WebSocketLaneOptions {
                index: ClientConnectionIndexOptions {
                    connection_limit: 8,
                    slow_client_budget_bytes: 1024 * 1024,
                    high_water_capacity: 8,
                },
                broker: WebSocketRequestBrokerOptions {
                    inbound_timeout_ms: 1000,
                    ..Default::default()
                },
                ..Default::default()
            },
            Arc::new(FakeRuntimePeer::new()),
            Arc::new(FakeRuntimeSessionClose::new()),
            Arc::new(AllowAnyPendingAdmission),
            Arc::new(FakeMethodCatalog::new()),
            Arc::new(NoopNotificationObserver),
            Arc::new(FakeRuntimeViolationSink::new()),
            dispatch,
        )
    }

    #[test]
    fn broker_forwards_params_value_slice_not_member_with_key() {
        let dispatch = Arc::new(FakeDispatchInbound::new());
        let lane = lane_with_dispatch(Arc::clone(&dispatch) as Arc<dyn DispatchInbound>);
        lane.reserve("c1").expect("reserve");
        let _ = lane.admit("c1", None, None, 8, OverflowPolicy::CloseOldest);
        let writer: Arc<dyn PeerWriter> = Arc::new(FakePeerWriter::new());
        lane.attach(
        "c1",
        1,
        "c1".to_string(),
        runtime_session("r1"),
        writer,
        skiff_router::ws::AttachMeta {
            service_id: "example.com/chat".to_string(),
            websocket_entry_id: websocket_entry(),
            profile:
                skiff_runtime_transport::connection_protocol::WebSocketRpcProfile::JsonRpc2_0Text,
        },
    )
    .expect("attach");
        let frame = r#"{"jsonrpc":"2.0","id":"p1","method":"chat.send","params":{"value":"x"}}"#;
        assert!(lane.handle_peer_text("c1", frame.as_bytes()).is_none());
        let action = dispatch.actions().pop().expect("dispatch action");
        assert_eq!(
            action.params, br#"{"value":"x"}"#,
            "broker must forward the params value slice, not the member including the key"
        );
    }

    // ---------------------------------------------------------------------------
    // Ledger expectation parity (Gap 1)
    // ---------------------------------------------------------------------------

    fn ledger(admission: Arc<dyn PendingAdmissionSender>) -> RuntimeGenerationPinLedger {
        RuntimeGenerationPinLedger::new(
            Arc::new(FakeRuntimePeer::new()),
            Arc::new(FakeRuntimeSessionClose::new()),
            admission,
            LedgerOptions::default(),
        )
    }

    #[test]
    fn ledger_expectation_matches_ignoring_runtime_minted_router_session_id() {
        let runtime = runtime_session("r1");
        let expected = router_side_tuple("c1", &runtime);
        let acquire = runtime_minted_tuple("c1", &runtime);
        assert_ne!(expected.router_session_id, acquire.router_session_id);

        let lane = ledger(Arc::new(AllowAnyPendingAdmission));
        lane.expect_connection(expected).expect("expect");
        let decision = lane.handle_acquire(&runtime, &acquire_control("acquire-1", &acquire));
        assert!(
            matches!(decision, AcquireDecision::Ack(_)),
            "TS matchesExpectation ignores routerSessionId; real Runtime acquire must Ack"
        );
        assert_eq!(lane.snapshot().pins_acquired, 1);
    }

    #[test]
    fn ledger_expectation_rejects_each_matching_field_mismatch() {
        let runtime = runtime_session("r1");
        let expected = router_side_tuple("c1", &runtime);
        let lane = ledger(Arc::new(AllowAnyPendingAdmission));
        lane.expect_connection(expected.clone()).expect("expect");

        let mut cases = Vec::new();
        let mut service = runtime_minted_tuple("c1", &runtime);
        service.service_id = "other.example/chat".to_string();
        cases.push(service);
        let mut assembly = runtime_minted_tuple("c1", &runtime);
        assembly.assembly_identity = AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "d".repeat(64)
        ));
        cases.push(assembly);
        let mut generation = runtime_minted_tuple("c1", &runtime);
        generation.assembly_generation = 8;
        cases.push(generation);
        let mut entry = runtime_minted_tuple("c1", &runtime);
        entry.websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "e".repeat(64));
        cases.push(entry);
        let mut connection = runtime_minted_tuple("c1", &runtime);
        connection.connection_id = "c2".to_string();
        cases.push(connection);

        for (index, mismatch) in cases.into_iter().enumerate() {
            let expected_code = if mismatch.connection_id != expected.connection_id {
                // TS parity: a different connection id misses the expectation map
                // and is rejected `not-acquired`, not `tuple-mismatch`.
                WebSocketGenerationLifecycleRejectionCode::NotAcquired
            } else {
                WebSocketGenerationLifecycleRejectionCode::TupleMismatch
            };
            let decision = lane.handle_acquire(
                &runtime,
                &acquire_control(&format!("acquire-mismatch-{index}"), &mismatch),
            );
            assert!(
                matches!(
                    decision,
                    AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                        if code == expected_code
                ),
                "field mismatch case {index} must reject {expected_code:?}"
            );
        }
        assert_eq!(lane.snapshot().pins_acquired, 0);
    }

    #[test]
    fn ledger_cached_acquire_keeps_full_tuple_equality() {
        let runtime = runtime_session("r1");
        let lane = ledger(Arc::new(AllowAnyPendingAdmission));
        lane.expect_connection(router_side_tuple("c1", &runtime))
            .expect("expect");
        let first = runtime_minted_tuple("c1", &runtime);
        assert!(matches!(
            lane.handle_acquire(&runtime, &acquire_control("acquire-1", &first)),
            AcquireDecision::Ack(_)
        ));
        let mut reused = first.clone();
        reused.router_session_id = "skiff-router-session-v1:opaque:other".to_string();
        let decision = lane.handle_acquire(&runtime, &acquire_control("acquire-1", &reused));
        assert!(
            matches!(
                decision,
                AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                    if code == WebSocketGenerationLifecycleRejectionCode::RequestConflict
            ),
            "cached acquire keeps tuplesEqual parity (routerSessionId is significant)"
        );
    }

    #[test]
    fn ledger_acquired_pin_keeps_full_tuple_equality() {
        let first_runtime = runtime_session("r1");
        let second_runtime = runtime_session("r2");
        let lane = ledger(Arc::new(AllowAnyPendingAdmission));
        lane.expect_connection(router_side_tuple("c1", &first_runtime))
            .expect("expect");
        let first = runtime_minted_tuple("c1", &first_runtime);
        assert!(matches!(
            lane.handle_acquire(&first_runtime, &acquire_control("acquire-1", &first)),
            AcquireDecision::Ack(_)
        ));
        let mut second = runtime_minted_tuple("c1", &second_runtime);
        second.router_session_id = "skiff-router-session-v1:opaque:second".to_string();
        let decision = lane.handle_acquire(&second_runtime, &acquire_control("acquire-2", &second));
        assert!(
            matches!(
                decision,
                AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject { code, .. })
                    if code == WebSocketGenerationLifecycleRejectionCode::TupleMismatch
            ),
            "acquired pin keeps tuplesEqual parity (different sender/tuple rejects)"
        );
    }

    // ---------------------------------------------------------------------------
    // Supervisor pending-admission timing (Gap 2)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn pending_admission_hits_acquire_before_connect_registration() {
        let runtime = runtime_session("r1");
        let other = runtime_session("r2");
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&writer);
        let (request_id, _rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &assembly_identity(),
                7,
                &WsConnectMetadata::default(),
                1000,
            )
            .expect("connect begin");
        assert_eq!(store.pinned_connection_count(), 0);
        assert_eq!(store.pending_admission_count(), 1);

        let sender = WsPendingAdmissionSender::new(Arc::clone(&store));
        let tuple = runtime_minted_tuple("wsconn-1", &runtime);
        assert!(
            sender.is_pending_acquire_sender(&runtime, &tuple),
            "real Runtime acquire arrives while websocketConnect is pending"
        );
        assert!(!sender.is_pending_acquire_sender(&other, &tuple));
        let mut wrong_connection = tuple.clone();
        wrong_connection.connection_id = "wsconn-2".to_string();
        assert!(!sender.is_pending_acquire_sender(&runtime, &wrong_connection));
        let mut wrong_generation = tuple.clone();
        wrong_generation.assembly_generation = 8;
        assert!(!sender.is_pending_acquire_sender(&runtime, &wrong_generation));

        store.connect_response(
            &request_id,
            ConnectOutcome::Accepted {
                business_identity: None,
                admission_rank: None,
                max_connections: u32::MAX,
                overflow: OverflowPolicy::RejectNew,
                close_code: None,
                close_reason: None,
            },
        );
        assert_eq!(store.pending_connect_count(), 0);
        assert_eq!(store.pending_admission_count(), 0);
        assert!(!sender.is_pending_acquire_sender(&runtime, &tuple));
    }

    #[tokio::test]
    async fn pending_admission_removed_on_unavailable_and_session_close() {
        let runtime = runtime_session("r1");
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&writer);

        let (request_id, _rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &assembly_identity(),
                7,
                &WsConnectMetadata::default(),
                1000,
            )
            .expect("connect begin");
        store.connect_unavailable(&request_id, "timeout".to_string());
        assert_eq!(store.pending_admission_count(), 0);

        let (_request_id, _rx) = store
            .connect_begin(
                "wsconn-2",
                &binding(),
                &runtime,
                &assembly_identity(),
                7,
                &WsConnectMetadata::default(),
                1000,
            )
            .expect("connect begin");
        store.on_session_closed(&runtime);
        assert_eq!(store.pending_connect_count(), 0);
        assert_eq!(store.pending_admission_count(), 0);
        assert_eq!(store.pinned_connection_count(), 0);
    }

    #[tokio::test]
    async fn pending_admission_sender_falls_back_to_pinned_connection() {
        let runtime = runtime_session("r1");
        let writer = Arc::new(FakeWsSessionWriter::default());
        let store = store_with(&writer);
        store.register_connection(record("wsconn-1", &runtime));
        let sender = WsPendingAdmissionSender::new(Arc::clone(&store));
        let tuple = runtime_minted_tuple("wsconn-1", &runtime);
        assert!(sender.is_pending_acquire_sender(&runtime, &tuple));
        assert!(!sender.is_pending_acquire_sender(&runtime_session("r2"), &tuple));
    }

    // ---------------------------------------------------------------------------
    // Full real-order lane sequence: acquire while connect pending -> Ack ->
    // settle -> admit -> register (listener.rs order)
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn real_order_acquire_ack_before_connect_settle_admit_register() {
        let runtime = runtime_session("r1");
        let writer = Arc::new(FakeWsSessionWriter::default());
        let handle = WsLaneHandle::new();
        let store = WsDispatchStore::new(
            handle.clone(),
            writer.clone(),
            RuntimeAdmissionPool::new(4),
            1000,
        );
        let lane = lane_with_pending_admission(&store);
        handle.set(Arc::clone(&lane));

        lane.ledger
            .expect_connection(router_side_tuple("wsconn-1", &runtime))
            .expect("expect connection");
        let (request_id, _rx) = store
            .connect_begin(
                "wsconn-1",
                &binding(),
                &runtime,
                &assembly_identity(),
                7,
                &WsConnectMetadata::default(),
                1000,
            )
            .expect("connect begin");

        // The Runtime acquire (self-minted router session id) arrives while the
        // websocketConnect dispatch is still pending and no pinned record exists.
        assert_eq!(store.pinned_connection_count(), 0);
        let acquire = runtime_minted_tuple("wsconn-1", &runtime);
        assert!(
            matches!(
                lane.ledger
                    .handle_acquire(&runtime, &acquire_control("acquire-1", &acquire)),
                AcquireDecision::Ack(_)
            ),
            "real-order acquire must Ack during pending admission"
        );
        assert_eq!(lane.ledger.snapshot().pins_acquired, 1);

        store.connect_response(
            &request_id,
            ConnectOutcome::Accepted {
                business_identity: None,
                admission_rank: None,
                max_connections: u32::MAX,
                overflow: OverflowPolicy::RejectNew,
                close_code: None,
                close_reason: None,
            },
        );
        assert_eq!(store.pending_admission_count(), 0);
        lane.reserve("wsconn-1").expect("reserve");
        assert!(matches!(
            lane.admit(
                "wsconn-1",
                None,
                None,
                u32::MAX as usize,
                OverflowPolicy::RejectNew,
            ),
            skiff_router::ws::AdmissionOutcome::Accepted
        ));
        store.register_connection(record("wsconn-1", &runtime));
        assert_eq!(store.pinned_connection_count(), 1);
    }
}
