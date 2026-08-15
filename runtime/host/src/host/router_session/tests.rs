use skiff_runtime_transport::{
    actor_method::{
        encode_actor_method_frame, ActorMethodFrame, ActorMethodReturnFrameHeader,
        ACTOR_RETURN_ENCODING_V1,
    },
    connection_protocol::{
        encode_connection_response_frame, ConnectionResponseFrameHeader, ConnectionResponseOutcome,
    },
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
};

use super::*;

#[tokio::test]
async fn connection_request_response_demux_uses_exact_router_session() {
    let host = test_host();
    let session = skiff_runtime_capability_context::ConnectionRequestSession::new(
        "skiff-router-session-v1:opaque:test-session",
    )
    .expect("test session");
    let cancellation = skiff_runtime_capability_context::CancellationSource::new();
    let scope =
        skiff_runtime_capability_context::ExecutionScope::request(cancellation.token(), None);
    let mut pending = host
        .connection_requests
        .install(session, scope, std::sync::Arc::new(|_, _| Ok(())))
        .expect("pending request");
    let request_id = pending.request_id().to_string();
    let frame = encode_connection_response_frame(
        &ConnectionResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "connection.response".to_string(),
            request_id,
            outcome: ConnectionResponseOutcome::Success,
            remote: None,
        },
        b"null",
    )
    .expect("strict response frame");
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("response should dispatch");

    assert_eq!(
        pending.wait().await,
        skiff_runtime_capability_context::ConnectionRequestTerminal::Success(b"null".to_vec())
    );
    assert_eq!(host.connection_requests.pending_count(), 0);
    assert_eq!(host.connection_requests.active_lease_count(), 0);
    assert_eq!(host.connection_requests.active_timer_count(), 0);
}

#[tokio::test]
async fn actor_method_return_frame_demuxes_to_outbound_registry() {
    let host = test_host();
    let mut lease = host
        .actor_method_outbound
        .register(
            "invoke-return".to_string(),
            "cancel-return".to_string(),
            1,
            skiff_artifact_model::ActorImplementationIdentity::new(format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "a".repeat(64)
            )),
        )
        .expect("register actor return lease");
    let header = ActorMethodReturnFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.method.return".to_string(),
        invocation_id: "invoke-return".to_string(),
        return_encoding_version: ACTOR_RETURN_ENCODING_V1.to_string(),
    };
    let frame = encode_actor_method_frame(&ActorMethodFrame::Return(header, b"ok".to_vec()))
        .expect("encode actor return frame");
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;

    dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("actor.method.return should dispatch");

    assert_eq!(
        lease.receive().await.unwrap().unwrap(),
        ActorInvocationOutcome::Returned(b"ok".to_vec())
    );
}

mod control_response_lifecycle;
mod h_task_parent_cut;
mod stream_flush_ack;

#[derive(Clone)]
struct TestDbCapabilityFactory;

impl skiff_runtime_capability_context::DbCapabilityFactory for TestDbCapabilityFactory {
    fn context_for_request(
        &self,
        _owner: String,
        _request_id: String,
    ) -> skiff_runtime_capability_context::DbCapabilityContext {
        skiff_runtime_capability_context::DbCapabilityContext::unavailable()
    }
}

#[derive(Clone)]
struct TestDbProviderFactory;

impl skiff_runtime_capability_context::DbProviderFactory for TestDbProviderFactory {
    fn build(
        &self,
        _input: skiff_runtime_capability_context::DbProviderBuildInput,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilitySource,
    > {
        Ok(skiff_runtime_capability_context::DbCapabilitySource::new(
            Some(TestDbCapabilityFactory),
        ))
    }
}

fn test_db_provider() -> skiff_runtime_capability_context::DbProviderSource {
    skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory)
}

fn test_host() -> super::super::RuntimeHost {
    super::super::RuntimeHost::new(super::super::RuntimeConfig {
        db_provider: test_db_provider(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        profile: "test".to_string(),
        bytecode_only: true,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("runtime host should build")
}
