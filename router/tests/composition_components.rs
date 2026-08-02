//! W-composition component tests: `HttpDispatchPort` ↔ `RequestDispatcher`
//! adapter (contract conversion, reject mapping, unary/stream round-trip,
//! timeout/cancel), request-family sink, activation enqueue port, WS
//! responder and the actor outbound control ports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, DeploymentArtifactIdentity, DeploymentRevision,
    GatewayEntryIdentity, RuntimeAssembly, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
    ServiceDeploymentRef,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::activation::SessionEnqueuePort;
use skiff_router::actor::{ActivationControlPort, IdleEvictControlPort};
use skiff_router::bootstrap::RoutingEpoch;
use skiff_router::dispatch::{
    CandidateViewSource, LeaseRevalidate, RevalidateOutcome, RoutingEpochSource,
    RuntimeDispatcherOptions, RuntimePeer, SessionAbortControl,
};
use skiff_router::http::dispatch::{
    cancel_channel, DispatchRequest, HttpDispatchError, HttpDispatchPort, PendingTerminalSource,
};
use skiff_router::http::stream::HttpStreamSink;
use skiff_router::routing::{
    CandidateDirectoryView, DispatchCapabilities, RegisteredSessionLease, RuntimeCandidateQuery,
};
use skiff_router::session::consumer::{ConsumerKind, ConsumerManifest};
use skiff_router::session::demux::InboundFrameSink;
use skiff_router::session::directory::RuntimeRegistrationDirectory;
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::layer::{SessionLayer, SessionLayerOptions};
use skiff_router::supervisor::http::{
    dispatch_submit_from_request, DispatcherHttpPort, PendingHttpRouter, RequestFrameSink,
};
use skiff_router::supervisor::session_ports::{
    ActivationSessionEnqueuePort, SessionHandle, WsRuntimeResponder,
};
use skiff_router::supervisor::sinks::ActivationTransactionSink;
use skiff_router::ws::BrokerRuntimeResponse;
use skiff_router::ws::RuntimeResponder;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::connection_protocol::{
    decode_connection_response_frame, ConnectionResponseOutcome,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, ResponseChunkFrameHeader, ResponseEndFrameHeader,
    ResponseEndFrameMetadata, ResponseStartFrameHeader, RuntimeHttpNameValueFrameHeader,
    RuntimeHttpResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
    RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestRoutingFrameHeader,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
};
fn assembly_identity() -> AssemblyIdentity {
    AssemblyIdentity::new(
        "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
}

fn deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/docs".to_string(),
        contract_version: "example.com/docs@1".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-deployment-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
    }
}

fn snapshot_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .expect("snapshot id"),
    }
}

fn epoch() -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: "skiff-runtime-assembly-v3".to_string(),
        assembly_identity: assembly_identity(),
        roots: Vec::new(),
        resolved_deployments: vec![deployment()],
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
    let snapshot =
        RuntimeConfigSnapshot::new("prod", snapshot_ref(), Vec::new()).expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog =
        skiff_router::artifact::ActorRoutingCatalog::from_projection(Arc::new(projection));
    Arc::new(
        RoutingEpoch::new(
            "prod",
            7,
            Arc::new(assembly),
            Arc::new(snapshot),
            Arc::new(catalog),
        )
        .expect("epoch"),
    )
}

fn manifest() -> ConsumerManifest {
    ConsumerManifest::installed([
        ConsumerKind::HealthLedger,
        ConsumerKind::RequestDispatcher,
        ConsumerKind::RuntimeGenerationPinLedger,
        ConsumerKind::WebSocketRequestBroker,
        ConsumerKind::ActivationCoordinator,
    ])
}

fn registered_directory() -> (RuntimeRegistrationDirectory, RuntimeSessionEpoch) {
    let manifest = manifest();
    let mut directory = RuntimeRegistrationDirectory::new(&manifest);
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    let tuple = epoch().registered_tuple();
    directory
        .publish_pending(
            &session,
            tuple,
            &[
                ConsumerKind::HealthLedger,
                ConsumerKind::RequestDispatcher,
                ConsumerKind::RuntimeGenerationPinLedger,
                ConsumerKind::WebSocketRequestBroker,
                ConsumerKind::ActivationCoordinator,
            ],
        )
        .expect("publish pending");
    assert!(directory.mark_registered(&session));
    (directory, session)
}

