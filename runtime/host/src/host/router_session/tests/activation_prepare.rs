use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_capability_context::{
    DbCapabilityFuture, DbCapabilityResult, DbCapabilitySource, DbProviderBuildInput,
    DbProviderFactory, DbProviderSource,
};
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection, ASSEMBLY_ACTIVATION_FRAME_TYPE,
    },
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, TypedEnvelope, RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        decode_runtime_assembly_websocket_connect_response_end_frame,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader,
        RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
    },
    websocket_generation_lifecycle::{
        decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
        WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
        WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleSender,
        WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    },
};
use tokio::{
    io::{duplex, AsyncWriteExt},
    sync::Notify,
    time::timeout,
};
use tokio_tungstenite::{
    tungstenite::{protocol::Role, Message},
    WebSocketStream,
};

use crate::host::RuntimeHost;

#[derive(Clone, Default)]
struct SessionBlockingDbProvider {
    blocking: Arc<AtomicBool>,
    panic_on_provision: Arc<AtomicBool>,
    starts: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
    completed: Arc<Notify>,
    dropped: Arc<Notify>,
}

impl DbProviderFactory for SessionBlockingDbProvider {
    fn build(&self, _input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        Ok(DbCapabilitySource::unavailable())
    }

    fn provision<'a>(&'a self, _inputs: Vec<DbProviderBuildInput>) -> DbCapabilityFuture<'a, ()> {
        let blocking = Arc::clone(&self.blocking);
        let panic_on_provision = Arc::clone(&self.panic_on_provision);
        let starts = Arc::clone(&self.starts);
        let started = Arc::clone(&self.started);
        let release = Arc::clone(&self.release);
        let completed = Arc::clone(&self.completed);
        let dropped = Arc::clone(&self.dropped);
        Box::pin(async move {
            assert!(
                !panic_on_provision.load(Ordering::Acquire),
                "injected activation prepare panic"
            );
            if !blocking.load(Ordering::Acquire) {
                return Ok(());
            }
            struct DropNotification(Arc<Notify>);
            impl Drop for DropNotification {
                fn drop(&mut self) {
                    self.0.notify_one();
                }
            }
            let _drop_notification = DropNotification(dropped);
            starts.fetch_add(1, Ordering::AcqRel);
            started.notify_one();
            release.notified().await;
            completed.notify_one();
            Ok(())
        })
    }
}

