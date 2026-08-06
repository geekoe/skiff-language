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
        decode_typed_binary_frame, encode_binary_frame, RuntimeRegisteredFrameHeader,
        TypedEnvelope, RUNTIME_FRAME_SCHEMA_VERSION,
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
async fn prepare_acks_immediately_without_loading_or_materialization() {
    let mut session = ActivationSession::start("ack-only-prepare").await;
    // Blocking provider: any accidental candidate build or DB provisioning
    // would hang before the ACK. M2 Prepare is wire compatibility only.
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-ack-only");
    session.send_activation(&prepare).await;
    let prepared = session.recv_activation("Prepared ACK").await;
    assert!(
        matches!(
            &prepared,
            AssemblyActivationControl::Prepared { activation_id, .. }
                if activation_id == "activation-ack-only"
        ),
        "unexpected Prepare reply: {prepared:?}"
    );
    assert_eq!(
        session.provider.starts.load(Ordering::Acquire),
        0,
        "Prepare must not trigger any loading or materialization"
    );
    assert_clean_cancelled_activation(&session.host);
    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn commit_registers_tuple_and_refreshes_capabilities_without_loading() {
    let mut session = ActivationSession::start("ack-only-commit").await;
    // Blocking provider: any accidental materialization would hang before the
    // Register ACK. M2 Commit only records the committed tuple metadata.
    session.provider.blocking.store(true, Ordering::Release);
    let prepare = session.prepare("activation-ack-only-commit");
    session.send_activation(&prepare).await;
    let prepared = session.recv_activation("Prepared ACK").await;
    assert!(matches!(prepared, AssemblyActivationControl::Prepared { .. }));
    session.send_activation(&activation_commit(&prepare)).await;
    let register = session.recv_activation("Register ACK").await;
    assert!(
        matches!(
            &register,
            AssemblyActivationControl::Register { generation: 2, .. }
        ),
        "Commit must answer with the exact Register tuple: {register:?}"
    );
    let registration = session
        .host
        .active_assembly_registration()
        .expect("registration")
        .expect("committed tuple must be recorded");
    assert!(
        matches!(
            registration,
            AssemblyActivationControl::Register { generation: 2, .. }
        ),
        "committed metadata must track the committed tuple"
    );
    assert_eq!(
        session.provider.starts.load(Ordering::Acquire),
        0,
        "Commit must not trigger any loading or materialization"
    );
    assert_clean_cancelled_activation(&session.host);
    session.close().await.expect("clean Router close");
}

#[tokio::test]
async fn abort_without_pending_prepare_is_an_idempotent_noop() {
    let mut session = ActivationSession::start("abort-noop").await;
    let prepare = session.prepare("activation-abort-noop");
    session.send_activation(&activation_abort(&prepare)).await;
    session
        .ping_without_activation_terminal(b"after-abort", "activation-abort-noop")
        .await;
    assert_clean_cancelled_activation(&session.host);
    session.close().await.expect("clean Router close");
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
    assert!(
        error.to_string().contains("runtime handshake terminal")
            && error.to_string().contains("WrongOrder"),
        "activation before bootstrap must be a strict handshake terminal: {error:?}"
    );

    let mut foreign_profile = ActivationSession::start("foreign-profile").await;
    let mut prepare = foreign_profile.prepare("activation-foreign-profile");
    let AssemblyActivationControl::Prepare { profile, .. } = &mut prepare else {
        unreachable!();
    };
    *profile = "prod".to_string();
    foreign_profile.send_activation(&prepare).await;
    let error = foreign_profile
        .wait_for_session("foreign activation profile")
        .await
        .expect_err("foreign activation profile must fail");
    assert!(error
        .to_string()
        .contains("does not match Runtime frozen profile"));
    assert_no_activation_candidate(&foreign_profile.host);

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
        .contains("serviceDb is not supported"));
    assert_no_activation_candidate(&transient_service_db.host);
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
            profile: "test".to_string(),
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
                    "profile": "test",
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
        // H-registration-cut: the Runtime must send capabilities and
        // assembly.activation:Register before it is bound, and only the
        // registered ACK transitions it to Registered.
        let register = timeout(Duration::from_secs(10), async {
            let mut saw_capabilities = false;
            loop {
                let message = self
                    .router
                    .next()
                    .await
                    .expect("bootstrap registration frame")
                    .expect("valid bootstrap registration frame");
                let Message::Binary(frame) = message else {
                    continue;
                };
                let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(&frame)
                    .expect("runtime binary frame during bootstrap");
                if typed.envelope_type == "runtime.capabilities" {
                    saw_capabilities = true;
                    continue;
                }
                if typed.envelope_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                    let control = decode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &frame,
                    )
                    .expect("runtime registration frame during bootstrap");
                    assert!(
                        matches!(control, AssemblyActivationControl::Register { .. }),
                        "bootstrap must be answered with assembly.activation:Register"
                    );
                    assert!(
                        saw_capabilities,
                        "runtime.capabilities must precede assembly.activation:Register"
                    );
                    return frame;
                }
            }
        })
        .await
        .expect("bootstrap registration timeout");
        let ack = encode_binary_frame(
            &RuntimeRegisteredFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "runtime.registered".to_string(),
                runtime_id: self.runtime_id.clone(),
            },
            &[],
        )
        .expect("registered ACK frame");
        self.router
            .send(Message::Binary(ack.into()))
            .await
            .expect("send registered ACK");
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
            profile: "test".to_string(),
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
        profile,
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
        profile: profile.clone(),
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
        profile,
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
        profile: profile.clone(),
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
    assert_eq!(health.active_generation, Some(1));
}

fn assert_no_activation_candidate(host: &RuntimeHost) {
    let health = host
        .runtime_assembly_admission_health()
        .expect("activation health");
    assert!(health.candidate.is_none());
    assert_eq!(health.active_generation, Some(1));
}