#[derive(Debug, Clone)]
struct DirectoryViewSource {
    directory: Arc<Mutex<RuntimeRegistrationDirectory>>,
}

impl CandidateViewSource for DirectoryViewSource {
    fn view(&self) -> CandidateDirectoryView {
        let directory = self.directory.lock().unwrap();
        let capabilities = HashMap::from([(
            RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            },
            DispatchCapabilities {
                unary: true,
                server_stream: true,
            },
        )]);
        RuntimeCandidateQuery::snapshot_directory_view(&directory, &capabilities)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct OkRevalidate;

impl LeaseRevalidate for OkRevalidate {
    fn revalidate(&self, _request_id: &str, _lease: &RegisteredSessionLease) -> RevalidateOutcome {
        RevalidateOutcome::Ok
    }
}

#[derive(Debug, Clone)]
struct SomeEpochSource {
    epoch: std::sync::Arc<RoutingEpoch>,
}

impl RoutingEpochSource for SomeEpochSource {
    fn capture(&self) -> Option<Arc<RoutingEpoch>> {
        Some(self.epoch.clone())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NoneEpochSource;

impl RoutingEpochSource for NoneEpochSource {
    fn capture(&self) -> Option<Arc<RoutingEpoch>> {
        None
    }
}

type PeerStart = (RuntimeSessionEpoch, Vec<u8>);

#[derive(Debug, Clone, Default)]
struct RecordingPeer {
    starts: Arc<Mutex<Vec<PeerStart>>>,
    cancels: Arc<Mutex<Vec<(RuntimeSessionEpoch, String, String)>>>,
}

impl RuntimePeer for RecordingPeer {
    fn send_request_start(
        &self,
        session: &RuntimeSessionEpoch,
        request: &skiff_router::dispatch::DispatchSubmit,
    ) -> Result<(), String> {
        let bytes = encode_binary_frame(&request.header, &request.payload_bytes)
            .map_err(|error| error.to_string())?;
        self.starts.lock().unwrap().push((session.clone(), bytes));
        Ok(())
    }

    fn send_request_cancel(
        &self,
        session: &RuntimeSessionEpoch,
        request_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        self.cancels.lock().unwrap().push((
            session.clone(),
            request_id.to_string(),
            reason.to_string(),
        ));
        Ok(())
    }

    fn send_spawn_submit(
        &self,
        _session: &RuntimeSessionEpoch,
        _spawn: &skiff_router::dispatch::SpawnSubmit,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct RecordingAbort {
    aborts: Arc<Mutex<Vec<RuntimeSessionEpoch>>>,
}

impl SessionAbortControl for RecordingAbort {
    fn abort_session(&self, session: &RuntimeSessionEpoch) {
        self.aborts.lock().unwrap().push(session.clone());
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct NoActorSpawnControl;

impl skiff_router::dispatch::ActorMethodSpawnControl for NoActorSpawnControl {
    fn is_active_invocation_parent(&self, _caller_request_id: &str) -> bool {
        false
    }

    fn submit_spawn(&self, _spawn: skiff_router::dispatch::ActorMethodSpawnDispatch) {}
}

fn dispatcher_pair(
    directory: Arc<Mutex<RuntimeRegistrationDirectory>>,
    epoch: Arc<RoutingEpoch>,
) -> (
    Arc<skiff_router::dispatch::RequestDispatcher>,
    Arc<RecordingPeer>,
) {
    let peer = Arc::new(RecordingPeer::default());
    let dispatcher = Arc::new(
        skiff_router::dispatch::RequestDispatcher::new(
            RuntimeDispatcherOptions::new(
                4,
                Arc::new(SomeEpochSource {
                    epoch: epoch.clone(),
                }),
                Arc::new(DirectoryViewSource { directory }),
                Arc::new(OkRevalidate),
                Arc::clone(&peer) as Arc<dyn RuntimePeer>,
                Arc::new(RecordingAbort::default()) as Arc<dyn SessionAbortControl>,
                Arc::new(NoActorSpawnControl),
            )
            .expect("dispatcher options"),
        )
        .expect("dispatcher"),
    );
    (dispatcher, peer)
}

fn request_start_header(request_id: &str, mode: &str) -> RuntimeAssemblyRequestStartFrameHeader {
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: mode.to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: assembly_identity(),
            assembly_generation: 7,
            deployment: deployment(),
            gateway_entry_identity: GatewayEntryIdentity::parse(
                "skiff-gateway-entry-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            )
            .expect("gateway entry identity"),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: "GET".to_string(),
                path: "/docs".to_string(),
            },
        },
        client_session: None,
        deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms: 5000,
            expires_at: "2026-08-02T00:00:00Z".to_string(),
        }),
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: "trace-1".to_string(),
            span_id: "span-1".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method: "GET".to_string(),
            url: "http://127.0.0.1/docs".to_string(),
            path: "/docs".to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    }
}

fn http_request(request_id: &str) -> DispatchRequest {
    let (_, cancel_watch) = cancel_channel();
    DispatchRequest {
        header: request_start_header(request_id, "unary"),
        payload_bytes: Bytes::from_static(b"payload"),
        timeout: Duration::from_secs(5),
        client_disconnect: cancel_watch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_submit_conversion_preserves_header_payload_and_no_preference() {
        let request = http_request("req-convert");
        let submit = dispatch_submit_from_request(&request);
        assert_eq!(submit.header, request.header);
        assert_eq!(submit.payload_bytes, request.payload_bytes.to_vec());
        assert!(submit.prefer_session.is_none());
        assert_eq!(submit.request_id(), "req-convert");
    }

    #[tokio::test]
    async fn dispatcher_http_port_rejects_with_no_epoch() {
        let (directory, _) = registered_directory();
        let peer = Arc::new(RecordingPeer::default());
        let dispatcher = Arc::new(
            skiff_router::dispatch::RequestDispatcher::new(
                RuntimeDispatcherOptions::new(
                    4,
                    Arc::new(NoneEpochSource),
                    Arc::new(DirectoryViewSource {
                        directory: Arc::new(Mutex::new(directory)),
                    }),
                    Arc::new(OkRevalidate),
                    Arc::clone(&peer) as Arc<dyn RuntimePeer>,
                    Arc::new(RecordingAbort::default()) as Arc<dyn SessionAbortControl>,
                    Arc::new(NoActorSpawnControl),
                )
                .expect("dispatcher options"),
            )
            .expect("dispatcher"),
        );
        let adapter = DispatcherHttpPort::new(
            dispatcher,
            Arc::new(PendingHttpRouter::new()),
            Duration::from_secs(5),
        );
        let error = adapter
            .dispatch_unary(http_request("req-no-epoch"))
            .await
            .expect_err("no epoch must fail closed");
        match error {
            HttpDispatchError::Control {
                status: Some(503), ..
            } => {}
            other => panic!("expected 503 control error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatcher_http_port_unary_round_trip_through_request_sink() {
        let (directory, session) = registered_directory();
        let (dispatcher, peer) = dispatcher_pair(Arc::new(Mutex::new(directory)), epoch());
        let router = Arc::new(PendingHttpRouter::new());
        let sink = Arc::new(RequestFrameSink::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
        ));
        let adapter = DispatcherHttpPort::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
            Duration::from_secs(5),
        );
        let (signal, cancel_watch) = cancel_channel();
        let request = DispatchRequest {
            header: request_start_header("req-unary", "unary"),
            payload_bytes: Bytes::from_static(b"payload"),
            timeout: Duration::from_secs(5),
            client_disconnect: cancel_watch,
        };
        let handle = tokio::spawn(async move { adapter.dispatch_unary(request).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while dispatcher.pending_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("request becomes pending");
        // Runtime response.end with HTTP metadata + payload.
        let header = ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.end".to_string(),
            request_id: "req-unary".to_string(),
            payload_present: true,
            metadata: ResponseEndFrameMetadata::Http(RuntimeHttpResponseFrameHeader {
                status: 200,
                headers: vec![RuntimeHttpNameValueFrameHeader {
                    name: "content-type".to_string(),
                    value: "text/plain".to_string(),
                }],
            }),
        };
        let bytes = encode_binary_frame(&header, b"hello".as_slice()).expect("encode response.end");
        sink.handle(&session, &bytes)
            .expect("sink accepts response.end");

        let response = handle
            .await
            .expect("dispatch task")
            .expect("unary response");
        drop(signal);
        assert_eq!(response.status, 200);
        assert_eq!(response.payload, Bytes::from_static(b"hello"));
        assert_eq!(response.headers.len(), 1);
        assert_eq!(dispatcher.pending_count(), 0);
        assert_eq!(peer.starts.lock().unwrap().len(), 1);
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingStreamSink {
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl HttpStreamSink for RecordingStreamSink {
        async fn enqueue_start(
            &self,
            _http_response: RuntimeHttpResponseFrameHeader,
        ) -> Result<(), skiff_router::http::stream::HttpStreamError> {
            self.events.lock().unwrap().push("start".to_string());
            Ok(())
        }

        async fn enqueue_chunk(
            &self,
            _payload: Bytes,
        ) -> Result<(), skiff_router::http::stream::HttpStreamError> {
            self.events.lock().unwrap().push("chunk".to_string());
            Ok(())
        }

        async fn enqueue_end(&self) -> Result<(), skiff_router::http::stream::HttpStreamError> {
            self.events.lock().unwrap().push("end".to_string());
            Ok(())
        }

        fn close(&self) {}
    }

    #[tokio::test]
    async fn dispatcher_http_port_stream_round_trip_through_request_sink() {
        let (directory, session) = registered_directory();
        let (dispatcher, _peer) = dispatcher_pair(Arc::new(Mutex::new(directory)), epoch());
        let router = Arc::new(PendingHttpRouter::new());
        let sink = Arc::new(RequestFrameSink::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
        ));
        let adapter = DispatcherHttpPort::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
            Duration::from_secs(5),
        );
        let stream_sink = Arc::new(RecordingStreamSink::default());
        let (signal, cancel_watch) = cancel_channel();
        let request = DispatchRequest {
            header: request_start_header("req-stream", "serverStream"),
            payload_bytes: Bytes::from_static(b"payload"),
            timeout: Duration::from_secs(5),
            client_disconnect: cancel_watch,
        };
        let handle = tokio::spawn({
            let adapter = adapter;
            let stream_sink = Arc::clone(&stream_sink);
            async move { adapter.dispatch_stream(request, stream_sink).await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while dispatcher.pending_count() == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("stream request becomes pending");

        let start = ResponseStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.start".to_string(),
            request_id: "req-stream".to_string(),
            http_response: RuntimeHttpResponseFrameHeader {
                status: 200,
                headers: Vec::new(),
            },
        };
        sink.handle(
            &session,
            &encode_binary_frame(&start, &[]).expect("encode response.start"),
        )
        .expect("sink accepts response.start");
        let chunk = ResponseChunkFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.chunk".to_string(),
            request_id: "req-stream".to_string(),
            seq: 0,
        };
        sink.handle(
            &session,
            &encode_binary_frame(&chunk, b"chunk".as_slice()).expect("encode response.chunk"),
        )
        .expect("sink accepts response.chunk");
        let end = ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.end".to_string(),
            request_id: "req-stream".to_string(),
            payload_present: false,
            metadata: ResponseEndFrameMetadata::None,
        };
        sink.handle(
            &session,
            &encode_binary_frame(&end, &[]).expect("encode response.end"),
        )
        .expect("sink accepts response.end");

        handle.await.expect("stream task").expect("stream dispatch");
        drop(signal);
        assert_eq!(
            stream_sink.events.lock().unwrap().clone(),
            vec!["start".to_string(), "chunk".to_string(), "end".to_string()]
        );
        assert_eq!(dispatcher.pending_count(), 0);
    }

    #[tokio::test]
    async fn dispatcher_http_port_client_disconnect_terminates_pending() {
        let (directory, session) = registered_directory();
        let (dispatcher, peer) = dispatcher_pair(Arc::new(Mutex::new(directory)), epoch());
        let router = Arc::new(PendingHttpRouter::new());
        let _sink = Arc::new(RequestFrameSink::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
        ));
        let adapter = DispatcherHttpPort::new(
            Arc::clone(&dispatcher),
            Arc::clone(&router),
            Duration::from_secs(5),
        );
        let (signal, cancel_watch) = cancel_channel();
        let request = DispatchRequest {
            header: request_start_header("req-cancel", "unary"),
            payload_bytes: Bytes::new(),
            timeout: Duration::from_secs(5),
            client_disconnect: cancel_watch,
        };
        let handle = tokio::spawn(async move { adapter.dispatch_unary(request).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal
            .cancel(skiff_runtime_transport::cancel_reason::RequestCancelReason::ClientDisconnect);

        let error = handle
            .await
            .expect("dispatch task")
            .expect_err("cancel must fail closed");
        assert!(matches!(
            error,
            HttpDispatchError::Cancelled {
                source: PendingTerminalSource::ClientDisconnect,
                ..
            }
        ));
        assert_eq!(dispatcher.pending_count(), 0);
        let cancels = peer.cancels.lock().unwrap().clone();
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].2, "client_disconnect");
        assert_eq!(router.pending_count(), 0);
        let _ = session;
    }

    fn session_layer_with_writer() -> (
        Arc<SessionLayer>,
        SessionHandle,
        Arc<RecordingWriter>,
        RuntimeSessionEpoch,
    ) {
        let config = RouterConfigFixture::config();
        let manifest = manifest();
        let consumers: Vec<Arc<dyn skiff_router::session::SessionConsumer>> = vec![
            Arc::new(skiff_router::session::health::RuntimeHealthLedger::new()),
            Arc::new(GenericConsumer(ConsumerKind::RequestDispatcher)),
            Arc::new(GenericConsumer(ConsumerKind::RuntimeGenerationPinLedger)),
            Arc::new(GenericConsumer(ConsumerKind::WebSocketRequestBroker)),
            Arc::new(GenericConsumer(ConsumerKind::ActivationCoordinator)),
        ];
        let layer = Arc::new(
            SessionLayer::with_options(
                config,
                SessionLayerOptions {
                    committed_epoch: None,
                    pending_epoch: None,
                    manifest,
                    consumers,
                    timing: Default::default(),
                    budgets: Default::default(),
                    writer_delay: None,
                },
            )
            .expect("composition session layer"),
        );
        let handle = SessionHandle::new();
        handle.set(Arc::clone(&layer));
        let session = RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        };
        let writer = Arc::new(RecordingWriter::new());
        layer.register_frame_writer(&session, writer.clone());
        (layer, handle, writer, session)
    }

    #[derive(Debug, Clone, Copy)]
    struct GenericConsumer(ConsumerKind);

    impl skiff_router::session::SessionConsumer for GenericConsumer {
        fn kind(&self) -> ConsumerKind {
            self.0
        }

        fn on_session_closed(&self, _session: &RuntimeSessionEpoch) -> Result<(), String> {
            Ok(())
        }
    }

    struct RouterConfigFixture;

    impl RouterConfigFixture {
        fn config() -> skiff_router::config::RouterConfig {
            skiff_router::config::RouterConfig {
                activation_prepare_timeout_ms: 120_000,
                artifacts_path: "/opt/skiff/artifacts".into(),
                dev_reload: None,
                environment: Some("prod".to_string()),
                host: "127.0.0.1".to_string(),
                http_max_request_bytes: 1,
                http_max_response_bytes: 8_388_608,
                http_port: 0,
                manifests: vec![],
                profile: "dev".to_string(),
                release_mode: None,
                request_timeout_ms: 20_000,
                rewrite: vec![],
                runtime_path: "/runtime".to_string(),
                runtime_port: 0,
                runtime_max_concurrency: 4,
                file_backend: None,
                service_db: skiff_router::config::ServiceDbConfig {
                    mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
                },
                telemetry: None,
                websocket_path: "/ws".to_string(),
            }
        }
    }

    #[tokio::test]
    async fn activation_enqueue_port_encodes_prepare_and_fails_closed_on_queue_full() {
        let (_layer, handle, writer, session) = session_layer_with_writer();
        let port = ActivationSessionEnqueuePort::new(handle);
        let control = skiff_artifact_model::AssemblyActivationControl::Prepare {
            environment: "prod".to_string(),
            activation_id: "activation-1".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: RuntimeAssemblyRef {
                assembly_identity: assembly_identity(),
            },
            config_snapshot: snapshot_ref(),
            replica_id: "runtime-a".to_string(),
            service_db: None,
        };
        let binding = skiff_router::activation::ActivationParticipantBinding {
            replica_id: "runtime-a".to_string(),
            session_epoch: session.clone(),
        };
        assert_eq!(
            port.enqueue_prepare(&binding, &control),
            skiff_router::activation::EnqueueResult::Ok
        );
        let bytes = writer.recorded().pop().expect("prepare frame written");
        let decoded = decode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RouterToRuntime,
            &bytes,
        )
        .expect("prepare frame must decode");
        assert!(
            matches!(decoded, skiff_artifact_model::AssemblyActivationControl::Prepare { activation_id, .. } if activation_id == "activation-1")
        );

        writer.fail_next();
        assert_eq!(
            port.enqueue_prepare(&binding, &control),
            skiff_router::activation::EnqueueResult::QueueFull
        );
    }

    #[tokio::test]
    async fn ws_runtime_responder_encodes_connection_response() {
        let (_layer, handle, writer, session) = session_layer_with_writer();
        let responder = WsRuntimeResponder::new(handle, session.clone());
        let response = BrokerRuntimeResponse {
            request_id: "peer-1".to_string(),
            outcome: ConnectionResponseOutcome::Success,
            remote: None,
            payload: br#""result""#.to_vec(),
        };
        responder.respond(&response).expect("respond writes frame");
        let bytes = writer.recorded().pop().expect("response frame written");
        let (header, payload) = decode_connection_response_frame(&bytes).expect("decode response");
        assert_eq!(header.request_id, "peer-1");
        assert_eq!(payload, br#""result""#);
    }

    #[tokio::test]
    async fn activation_transaction_sink_delivers_prepared_ack() {
        let (_layer, handle, writer, session) = session_layer_with_writer();
        // The coordinator handle is created with fake ports; a prepared ACK must
        // be accepted into its mailbox (no durable effect is asserted here).
        use skiff_router::activation::{
            ActivationCoordinator, ActivationCoordinatorOptions, ActivationCoordinatorPorts,
            NoopHealthSink,
        };
        let ports = ActivationCoordinatorPorts {
            repository: Arc::new(NoopRepository),
            loader: Arc::new(NoopLoader),
            candidates: Arc::new(NoopCandidates),
            sessions: Arc::new(ActivationSessionEnqueuePort::new(handle.clone())),
            publish: Arc::new(NoopPublish),
            health: Arc::new(NoopHealthSink),
        };
        let coordinator =
            ActivationCoordinator::spawn(ports, ActivationCoordinatorOptions::default());
        let sink = ActivationTransactionSink::new(coordinator.clone());
        let control = skiff_artifact_model::AssemblyActivationControl::Prepared {
            environment: "prod".to_string(),
            activation_id: "activation-1".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: RuntimeAssemblyRef {
                assembly_identity: assembly_identity(),
            },
            config_snapshot: snapshot_ref(),
            replica_id: "runtime-a".to_string(),
        };
        let bytes = encode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            &control,
        )
        .expect("encode prepared");
        sink.handle(&session, &bytes).expect("ack accepted");
        // ACK delivery is async through the mailbox; wait for the coordinator to
        // observe the event by polling health (phase stays idle without a
        // transaction, so just ensure no immediate failure path ran).
        let _ = writer;
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct NoopRepository;

    #[async_trait::async_trait]
    impl skiff_router::activation::ActivationStateRepository for NoopRepository {
        async fn read(
            &self,
            _environment: &str,
        ) -> Result<
            skiff_deployment::activation_state::EnvironmentActivationState,
            skiff_router::activation::RepositoryError,
        > {
            Err(
                skiff_router::activation::error::RepositoryError::CasMismatch {
                    environment: "prod".to_string(),
                    message: "noop".to_string(),
                },
            )
        }
        async fn initialize(
            &self,
            _state: &skiff_deployment::activation_state::EnvironmentActivationState,
        ) -> Result<
            skiff_deployment::activation_state::EnvironmentActivationState,
            skiff_router::activation::RepositoryError,
        > {
            unimplemented!("noop repository")
        }
        async fn prepare(
            &self,
            _input: skiff_router::activation::PrepareInput,
        ) -> Result<
            skiff_deployment::activation_state::EnvironmentActivationState,
            skiff_router::activation::RepositoryError,
        > {
            unimplemented!("noop repository")
        }
        async fn commit(
            &self,
            _input: skiff_router::activation::CommitInput,
        ) -> Result<
            skiff_deployment::activation_state::EnvironmentActivationState,
            skiff_router::activation::RepositoryError,
        > {
            unimplemented!("noop repository")
        }
        async fn abort(
            &self,
            _input: skiff_router::activation::AbortInput,
        ) -> Result<
            skiff_deployment::activation_state::EnvironmentActivationState,
            skiff_router::activation::RepositoryError,
        > {
            unimplemented!("noop repository")
        }
        async fn append_audit(
            &self,
            _event: &skiff_deployment::activation_state::ActivationAuditEvent,
        ) -> Result<(), skiff_router::activation::RepositoryError> {
            unimplemented!("noop repository")
        }
        async fn ensure_indexes(&self) -> Result<(), skiff_router::activation::RepositoryError> {
            Ok(())
        }
        fn health(&self) -> skiff_router::activation::health::ActivationRepositoryHealth {
            skiff_router::activation::health::ActivationRepositoryHealth::default()
        }
        async fn close(&self) -> Result<(), skiff_router::activation::RepositoryError> {
            Ok(())
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct NoopLoader;

    #[async_trait::async_trait]
    impl skiff_router::activation::BlockingLoaderPort for NoopLoader {
        async fn load_candidate(
            &self,
            _refs: &skiff_router::activation::CandidateEpochRefs,
        ) -> Result<Arc<RoutingEpoch>, skiff_router::activation::coordinator::CandidateLoadError>
        {
            unimplemented!("noop loader")
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct NoopCandidates;

    impl skiff_router::activation::RuntimeCandidateQueryPort for NoopCandidates {
        fn freeze(
            &self,
            _environment: &str,
        ) -> Result<
            Vec<RegisteredSessionLease>,
            skiff_router::activation::coordinator::ActivationCandidateError,
        > {
            Ok(Vec::new())
        }

        fn revalidate(
            &self,
            _activation_id: &str,
            _frozen: &[skiff_router::activation::ActivationParticipantBinding],
        ) -> skiff_router::activation::ActivationRevalidateOutcome {
            skiff_router::activation::ActivationRevalidateOutcome::Ok
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct NoopPublish;

    impl skiff_router::activation::PublishCommittedEpochPort for NoopPublish {
        fn publish(&self, _epoch: Arc<RoutingEpoch>) {}
    }

    #[derive(Debug, Clone)]
    struct RecordingWriter {
        inner: Arc<Mutex<Vec<Vec<u8>>>>,
        fail: Arc<Mutex<bool>>,
    }

    impl RecordingWriter {
        fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(Vec::new())),
                fail: Arc::new(Mutex::new(false)),
            }
        }

        fn recorded(&self) -> Vec<Vec<u8>> {
            self.inner.lock().unwrap().clone()
        }

        fn fail_next(&self) {
            *self.fail.lock().unwrap() = true;
        }
    }

    impl skiff_router::session::SessionFrameWriter for RecordingWriter {
        fn enqueue(&self, bytes: Vec<u8>) -> Result<(), String> {
            if *self.fail.lock().unwrap() {
                return Err("injected queue full".to_string());
            }
            self.inner.lock().unwrap().push(bytes);
            Ok(())
        }
    }

    fn actor_key() -> skiff_router::actor::ActorLogicalKey {
        skiff_router::actor::ActorLogicalKey {
            service_id: "example.com/docs".to_string(),
            actor_type_identity: "CounterActor".to_string(),
            actor_id_type_identity: "CounterId".to_string(),
            actor_id_encoding_version: "skiff-actor-id-encoding-v1".to_string(),
            canonical_actor_id_key_bytes_base64: "AQID".to_string(),
            actor_id_hash:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
        }
    }

    fn actor_abi() -> skiff_artifact_model::ActorAbiIdentity {
        skiff_artifact_model::ActorAbiIdentity::new(
        "skiff-actor-abi-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    }

    fn actor_implementation() -> skiff_artifact_model::ActorImplementationIdentity {
        skiff_artifact_model::ActorImplementationIdentity::new(
        "skiff-actor-implementation-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    }

    fn declaration_owner() -> skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader
    {
        skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader {
            unit: skiff_runtime_transport::actor_method::ActorOwnerUnitFrameHeader::Service,
            file: skiff_runtime_transport::actor_method::ActorOwnerFileFrameHeader::FileIrIdentity(
                "file:1".to_string(),
            ),
            actor_symbol: "Counter".to_string(),
        }
    }

    fn register_session_in_layer(
        layer: &SessionLayer,
        session: &RuntimeSessionEpoch,
        tuple: &RegisteredAssemblyTuple,
    ) {
        let mut directory = layer.directory_lock();
        directory
            .publish_pending(
                session,
                tuple.clone(),
                &[
                    ConsumerKind::HealthLedger,
                    ConsumerKind::RequestDispatcher,
                    ConsumerKind::RuntimeGenerationPinLedger,
                    ConsumerKind::WebSocketRequestBroker,
                    ConsumerKind::ActivationCoordinator,
                ],
            )
            .expect("publish pending");
        assert!(directory.mark_registered(session));
    }

    #[tokio::test]
    async fn actor_activation_control_port_builds_activate_initial_wire_frame() {
        use skiff_router::actor::{
            ActivateInitialControlRequest, ActorOwnerRouteAuthority, CommitFenceFacts,
        };
        use skiff_router::supervisor::actor::ActorActivationControlPort;
        use skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader;
        use skiff_runtime_transport::actor_owner::{
            decode_actor_owner_control_frame, ActorOwnerControlOperation,
        };

        let (layer, handle, writer, session) = session_layer_with_writer();
        register_session_in_layer(&layer, &session, &epoch().registered_tuple());
        let registry = Arc::new(skiff_router::actor::ActorOwnershipRegistry::new());
        let key = actor_key();
        let abi = actor_abi();
        let implementation = actor_implementation();
        let declaration = declaration_owner();
        let facts = registry.ensure_present(
            &key,
            abi.clone(),
            implementation.clone(),
            declaration.clone(),
        );
        let port = ActorActivationControlPort::new(handle, Arc::clone(&registry));
        let request = ActivateInitialControlRequest {
            request_id: "control:activate-initial-1".to_string(),
            actor_key: key.clone(),
            facts: CommitFenceFacts {
                actor_abi_identity: abi,
                actor_implementation_identity: implementation,
                declaration_owner: declaration,
            },
            owner_runtime_id: "runtime-a".to_string(),
            owner_connection: "conn-a".to_string(),
            route_authority: ActorOwnerRouteAuthority {
                assembly_identity: assembly_identity().into_string(),
                assembly_generation: 7,
            },
            bootstrap_bytes: b"bootstrap".to_vec(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: 30_000,
                expires_at: "2026-08-02T00:00:00Z".to_string(),
            },
            test_case_capability: None,
            test_case_parent_request_id: None,
        };
        port.send_activate_initial(&request)
            .expect("activateInitial writes");
        let bytes = writer.recorded().pop().expect("activateInitial frame");
        let header = decode_actor_owner_control_frame(&bytes).expect("decode activateInitial");
        assert_eq!(
            header.operation,
            ActorOwnerControlOperation::ActivateInitial
        );
        assert_eq!(header.fence.epoch, facts.epoch);
        assert_eq!(header.fence.owner_lease_id, "owner-lease-1");
        let bootstrap = header.bootstrap.expect("bootstrap present");
        assert_eq!(
            bootstrap.decode_payload().expect("bootstrap payload"),
            b"bootstrap"
        );

        writer.fail_next();
        assert!(port.send_activate_initial(&request).is_err());
    }

    #[tokio::test]
    async fn actor_idle_evict_control_port_builds_idle_evict_wire_frame() {
        use skiff_router::actor::ActorOwnerFence;
        use skiff_router::supervisor::actor::ActorIdleEvictControlPort;
        use skiff_runtime_transport::actor_owner::{
            decode_actor_owner_control_frame, ActorOwnerControlOperation,
        };

        let (layer, handle, writer, session) = session_layer_with_writer();
        register_session_in_layer(&layer, &session, &epoch().registered_tuple());
        let store = skiff_router::bootstrap::ActiveRoutingEpochStore::new();
        store.publish(epoch());
        let port = ActorIdleEvictControlPort::new(handle, Arc::new(store));
        let key = actor_key();
        let fence = ActorOwnerFence {
            epoch: 7,
            owner_runtime_id: "runtime-a".to_string(),
            owner_lease_id: "owner-lease-1".to_string(),
            lease_expires_at: u64::MAX,
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            declaration_owner: declaration_owner(),
        };
        port.send_idle_evict(&key, &fence, "evict-1", "conn-a")
            .expect("idleEvict writes");
        let bytes = writer.recorded().pop().expect("idleEvict frame");
        let header = decode_actor_owner_control_frame(&bytes).expect("decode idleEvict");
        assert_eq!(header.operation, ActorOwnerControlOperation::IdleEvict);
        assert_eq!(header.fence.eviction_request_id.as_deref(), Some("evict-1"));
        assert_eq!(header.route_authority.assembly_generation, 7);
        assert_eq!(
            header.route_authority.assembly_identity,
            assembly_identity().into_string()
        );
    }
}
