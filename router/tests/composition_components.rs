//! W-composition component tests: `HttpDispatchPort` ↔ `RequestDispatcher`
//! adapter (contract conversion, reject mapping, unary/stream round-trip,
//! timeout/cancel), request-family sink, WS responder and the actor outbound
//! control ports. M4: no activation enqueue/transaction ports (retired).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, GatewayEntryIdentity, PackageArtifactRef,
    PackageBuildId, PackageLocalAbiIdentity, ServiceDeploymentRef,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingMethod, ActorRoutingProjection, ActorRoutingRef,
    ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::actor::{ActivationControlPort, ActorMethodCatalogView, IdleEvictControlPort};
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::dispatch::{
    CandidateViewSource, LeaseRevalidate, RevalidateOutcome, RuntimeDispatcherOptions, RuntimePeer,
    SessionAbortControl,
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
use skiff_router::session::directory::{RegistrationFacts, RuntimeRegistrationDirectory};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::layer::{SessionLayer, SessionLayerOptions, SessionRegistrationFacts};
use skiff_router::supervisor::http::{
    dispatch_submit_from_request, DispatcherHttpPort, PendingHttpRouter, RequestFrameSink,
};
use skiff_router::supervisor::session_ports::{SessionHandle, WsRuntimeResponder};
use skiff_router::ws::BrokerRuntimeResponse;
use skiff_router::ws::RuntimeResponder;
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
fn deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/docs".to_string(),
        contract_version: "example.com/docs@1".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-deployment-artifact-v4:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        ),
    }
}

fn manifest() -> ConsumerManifest {
    ConsumerManifest::installed([
        ConsumerKind::HealthLedger,
        ConsumerKind::RequestDispatcher,
        ConsumerKind::WebSocketRequestBroker,
    ])
}