#[tokio::test]
async fn completed_prepare_sends_exactly_one_terminal_then_commit_registers_generation() {
    let mut session = ActivationSession::start("prepared-commit").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-prepared-commit");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("prepared commit provider entry")
        .await;
    session.send_activation(&prepare).await;
    session.provider.release.notify_one();

    let prepared = session.recv_activation("Prepared terminal").await;
    assert!(
        matches!(
            &prepared,
            AssemblyActivationControl::Prepared {
                activation_id,
                ..
            } if activation_id == "activation-prepared-commit"
        ),
        "unexpected prepare terminal: {prepared:?}"
    );
    assert_eq!(session.provider.starts.load(Ordering::Acquire), 1);
    session
        .ping_without_activation_terminal(b"after-prepared", "activation-prepared-commit")
        .await;

    session.send_activation(&activation_commit(&prepare)).await;
    let registered = session.recv_activation("committed registration").await;
    assert!(matches!(
        registered,
        AssemblyActivationControl::Register { generation: 2, .. }
    ));
    assert_eq!(
        session
            .host
            .active_runtime_assembly()
            .unwrap()
            .unwrap()
            .generation(),
        2
    );
    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn completion_racing_exact_abort_suppresses_terminal_and_cleans_staging() {
    let mut session = ActivationSession::start("completion-abort").await;
    session.provider.blocking.store(true, Ordering::Release);
    for iteration in 0_u32..100 {
        let activation_id = format!("activation-completion-abort-{iteration}");
        let prepare = session.prepare(&activation_id);
        session.send_activation(&prepare).await;
        session
            .wait_for_provider_start("completion abort provider entry")
            .await;

        session.provider.release.notify_one();
        session.send_activation(&activation_abort(&prepare)).await;
        session
            .ping_without_activation_terminal(&iteration.to_be_bytes(), &activation_id)
            .await;
        assert_clean_cancelled_activation(&session.host);
    }
    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn terminal_send_failure_runs_exact_abort_cleanup_before_returning_primary_error() {
    let mut session = ActivationSession::start("terminal-send-failure").await;
    let prepare = session.prepare("test-fault-terminal-send");
    session.send_activation(&prepare).await;

    let error = session
        .wait_for_session("terminal send failure cleanup")
        .await
        .expect_err("terminal send failure must fail the session");
    assert!(
        error
            .to_string()
            .contains("injected assembly activation terminal send failure"),
        "unexpected terminal send primary error: {error}"
    );
    assert_clean_cancelled_activation(&session.host);
}

#[tokio::test]
async fn close_and_eof_cancel_pending_prepare_with_exact_synthetic_abort() {
    let mut close_session = ActivationSession::start("close-pending").await;
    close_session
        .provider
        .blocking
        .store(true, Ordering::Release);
    let close_prepare = close_session.prepare("activation-close-pending");
    close_session.send_activation(&close_prepare).await;
    close_session
        .wait_for_provider_start("close pending provider entry")
        .await;
    close_session.close().await.expect("close cleanup");
    assert_clean_cancelled_activation(&close_session.host);

    let mut eof_session = ActivationSession::start("eof-pending").await;
    eof_session.provider.blocking.store(true, Ordering::Release);
    let eof_prepare = eof_session.prepare("activation-eof-pending");
    eof_session.send_activation(&eof_prepare).await;
    eof_session
        .wait_for_provider_start("EOF pending provider entry")
        .await;
    eof_session.end_transport().await;
    let eof_error = eof_session
        .wait_for_session("EOF cleanup")
        .await
        .expect_err("transport EOF without close handshake is the primary session error");
    assert!(eof_error.to_string().contains("router read failed"));
    assert_clean_cancelled_activation(&eof_session.host);
}

#[tokio::test]
async fn prepare_task_panic_preserves_join_error_after_exact_abort_cleanup() {
    let mut session = ActivationSession::start("prepare-panic").await;
    session
        .provider
        .panic_on_provision
        .store(true, Ordering::Release);
    let prepare = session.prepare("activation-prepare-panic");
    session.send_activation(&prepare).await;

    let error = session
        .wait_for_session("prepare panic cleanup")
        .await
        .expect_err("prepare panic must fail session");
    assert!(error
        .to_string()
        .contains("assembly activation prepare task failed"));
    assert_clean_cancelled_activation(&session.host);
}

#[tokio::test]
async fn continuous_inbound_frames_do_not_starve_ready_prepare_terminal() {
    let mut session = ActivationSession::start("fair-terminal").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-fair-terminal");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("fair terminal provider entry")
        .await;
    session.provider.release.notify_one();
    for sequence in 0..256_u16 {
        session.send_ping(&sequence.to_be_bytes()).await;
    }
    let terminal = session
        .recv_activation("non-starved Prepared terminal")
        .await;
    assert!(
        matches!(&terminal, AssemblyActivationControl::Prepared { .. }),
        "unexpected fair terminal: {terminal:?}"
    );
    session.send_activation(&activation_abort(&prepare)).await;
    session
        .ping_without_activation_terminal(b"after-fair-abort", "activation-fair-terminal")
        .await;
    assert_clean_cancelled_activation(&session.host);
    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn different_prepare_fails_session_and_cleans_original_exact_tuple() {
    let mut session = ActivationSession::start("different-prepare").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-original-prepare");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("different prepare provider entry")
        .await;
    let different = session.prepare("activation-foreign-prepare");
    session.send_activation(&different).await;

    let error = session
        .wait_for_session("different prepare cleanup")
        .await
        .expect_err("different Prepare must fail session");
    assert!(error
        .to_string()
        .contains("different assembly activation prepare is already pending"));
    assert_clean_cancelled_activation(&session.host);
}

#[tokio::test]
async fn activation_contract_errors_run_through_the_live_session_path() {
    let mut before_bootstrap = ActivationSession::start_unbootstrapped("before-bootstrap").await;
    let prepare = before_bootstrap.prepare("activation-before-bootstrap");
    before_bootstrap.send_activation(&prepare).await;
    let error = before_bootstrap
        .wait_for_session("activation before bootstrap")
        .await
        .expect_err("activation before bootstrap must fail");
    assert!(error
        .to_string()
        .contains("assembly activation requires router.bootstrap first"));

    let mut foreign_environment = ActivationSession::start("foreign-environment").await;
    let mut prepare = foreign_environment.prepare("activation-foreign-environment");
    let AssemblyActivationControl::Prepare { environment, .. } = &mut prepare else {
        unreachable!();
    };
    *environment = "prod".to_string();
    foreign_environment.send_activation(&prepare).await;
    let error = foreign_environment
        .wait_for_session("foreign activation environment")
        .await
        .expect_err("foreign activation environment must fail");
    assert!(error
        .to_string()
        .contains("does not match Runtime trusted environment"));
    assert_no_activation_candidate(&foreign_environment.host);

    let mut transient_service_db = ActivationSession::start("transient-service-db").await;
    let mut prepare = transient_service_db.prepare("activation-transient-service-db");
    let AssemblyActivationControl::Prepare { service_db, .. } = &mut prepare else {
        unreachable!();
    };
    *service_db = Some(skiff_artifact_model::AssemblyActivationServiceDb {
        mongo_url: "mongodb://transient-owner".to_string(),
    });
    transient_service_db.send_activation(&prepare).await;
    let error = transient_service_db
        .wait_for_session("transient serviceDb")
        .await
        .expect_err("transient serviceDb must fail");
    assert!(error
        .to_string()
        .contains("assembly activation serviceDb is not supported"));
    assert_no_activation_candidate(&transient_service_db.host);
}

#[tokio::test]
async fn prepare_task_state_invariants_abort_exact_tuple_before_returning_primary_error() {
    for (activation_id, expected_error) in [
        (
            "test-fault-missing-prepare-task",
            "expected 1 prepare task(s), found 0",
        ),
        (
            "test-fault-multiple-prepare-tasks",
            "expected 1 prepare task(s), found 2",
        ),
        (
            "test-fault-complete-without-terminal",
            "prepare completed without a terminal reply",
        ),
    ] {
        let mut session = ActivationSession::start(activation_id).await;
        let prepare = session.prepare(activation_id);
        session.send_activation(&prepare).await;
        let error = session
            .wait_for_session("injected prepare state invariant")
            .await
            .expect_err("injected prepare state invariant must fail the session");
        assert!(
            error.to_string().contains(expected_error),
            "unexpected primary error for {activation_id}: {error}"
        );
        if activation_id == "test-fault-complete-without-terminal" {
            assert_clean_cancelled_activation(&session.host);
        } else {
            assert_no_activation_candidate(&session.host);
        }
    }
}

#[tokio::test]
async fn blocked_prepare_keeps_reader_live_and_exact_abort_cancels_without_late_terminal() {
    let mut session = ActivationSession::start("exact-abort").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-session-abort");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("first blocking provider entry")
        .await;

    // Exact duplicate Prepare is idempotent while the first task remains pending.
    session.send_activation(&prepare).await;
    session
        .ping_without_activation_terminal(b"prepare-reader-live", "activation-session-abort")
        .await;
    assert_eq!(session.provider.starts.load(Ordering::Acquire), 1);

    let abort = activation_abort(&prepare);
    session.send_activation(&abort).await;
    session.send_ping(b"after-prepare-abort").await;
    timeout(
        Duration::from_millis(100),
        session.provider.dropped.notified(),
    )
    .await
    .expect("Abort must drop pending provider future within 100ms");
    session
        .expect_pong_without_activation_terminal(b"after-prepare-abort", "activation-session-abort")
        .await;
    assert_clean_cancelled_activation(&session.host);

    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn blocked_prepare_keeps_ranked_websocket_connect_outbound_live() {
    let mut session = ActivationSession::start("ranked-connect-outbound").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-ranked-connect-outbound");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("ranked connect blocked provider entry")
        .await;

    let response = session.ranked_websocket_connect().await;
    assert_eq!(response.request_id, "ranked-connect-during-prepare");
    assert_eq!(
        response.websocket_connect,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
            business_identity: Some("ws://activation.test/socket".to_string()),
            connection_policy: None,
            admission_rank: Some(42),
        },
        "ordinary Host output must cross the session writer while Prepare remains pending"
    );
    assert_eq!(session.provider.starts.load(Ordering::Acquire), 1);
    assert_eq!(session.host.websocket_generations.pin_count().unwrap(), 1);

    session.send_activation(&activation_abort(&prepare)).await;
    session
        .ping_without_activation_terminal(
            b"after-ranked-connect-abort",
            "activation-ranked-connect-outbound",
        )
        .await;
    assert_clean_cancelled_activation(&session.host);
    session.close().await.expect("clean Router close");
    assert_eq!(session.host.websocket_generations.pin_count().unwrap(), 0);
}

#[tokio::test]
async fn mismatched_abort_fails_session_and_disconnect_cleanup_cancels_exact_pending_tuple() {
    let mut session = ActivationSession::start("mismatched-abort").await;
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-session-original");
    session.send_activation(&prepare).await;
    session
        .wait_for_provider_start("blocking provider entry")
        .await;

    let mut mismatched = activation_abort(&prepare);
    let AssemblyActivationControl::Abort { activation_id, .. } = &mut mismatched else {
        unreachable!();
    };
    *activation_id = "activation-session-foreign".to_string();
    session.send_activation(&mismatched).await;

    timeout(
        Duration::from_millis(100),
        session.provider.dropped.notified(),
    )
    .await
    .expect("session error cleanup must drop the exact pending provider future within 100ms");
    let error = timeout(Duration::from_millis(100), &mut session.session_task)
        .await
        .expect("session error cleanup must complete within 100ms")
        .expect("session task")
        .expect_err("mismatched Abort must fail the session");
    assert!(error
        .to_string()
        .contains("activation abort tuple does not match pending prepare"));
    assert_clean_cancelled_activation(&session.host);
}

struct ActivationSession {
    host: RuntimeHost,
    provider: SessionBlockingDbProvider,
    runtime_id: String,
    assembly: skiff_artifact_model::RuntimeAssemblyRef,
    config_snapshot: skiff_artifact_model::RuntimeConfigSnapshotRef,
    websocket_ingress: skiff_artifact_model::ServiceIngressKey,
    artifact_root: std::path::PathBuf,
    router: WebSocketStream<tokio::io::DuplexStream>,
    session_task: tokio::task::JoinHandle<crate::error::Result<()>>,
}

impl ActivationSession {
    async fn start(label: &str) -> Self {
        let mut session = Self::start_unbootstrapped(label).await;
        session.bootstrap().await;
        session
    }

    async fn start_unbootstrapped(label: &str) -> Self {
        let (assembly, artifact_root, config_snapshot) =
            super::runtime_assembly_request::fixture::blocking_activation_fixture();
        let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&assembly).unwrap();
        let websocket_ingress = assembly
            .gateway_ingress
            .iter()
            .find(|binding| binding.selector.path == "/socket" && binding.selector.method.is_none())
            .expect("activation fixture physical WebSocket ingress")
            .service_ingress_key();
        let config_snapshot_ref = config_snapshot.snapshot_ref().clone();
        let snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::open(
            artifact_root.join("runtime-config"),
        )
        .expect("activation config snapshot store");
        snapshot_store
            .publish(&config_snapshot)
            .expect("activation config snapshot publication");

        let provider = SessionBlockingDbProvider::default();
        let runtime_id = format!("runtime-session-activation-{label}");
        let host = RuntimeHost::new(crate::host::RuntimeConfig {
            db_provider: DbProviderSource::new(provider.clone()),
            router_url: "ws://127.0.0.1:4001/runtime".to_string(),
            base_runtime_id: runtime_id.clone(),
            runtime_home: std::env::temp_dir().join(format!(
                "skiff-runtime-session-activation-{label}-{}",
                uuid::Uuid::new_v4()
            )),
            environment: "test".to_string(),
            http_response_max_bytes: 1024,
            http_egress_proxy: None,
        })
        .expect("activation cancellation host");
        let (client_io, server_io) = duplex(1 << 20);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let router = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        let session_task = tokio::spawn(super::super::run_connected_session(
            host.clone(),
            client,
            format!("skiff-router-session-v1:opaque:activation-{label}"),
        ));
        let session = Self {
            host,
            provider,
            runtime_id,
            assembly: assembly_ref,
            config_snapshot: config_snapshot_ref,
            websocket_ingress,
            artifact_root,
            router,
            session_task,
        };
        session
    }

    async fn bootstrap(&mut self) {
        let bootstrap = encode_binary_frame(
            &json!({
                "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
                "type": "router.bootstrap",
                "artifactsPath": self.artifact_root,
                "serviceDb": { "mongoUrl": "mongodb://activation-cancel" },
                "activation": {
                    "environment": "test",
                    "generation": 1,
                    "assembly": self.assembly,
                    "configSnapshot": self.config_snapshot,
                },
                "http": { "maxResponseBytes": 1024 }
            }),
            &[],
        )
        .expect("bootstrap frame");
        self.router
            .send(Message::Binary(bootstrap.into()))
            .await
            .expect("send bootstrap");
        timeout(Duration::from_secs(10), self.router.next())
            .await
            .expect("bootstrap registration timeout")
            .expect("bootstrap registration frame")
            .expect("valid bootstrap registration frame");
        assert_eq!(
            self.host
                .active_runtime_assembly()
                .unwrap()
                .unwrap()
                .generation(),
            1
        );
    }

    fn prepare(&self, activation_id: &str) -> AssemblyActivationControl {
        AssemblyActivationControl::Prepare {
            environment: "test".to_string(),
            activation_id: activation_id.to_string(),
            expected_generation: 1,
            candidate_generation: 2,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
            replica_id: self.runtime_id.clone(),
            service_db: None,
        }
    }

    async fn send_activation(&mut self, control: &AssemblyActivationControl) {
        let frame = encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RouterToRuntime,
            control,
        )
        .expect("activation frame");
        self.router
            .send(Message::Binary(frame.into()))
            .await
            .expect("send activation frame");
    }

    async fn ranked_websocket_connect(
        &mut self,
    ) -> skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketConnectResponseEndFrameHeader
    {
        let route = self
            .host
            .lookup_active_assembly_request_route(&self.websocket_ingress)
            .expect("active activation fixture WebSocket route");
        let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
            &route.entry().owner().service_id,
            route.gateway_entry_key(),
        )
        .expect("activation fixture WebSocket entry identity");
        let request = RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: "ranked-connect-during-prepare".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: route.assembly_identity().clone(),
                assembly_generation: route.generation(),
                deployment: route.deployment().clone(),
                gateway_entry_identity: route.gateway_entry_identity().clone(),
                ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                    method: (),
                    path: route.selector().path.clone(),
                },
            },
            client_session: None,
            deadline: None,
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: "trace-ranked-connect-during-prepare".to_string(),
                span_id: "span-ranked-connect-during-prepare".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
                connection_id: "ranked-connect-during-prepare".to_string(),
                url: "ws://activation.test/socket".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
                cookies: Vec::new(),
                version: None,
                websocket_entry_id,
                gateway_entry_identity: route.gateway_entry_identity().clone(),
            },
            test_effects_enabled: false,
        };
        let frame = encode_binary_frame(&request, &[]).expect("ranked websocket connect frame");
        self.router
            .send(Message::Binary(frame.into()))
            .await
            .expect("send ranked websocket connect during Prepare");

        timeout(Duration::from_secs(10), async {
            let acquire = loop {
                let message = self
                    .router
                    .next()
                    .await
                    .expect("runtime websocket remains open")
                    .expect("valid runtime websocket frame");
                let Message::Binary(frame) = message else {
                    continue;
                };
                let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&frame)
                    .expect("runtime binary frame");
                if typed.envelope_type == WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE {
                    break decode_websocket_generation_lifecycle_frame(
                        WebSocketGenerationLifecycleDirection::RuntimeToRouter,
                        &frame,
                    )
                    .expect("ranked connect generation acquire");
                }
                if typed.envelope_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                    let control = decode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &frame,
                    )
                    .expect("runtime activation frame");
                    assert!(
                        matches!(control, AssemblyActivationControl::Register { generation: 1, .. }),
                        "blocked Prepare must not emit a terminal before its provider is released: {control:?}"
                    );
                }
            };
            let WebSocketGenerationLifecycleControl::Acquire {
                request_id, tuple, ..
            } = acquire
            else {
                panic!("ranked connect must acquire its generation before acceptance")
            };
            let ack = WebSocketGenerationLifecycleControl::Ack {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                request_id,
                sender: WebSocketGenerationLifecycleSender::Router,
                tuple,
            };
            let ack = encode_websocket_generation_lifecycle_frame(
                WebSocketGenerationLifecycleDirection::RouterToRuntime,
                &ack,
            )
            .expect("ranked connect generation acquire acknowledgement");
            self.router
                .send(Message::Binary(ack.into()))
                .await
                .expect("send ranked connect generation acquire acknowledgement");

            loop {
                let message = self
                    .router
                    .next()
                    .await
                    .expect("runtime websocket remains open")
                    .expect("valid runtime websocket frame");
                let Message::Binary(frame) = message else {
                    continue;
                };
                let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&frame)
                    .expect("runtime binary frame");
                if typed.envelope_type == "response.end" {
                    return decode_runtime_assembly_websocket_connect_response_end_frame(&frame)
                        .expect("ranked websocket connect canonical response wire");
                }
                if typed.envelope_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                    let control = decode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &frame,
                    )
                    .expect("runtime activation frame");
                    assert!(
                        matches!(control, AssemblyActivationControl::Register { generation: 1, .. }),
                        "blocked Prepare must not emit a terminal while ranked output is pending: {control:?}"
                    );
                }
            }
        })
        .await
        .expect("ranked websocket connect must complete while Prepare remains blocked")
    }

    async fn wait_for_provider_start(&self, context: &str) {
        timeout(Duration::from_secs(10), self.provider.started.notified())
            .await
            .unwrap_or_else(|_| panic!("{context}"));
    }

    async fn recv_activation(&mut self, context: &str) -> AssemblyActivationControl {
        timeout(Duration::from_secs(10), async {
            loop {
                let message = self
                    .router
                    .next()
                    .await
                    .expect("runtime websocket remains open")
                    .expect("valid runtime websocket frame");
                if let Message::Binary(frame) = message {
                    let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&frame)
                        .expect("runtime binary frame");
                    if typed.envelope_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                        let control = decode_assembly_activation_frame(
                            AssemblyActivationFrameDirection::RuntimeToRouter,
                            &frame,
                        )
                        .expect("runtime activation frame");
                        if matches!(
                            control,
                            AssemblyActivationControl::Register { generation: 1, .. }
                        ) {
                            continue;
                        }
                        return control;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{context}"))
    }

    async fn ping_without_activation_terminal(&mut self, payload: &[u8], activation_id: &str) {
        self.send_ping(payload).await;
        self.expect_pong_without_activation_terminal(payload, activation_id)
            .await;
    }

    async fn send_ping(&mut self, payload: &[u8]) {
        self.router
            .send(Message::Ping(payload.to_vec().into()))
            .await
            .expect("send Router ping");
    }

    async fn expect_pong_without_activation_terminal(
        &mut self,
        payload: &[u8],
        activation_id: &str,
    ) {
        timeout(Duration::from_millis(100), async {
            loop {
                match self
                    .router
                    .next()
                    .await
                    .expect("runtime websocket remains open")
                    .expect("valid runtime websocket frame")
                {
                    Message::Pong(actual) if actual.as_ref() == payload => return,
                    Message::Binary(frame) => {
                        let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&frame)
                            .expect("runtime binary frame");
                        if typed.envelope_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                            let control = decode_assembly_activation_frame(
                                AssemblyActivationFrameDirection::RuntimeToRouter,
                                &frame,
                            )
                            .expect("runtime activation frame");
                            if matches!(
                                control,
                                AssemblyActivationControl::Prepared { activation_id: ref id, .. }
                                    | AssemblyActivationControl::Reject { activation_id: ref id, .. }
                                    if id == activation_id
                            ) {
                                panic!(
                                    "cancelled activation emitted a late terminal: {control:?}"
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("runtime reader must answer ping within 100ms");
    }

    async fn end_transport(&mut self) {
        self.router
            .get_mut()
            .shutdown()
            .await
            .expect("shutdown activation test transport");
    }

    async fn wait_for_session(&mut self, context: &str) -> crate::error::Result<()> {
        timeout(Duration::from_secs(10), &mut self.session_task)
            .await
            .unwrap_or_else(|_| panic!("{context}"))
            .expect("session task")
    }

    async fn close(&mut self) -> crate::error::Result<()> {
        self.router
            .send(Message::Close(None))
            .await
            .expect("close activation test session");
        self.wait_for_session("session close cleanup").await
    }
}

fn activation_abort(prepare: &AssemblyActivationControl) -> AssemblyActivationControl {
    let AssemblyActivationControl::Prepare {
        environment,
        activation_id,
        expected_generation,
        candidate_generation,
        assembly,
        config_snapshot,
        replica_id,
        ..
    } = prepare
    else {
        panic!("test abort requires Prepare");
    };
    AssemblyActivationControl::Abort {
        environment: environment.clone(),
        activation_id: activation_id.clone(),
        expected_generation: *expected_generation,
        candidate_generation: *candidate_generation,
        assembly: assembly.clone(),
        config_snapshot: config_snapshot.clone(),
        replica_id: replica_id.clone(),
    }
}

fn activation_commit(prepare: &AssemblyActivationControl) -> AssemblyActivationControl {
    let AssemblyActivationControl::Prepare {
        environment,
        activation_id,
        expected_generation,
        candidate_generation,
        assembly,
        config_snapshot,
        replica_id,
        ..
    } = prepare
    else {
        panic!("test commit requires Prepare");
    };
    AssemblyActivationControl::Commit {
        environment: environment.clone(),
        activation_id: activation_id.clone(),
        expected_generation: *expected_generation,
        candidate_generation: *candidate_generation,
        assembly: assembly.clone(),
        config_snapshot: config_snapshot.clone(),
        replica_id: replica_id.clone(),
        service_db: None,
    }
}

fn assert_clean_cancelled_activation(host: &RuntimeHost) {
    let health = host
        .runtime_assembly_admission_health()
        .expect("activation health");
    assert!(health.candidate.is_none());
    assert!(health.last_outcome.is_none());
    assert_eq!(health.active_generation, Some(1));
}

fn assert_no_activation_candidate(host: &RuntimeHost) {
    let health = host
        .runtime_assembly_admission_health()
        .expect("activation health");
    assert!(health.candidate.is_none());
    assert_eq!(health.active_generation, Some(1));
}