fn registered_directory() -> (RuntimeRegistrationDirectory, RuntimeSessionEpoch) {
    let manifest = manifest();
    let mut directory = RuntimeRegistrationDirectory::new(&manifest);
    let session = RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    directory
        .publish_pending(
            &session,
            &[
                ConsumerKind::HealthLedger,
                ConsumerKind::RequestDispatcher,
                ConsumerKind::WebSocketRequestBroker,
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
            SessionRegistrationFacts {
                dispatch: DispatchCapabilities {
                    unary: true,
                    server_stream: true,
                },
                registration: RegistrationFacts {
                    registered_build_ids: Vec::new(),
                    lazy_load: true,
                    artifact_root: Some("shared-artifact-root".to_string()),
                },
            },
        )]);
        RuntimeCandidateQuery::snapshot_directory_view(
            &directory,
            &capabilities,
            Some("shared-artifact-root".to_string()),
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct OkRevalidate;

impl LeaseRevalidate for OkRevalidate {
    fn revalidate(&self, _request_id: &str, _lease: &RegisteredSessionLease) -> RevalidateOutcome {
        RevalidateOutcome::Ok
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

    fn send_task_attempt_start(
        &self,
        _session: &RuntimeSessionEpoch,
        _attempt: &skiff_router::dispatch::TaskAttemptSubmit,
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

fn dispatcher_pair(
    directory: Arc<Mutex<RuntimeRegistrationDirectory>>,
) -> (
    Arc<skiff_router::dispatch::RequestDispatcher>,
    Arc<RecordingPeer>,
) {
    let peer = Arc::new(RecordingPeer::default());
    let dispatcher = Arc::new(
        skiff_router::dispatch::RequestDispatcher::new(
            RuntimeDispatcherOptions::new(
                4,
                Arc::new(DirectoryViewSource { directory }),
                Arc::new(OkRevalidate),
                Arc::clone(&peer) as Arc<dyn RuntimePeer>,
                Arc::new(RecordingAbort::default()) as Arc<dyn SessionAbortControl>,
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
            assembly_identity: None,
            assembly_generation: None,
            deployment: deployment(),
            build_id: Some(deployment().deployment_artifact_identity.to_string()),
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
    async fn dispatcher_http_port_rejects_without_candidates() {
        let mut directory = RuntimeRegistrationDirectory::new(&manifest());
        let session = RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        };
        directory
            .publish_pending(
                &session,
                &[
                    ConsumerKind::HealthLedger,
                    ConsumerKind::RequestDispatcher,
                    ConsumerKind::WebSocketRequestBroker,
                ],
            )
            .expect("publish pending");
        // No registered session: the candidate projection is empty and the
        // dispatch must fail closed (503 no eligible runtime).
        let peer = Arc::new(RecordingPeer::default());
        let dispatcher = Arc::new(
            skiff_router::dispatch::RequestDispatcher::new(
                RuntimeDispatcherOptions::new(
                    4,
                    Arc::new(DirectoryViewSource {
                        directory: Arc::new(Mutex::new(directory)),
                    }),
                    Arc::new(OkRevalidate),
                    Arc::clone(&peer) as Arc<dyn RuntimePeer>,
                    Arc::new(RecordingAbort::default()) as Arc<dyn SessionAbortControl>,
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
            .dispatch_unary(http_request("req-no-candidate"))
            .await
            .expect_err("no candidate must fail closed");
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
        let (dispatcher, peer) = dispatcher_pair(Arc::new(Mutex::new(directory)));
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
        let (dispatcher, _peer) = dispatcher_pair(Arc::new(Mutex::new(directory)));
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
        let (dispatcher, peer) = dispatcher_pair(Arc::new(Mutex::new(directory)));
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
            Arc::new(GenericConsumer(ConsumerKind::WebSocketRequestBroker)),
        ];
        let layer = Arc::new(
            SessionLayer::with_options(
                config,
                SessionLayerOptions {
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
                artifacts_path: "/opt/skiff/artifacts".into(),
                dev_reload: None,
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

    fn register_session_in_layer(layer: &SessionLayer, session: &RuntimeSessionEpoch) {
        let mut directory = layer.directory_lock();
        directory
            .publish_pending(
                session,
                &[
                    ConsumerKind::HealthLedger,
                    ConsumerKind::RequestDispatcher,
                    ConsumerKind::WebSocketRequestBroker,
                ],
            )
            .expect("publish pending");
        assert!(directory.mark_registered(session));
    }

    /// Temp artifact root with the fixture actor routing projection record
    /// (the M4 catalog view lazy-loads from the artifact store).
    struct CatalogRoot {
        root: PathBuf,
    }

    impl CatalogRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "skiff-router-composition-components-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create artifact root");
            skiff_deployment::storage::CanonicalArtifactStore::create(&root)
                .expect("create artifact store");
            let projection = ActorRoutingProjection::new(
                ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
                vec![ActorRoutingMethod {
                    actor: ActorRoutingRef {
                        service_id: "example.com/docs".to_string(),
                        actor_abi_identity: actor_abi(),
                    },
                    actor_implementation_identity: actor_implementation(),
                    method_identity: skiff_artifact_model::ActorMethodIdentity::new(
                        "skiff-actor-method-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    ),
                    deployment: deployment(),
                    package: PackageArtifactRef {
                        package_id: "example.com/docs".to_string(),
                        package_version: "0.1.0".to_string(),
                        package_build_id: PackageBuildId::new(
                            "skiff-package-build-v10:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                        ),
                        package_local_abi_identity: PackageLocalAbiIdentity::new(
                            "skiff-package-local-abi-v7:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                        ),
                    },
                }],
            )
            .expect("projection");
            let bytes = skiff_canonical_json::canonical_json_bytes(&projection)
                .expect("canonical projection");
            let path = root.join(skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH);
            std::fs::create_dir_all(path.parent().expect("projection parent"))
                .expect("create projection dirs");
            std::fs::write(path, bytes).expect("write projection record");
            Self { root }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for CatalogRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn projection_ref() -> ActorRoutingProjectionRef {
        ActorRoutingProjectionRef::new(
            skiff_artifact_identity::ArtifactRelativePath::new(
                skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("projection path"),
        )
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
        register_session_in_layer(&layer, &session);
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
            &[],
        );
        let port = ActorActivationControlPort::new(handle, Arc::clone(&registry));
        let request = ActivateInitialControlRequest {
            request_id: "control:activate-initial-1".to_string(),
            actor_key: key.clone(),
            facts: CommitFenceFacts {
                actor_abi_identity: abi,
                actor_implementation_identity: implementation,
                declaration_owner: declaration,
                owner_lease_id: "owner-lease-1".to_string(),
            },
            owner_runtime_id: "runtime-a".to_string(),
            owner_connection: "conn-a".to_string(),
            route_authority: ActorOwnerRouteAuthority {
                build_id: deployment().deployment_artifact_identity.to_string(),
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
        // E-actor-parity: the wire fence uses the broker-minted lease id
        // carried on the control request facts (single mint per activation).
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
        register_session_in_layer(&layer, &session);
        let catalog_root = CatalogRoot::new();
        let view = Arc::new(
            ActorMethodCatalogView::new(catalog_root.path(), projection_ref())
                .expect("catalog view opens"),
        );
        let port = ActorIdleEvictControlPort::new(handle, view);
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
        assert_eq!(
            header.route_authority.build_id,
            deployment().deployment_artifact_identity.to_string()
        );
    }
}
