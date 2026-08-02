use serde_json::json;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, DeploymentArtifactIdentity,
    DeploymentRevision, ServiceDeploymentRef,
};

use super::*;
use skiff_runtime_transport::{
    actor_method::{
        ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
        ActorMethodDeadlineFrameHeader, ActorMethodInvokeFrameHeader, ActorOwnerFileFrameHeader,
        ActorOwnerUnitFrameHeader, ACTOR_ARGUMENTS_ENCODING_V1,
    },
    actor_owner::{
        encode_actor_owner_invoke_frame, ActorOwnerFenceFrameHeader, ActorOwnerInvokeFrameHeader,
        ActorOwnerRouteAuthorityFrameHeader,
    },
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    connection_protocol::{
        encode_connection_response_frame, ConnectionResponseFrameHeader, ConnectionResponseOutcome,
    },
    protocol::{
        encode_binary_frame, RequestCancelFrameHeader, RouterControlFrameHeader,
        RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader, RuntimeRegisteredFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

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

mod activation_prepare;
mod connection_lifecycle;
mod control_response_lifecycle;
mod foreign_db_exact_identity;
mod h_registration_cut;
mod h_spawn_parent_cut;
mod runtime_assembly_request;
mod websocket_generation_lifecycle;
mod websocket_jsonrpc_dispatch;

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
        environment: "test".to_string(),
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("runtime host should build")
}

fn actor_owner_test_deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "actor.test.service".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            "a".repeat(64)
        )),
    }
}

fn actor_owner_test_invoke(
    invocation_id: &str,
    capability: &str,
    parent_request_id: &str,
) -> ActorOwnerInvokeFrameHeader {
    let declaration_owner = ActorDeclarationOwnerFrameHeader {
        unit: ActorOwnerUnitFrameHeader::Service,
        file: ActorOwnerFileFrameHeader::FileIrIdentity("actor-test.skiff".to_string()),
        actor_symbol: "ActorTest".to_string(),
    };
    let actor_abi_identity =
        ActorAbiIdentity::new(format!("skiff-actor-abi-v1:sha256:{}", "b".repeat(64)));
    let actor_implementation_identity = ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        "c".repeat(64)
    ));
    ActorOwnerInvokeFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.owner.invoke".to_string(),
        target_runtime_id: "runtime-base".to_string(),
        owner_fence: ActorOwnerFenceFrameHeader {
            owner_runtime_id: "runtime-base".to_string(),
            owner_lease_id: "owner-lease-1".to_string(),
            epoch: 1,
            actor_abi_identity: actor_abi_identity.clone(),
            actor_implementation_identity: actor_implementation_identity.clone(),
            declaration_owner: declaration_owner.clone(),
        },
        route_authority: ActorOwnerRouteAuthorityFrameHeader {
            assembly_identity: format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
            assembly_generation: 1,
        },
        invoke: ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.method.invoke".to_string(),
            invocation_id: invocation_id.to_string(),
            actor_ref: ActorLogicalRefFrameHeader {
                service_id: "actor.test.service".to_string(),
                actor_type_identity: "actor-test-type".to_string(),
                actor_id_type_identity: "actor-test-id".to_string(),
                actor_id_encoding_version: "skiff-canonical-v1".to_string(),
                canonical_actor_id_key_bytes_base64: "MQ==".to_string(),
                actor_id_hash: format!("sha256:{}", "d".repeat(64)),
                epoch: 1,
            },
            declaration_owner,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity: ActorMethodIdentity::new(format!(
                "skiff-actor-method-v1:sha256:{}",
                "e".repeat(64)
            )),
            arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.to_string(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: 30_000,
                expires_at: "2099-01-01T00:00:00Z".to_string(),
            },
            cancellation_correlation: format!("{invocation_id}:cancel"),
            trace_id: None,
            test_case_capability: Some(capability.to_string()),
            test_case_parent_request_id: Some(parent_request_id.to_string()),
        },
        activation_bootstrap: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn actor_owner_test_admission_is_synchronous_before_detached_first_poll() {
    let host = test_host();
    host.open_actor_instance_session("router-session-sync")
        .unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-sync",
            "router-session-sync",
            "root:actor-sync".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let _owner_task = host
        .spawn_actor_owner_invoke(
            "router-session-sync".to_string(),
            actor_owner_test_invoke("actor:sync-child", "case:actor-sync", "root:actor-sync"),
            Vec::new(),
            sender,
        )
        .unwrap();

    // A current-thread spawn cannot run until this test yields, so these observations prove the
    // frame handler itself installed both owners.
    assert!(host
        .test_http_entries
        .self_ingress_for_request("router-session-sync", "actor:sync-child")
        .is_some());
    assert!(host.actor_owner_invocations.contains("actor:sync-child"));
    assert!(host.actor_owner_invocations.cancel_for_session(
        "actor:sync-child",
        "router-session-sync",
        "actor:sync-child:cancel",
        super::super::actor_owner_invocations::ActorOwnerCancellationReason::Cancelled,
    ));
    let mut finalization = Box::pin(root.finalize());
    let waker = futures_util::task::noop_waker_ref();
    let mut context = std::task::Context::from_waker(waker);
    assert!(std::future::Future::poll(finalization.as_mut(), &mut context).is_pending());

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect("detached Actor owner must terminate")
        .expect("Actor owner failure terminal");
    let skiff_runtime_request::RouterWriterMessage::Binary(terminal) = terminal else {
        panic!("Actor cancellation terminal must be binary")
    };
    let skiff_runtime_transport::actor_method::ActorMethodFrame::Cancel(cancel) =
        skiff_runtime_transport::actor_method::decode_actor_method_frame(&terminal)
            .expect("Actor cancellation terminal decodes")
    else {
        panic!("pre-first-poll cancellation must settle as Actor cancel")
    };
    assert_eq!(
        cancel.reason,
        skiff_runtime_transport::actor_method::ActorMethodCancelReason::Cancelled
    );
    tokio::time::timeout(std::time::Duration::from_secs(1), finalization)
        .await
        .expect("root finalization must resume after Actor terminal tail")
        .unwrap();
    assert!(!host.actor_owner_invocations.contains("actor:sync-child"));
}

#[tokio::test(flavor = "current_thread")]
async fn actor_owner_partial_admission_rolls_back_and_authority_failures_do_not_execute() {
    let host = test_host();
    host.open_actor_instance_session("router-session-sync")
        .unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-rollback",
            "router-session-sync",
            "root:actor-rollback".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let cancellation_correlation = "actor:duplicate:cancel".to_string();
    let duplicate_registration = host
        .actor_owner_invocations
        .register(
            "actor:duplicate".to_string(),
            "router-session-sync".to_string(),
            cancellation_correlation,
        )
        .expect("pre-register duplicate invocation");
    let duplicate_token = duplicate_registration.cancellation();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let duplicate = host
        .spawn_actor_owner_invoke(
            "router-session-sync".to_string(),
            actor_owner_test_invoke(
                "actor:duplicate",
                "case:actor-rollback",
                "root:actor-rollback",
            ),
            Vec::new(),
            sender.clone(),
        )
        .expect_err("duplicate invocation must fail in the frame handler");
    assert!(duplicate
        .to_string()
        .contains("duplicate Actor invocation id"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request("router-session-sync", "actor:duplicate")
        .is_none());
    assert!(host.actor_owner_invocations.contains("actor:duplicate"));
    assert!(
        !duplicate_token.is_cancelled(),
        "rollback must retain and not cancel the pre-existing registry entry"
    );
    assert!(receiver.try_recv().is_err(), "no detached task may execute");
    assert!(host.actor_owner_invocations.cancel_registered(
        duplicate_registration.identity(),
        super::super::actor_owner_invocations::ActorOwnerCancellationReason::Cancelled,
    ));
    assert_eq!(
        host.actor_owner_invocations
            .finish(duplicate_registration.identity()),
        Some(super::super::actor_owner_invocations::ActorOwnerCancellationReason::Cancelled),
        "rollback must leave the exact pre-existing registration finishable"
    );

    let unknown = host
        .spawn_actor_owner_invoke(
            "router-session-sync".to_string(),
            actor_owner_test_invoke("actor:unknown", "case:missing", "root:missing"),
            Vec::new(),
            sender,
        )
        .expect_err("unknown capability must fail in the frame handler");
    assert!(
        unknown.to_string().contains("parent request is unknown")
            || unknown.to_string().contains("unknown or finalized")
    );
    assert!(!host.actor_owner_invocations.contains("actor:unknown"));
    root.finalize().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn actor_owner_effective_deadline_sends_typed_terminal_and_cleans_real_owners() {
    let host = test_host();
    let session = "router-session-deadline";
    host.open_actor_instance_session(session).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-deadline",
            session,
            "root:actor-deadline".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:deadline-child",
        "case:actor-deadline",
        "root:actor-deadline",
    );
    // A zero timeout with a still-future wall-clock expiry exercises the real Host deadline
    // arbiter deterministically; both deadline and the route lookup may be ready on first poll.
    invoke.invoke.deadline.timeout_ms = 0;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let owner_task = host
        .spawn_actor_owner_invoke(session.to_string(), invoke, Vec::new(), sender)
        .unwrap();

    let terminal = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
        .await
        .expect("deadline terminal must be bounded")
        .expect("deadline terminal must be sent");
    let skiff_runtime_request::RouterWriterMessage::Binary(terminal) = terminal else {
        panic!("deadline terminal must be binary")
    };
    let ActorMethodFrame::Cancel(cancel) = decode_actor_method_frame(&terminal).unwrap() else {
        panic!("deadline must not degrade to actor.owner.failure")
    };
    assert_eq!(cancel.invocation_id, "actor:deadline-child");
    assert_eq!(
        cancel.cancellation_correlation,
        "actor:deadline-child:cancel"
    );
    assert_eq!(
        cancel.reason,
        skiff_runtime_transport::actor_method::ActorMethodCancelReason::DeadlineExceeded
    );
    owner_task.await.unwrap();
    root.finalize().await.unwrap();
    assert!(!host
        .actor_owner_invocations
        .contains("actor:deadline-child"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:deadline-child")
        .is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn already_expired_actor_owner_deadline_revokes_parent_and_finishes_before_terminal_handoff()
{
    let host = test_host();
    let session = "router-session-already-expired";
    host.open_actor_instance_session(session).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-already-expired",
            session,
            "root:actor-already-expired".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:already-expired",
        "case:actor-already-expired",
        "root:actor-already-expired",
    );
    invoke.invoke.deadline.timeout_ms = 30_000;
    invoke.invoke.deadline.expires_at = "2000-01-01T00:00:00Z".to_string();
    invoke.invoke.trace_id =
        Some("skiff-test:pause-expired-actor-owner-before-terminal".to_string());
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let owner_task = host
        .spawn_actor_owner_invoke(session.to_string(), invoke, Vec::new(), sender)
        .unwrap();

    assert!(host
        .actor_owner_invocations
        .contains("actor:already-expired"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:already-expired")
        .is_some());

    super::super::actor_owner_execution::expired_actor_owner_terminal_barrier()
        .wait()
        .await;
    assert!(
        !host
            .actor_owner_invocations
            .contains("actor:already-expired"),
        "the exact invocation registration must be finished"
    );
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:already-expired")
        .is_none());
    let before_terminal = host
        .test_http_entries
        .begin_actor_method(
            "case:actor-already-expired",
            "actor:already-expired",
            session,
            "actor:expired-before-terminal".to_string(),
        )
        .err()
        .expect("the expired invocation must not authorize descendants");
    assert!(before_terminal
        .to_string()
        .contains("parent request is unknown"));
    assert!(
        receiver.try_recv().is_err(),
        "the terminal must not be sent until authority revocation and registry finish are visible"
    );
    super::super::actor_owner_execution::expired_actor_owner_terminal_barrier()
        .wait()
        .await;

    let terminal = receiver
        .recv()
        .await
        .expect("deadline terminal must be sent");
    let skiff_runtime_request::RouterWriterMessage::Binary(terminal) = terminal else {
        panic!("deadline terminal must be binary")
    };
    let ActorMethodFrame::Cancel(cancel) = decode_actor_method_frame(&terminal).unwrap() else {
        panic!("expired deadline must not degrade to actor.owner.failure")
    };
    assert_eq!(cancel.invocation_id, "actor:already-expired");
    assert_eq!(
        cancel.cancellation_correlation,
        "actor:already-expired:cancel"
    );
    assert_eq!(
        cancel.reason,
        skiff_runtime_transport::actor_method::ActorMethodCancelReason::DeadlineExceeded
    );

    let after_terminal = host
        .test_http_entries
        .begin_actor_method(
            "case:actor-already-expired",
            "actor:already-expired",
            session,
            "actor:expired-after-terminal".to_string(),
        )
        .err()
        .expect("terminal delivery must not restore expired parent authority");
    assert!(after_terminal
        .to_string()
        .contains("parent request is unknown"));
    owner_task.await.unwrap();
    root.finalize().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn aborting_real_actor_owner_task_releases_invocation_and_test_lease_before_first_poll() {
    let host = test_host();
    let session = "router-session-abort";
    host.open_actor_instance_session(session).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-task-abort",
            session,
            "root:actor-task-abort".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let owner_task = host
        .spawn_actor_owner_invoke(
            session.to_string(),
            actor_owner_test_invoke(
                "actor:task-abort",
                "case:actor-task-abort",
                "root:actor-task-abort",
            ),
            Vec::new(),
            sender,
        )
        .unwrap();
    assert!(host.actor_owner_invocations.contains("actor:task-abort"));
    owner_task.abort();
    assert!(owner_task.await.unwrap_err().is_cancelled());

    root.finalize().await.unwrap();
    assert!(!host.actor_owner_invocations.contains("actor:task-abort"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:task-abort")
        .is_none());
}

#[tokio::test]
async fn panicking_real_actor_owner_task_releases_invocation_and_test_lease() {
    let host = test_host();
    let session = "router-session-panic";
    host.open_actor_instance_session(session).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:actor-task-panic",
            session,
            "root:actor-task-panic".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:task-panic",
        "case:actor-task-panic",
        "root:actor-task-panic",
    );
    invoke.invoke.trace_id = Some("skiff-test:panic-after-actor-owner-admission".to_string());
    let (sender, _receiver) = mpsc::unbounded_channel();
    let owner_task = host
        .spawn_actor_owner_invoke(session.to_string(), invoke, Vec::new(), sender)
        .unwrap();
    assert!(owner_task.await.unwrap_err().is_panic());

    root.finalize().await.unwrap();
    assert!(!host.actor_owner_invocations.contains("actor:task-panic"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:task-panic")
        .is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn stale_production_task_drop_cannot_finish_reused_invocation_registration() {
    let host = test_host();
    let reused_id = "actor:reused-production-id";
    let old_session = "router-session-reuse-old";
    host.open_actor_instance_session(old_session).unwrap();
    let old_root = host
        .test_http_entries
        .begin_root_case(
            "case:reuse-old",
            old_session,
            "root:reuse-old".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (old_sender, _old_receiver) = mpsc::unbounded_channel();
    let old_task = host
        .spawn_actor_owner_invoke(
            old_session.to_string(),
            actor_owner_test_invoke(reused_id, "case:reuse-old", "root:reuse-old"),
            Vec::new(),
            old_sender,
        )
        .unwrap();
    assert_eq!(host.actor_owner_invocations.cancel_session(old_session), 1);
    host.test_http_entries
        .disconnect_session(old_session)
        .unwrap();

    let new_session = "router-session-reuse-new";
    let new_root = host
        .test_http_entries
        .begin_root_case(
            "case:reuse-new",
            new_session,
            "root:reuse-new".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let new_test_execution = host
        .test_http_entries
        .begin_actor_method(
            "case:reuse-new",
            "root:reuse-new",
            new_session,
            reused_id.to_string(),
        )
        .unwrap();
    let new_registration = host
        .actor_owner_invocations
        .register(
            reused_id.to_string(),
            new_session.to_string(),
            format!("{reused_id}:cancel"),
        )
        .unwrap();

    old_task.abort();
    assert!(old_task.await.unwrap_err().is_cancelled());
    assert!(
        host.actor_owner_invocations.contains(reused_id),
        "old generation Drop must not remove the reused registration"
    );
    assert!(host
        .test_http_entries
        .self_ingress_for_request(new_session, reused_id)
        .is_some());

    assert_eq!(
        host.actor_owner_invocations
            .finish(new_registration.identity()),
        None
    );
    drop(new_test_execution);
    old_root.finalize().await.unwrap();
    new_root.finalize().await.unwrap();
    assert!(!host.actor_owner_invocations.contains(reused_id));
}

#[tokio::test(flavor = "current_thread")]
async fn closed_writer_does_not_leak_actor_terminal_owners() {
    let host = test_host();
    let session = "router-session-writer-closed";
    host.open_actor_instance_session(session).unwrap();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:writer-closed",
            session,
            "root:writer-closed".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let mut invoke = actor_owner_test_invoke(
        "actor:writer-closed",
        "case:writer-closed",
        "root:writer-closed",
    );
    invoke.invoke.deadline.timeout_ms = 0;
    let (sender, receiver) = mpsc::unbounded_channel();
    drop(receiver);
    let owner_task = host
        .spawn_actor_owner_invoke(session.to_string(), invoke, Vec::new(), sender)
        .unwrap();
    owner_task.await.unwrap();
    root.finalize().await.unwrap();
    assert!(!host.actor_owner_invocations.contains("actor:writer-closed"));
    assert!(host
        .test_http_entries
        .self_ingress_for_request(session, "actor:writer-closed")
        .is_none());
}

#[tokio::test]
async fn actor_owner_authority_errors_propagate_through_dispatch_and_cross_session_fails_closed() {
    const SESSION_A: &str = "skiff-router-session-v1:opaque:authority-a";
    const SESSION_B: &str = "skiff-router-session-v1:opaque:authority-b";
    let host = test_host();
    let root = host
        .test_http_entries
        .begin_root_case(
            "case:dispatch-authority",
            SESSION_A,
            "root:dispatch-authority".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            actor_owner_test_deployment(),
        )
        .unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let frame = encode_actor_owner_invoke_frame(
        &actor_owner_test_invoke(
            "actor:cross-session",
            "case:dispatch-authority",
            "root:dispatch-authority",
        ),
        &[],
    )
    .unwrap();
    let mut bootstrap = Some(super::test_connection_bootstrap("cross-session").unwrap());
    let mut handshake = super::handshake::ClientHandshake::registered();
    let error = super::dispatch_router_binary_frame_inner(
        &host,
        SESSION_B,
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("foreign Router session must not replay a valid parent");
    assert!(error.to_string().contains("another router session"));
    assert!(!host.actor_owner_invocations.contains("actor:cross-session"));
    assert!(receiver.try_recv().is_err());

    let stale_parent = host
        .test_http_entries
        .begin_actor_method(
            "case:dispatch-authority",
            "root:dispatch-authority",
            SESSION_A,
            "actor:stale-parent".to_string(),
        )
        .unwrap();
    drop(stale_parent);
    let frame = encode_actor_owner_invoke_frame(
        &actor_owner_test_invoke(
            "actor:stale-child",
            "case:dispatch-authority",
            "actor:stale-parent",
        ),
        &[],
    )
    .unwrap();
    let mut bootstrap = Some(super::test_connection_bootstrap("stale-parent").unwrap());
    let mut handshake = super::handshake::ClientHandshake::registered();
    let error = super::dispatch_router_binary_frame_inner(
        &host,
        SESSION_A,
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("released derived parent must fail in dispatch");
    assert!(error.to_string().contains("parent request is unknown"));
    assert!(!host.actor_owner_invocations.contains("actor:stale-child"));
    assert!(receiver.try_recv().is_err());
    root.finalize().await.unwrap();
}

#[tokio::test]
async fn text_json_router_control_is_rejected_on_runtime_websocket() {
    let error = reject_router_text_message(
        &json!({
            "type": "router.control",
            "artifactRoots": ["/tmp/skiff-runtime-router-control"],
        })
        .to_string(),
    )
    .expect_err("text JSON router.control should fail closed");

    assert!(
        matches!(error, RuntimeError::Decode(_)),
        "unexpected error: {error:?}"
    );
    assert!(error
        .to_string()
        .contains("text protocol messages are not supported on runtime WebSocket"));
}

#[test]
fn writer_encodes_outbound_control_command_as_binary_frame() {
    let message = super::super::RouterWriterMessage::Control(
        skiff_runtime_request::OutboundControlMessage::RequestCancel {
            request: skiff_runtime_request::RequestCancelControl {
                request_id: "request-cancel-from-control".to_string(),
                reason: "caller_cancel".to_string(),
            },
        },
    );

    let bytes = match encode_writer_message(message).expect("control command should encode") {
        tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes,
        other => panic!("expected binary websocket message, got {other:?}"),
    };
    let (header, payload): (RequestCancelFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&bytes).expect("request.cancel should decode");

    assert_eq!(header.request_id, "request-cancel-from-control");
    assert_eq!(header.reason, "caller_cancel");
    assert!(payload.is_empty());
}

#[tokio::test]
async fn writer_sends_no_websocket_frame_for_invalid_spawn_service_id() {
    use std::{
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{Context, Poll},
    };

    struct CountingSocket(Arc<AtomicUsize>);

    impl futures_util::Sink<tokio_tungstenite::tungstenite::Message> for CountingSocket {
        type Error = tokio_tungstenite::tungstenite::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(
            self: Pin<&mut Self>,
            _message: tokio_tungstenite::tungstenite::Message,
        ) -> std::result::Result<(), Self::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    let encoded_frames = Arc::new(AtomicUsize::new(0));
    let message = super::super::RouterWriterMessage::SpawnSubmit(
        skiff_runtime_request::SpawnSubmitControlMessage {
            request: skiff_runtime_request::SpawnSubmitControlRequest {
                rpc_id: "rpc-spawn".to_string(),
                runtime_id: "runtime-1".to_string(),
                target_kind: "operation".to_string(),
                service_id: "test.skiff/agine.ai/api-tests/case-23".to_string(),
                service_version: "1.0.0".to_string(),
                service_protocol_identity: "service-protocol-1".to_string(),
                target: "Worker.run".to_string(),
                spawn_id: Some("spawn-1".to_string()),
                build_id: Some("build-1".to_string()),
                activation_identity: skiff_runtime_request::ActivationIdentityControl {
                    assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                        "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    ),
                    generation: 7,
                    runtime_replica_id: "runtime-replica-7".to_string(),
                    deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                        "deployment-revision-7",
                    ),
                },
                caller_request_id: Some("request-1".to_string()),
                trace_id: Some("trace-1".to_string()),
                caller_target: Some("Caller.start".to_string()),
                max_queue_wait_ms: Some(250.0),
                actor_method: None,
            },
            payload: b"opaque spawn args".to_vec(),
            caller_kind: skiff_runtime_request::SpawnCallerKind::Request,
        },
    );
    send_writer_message(&mut CountingSocket(Arc::clone(&encoded_frames)), message)
        .await
        .expect_err("invalid service ID must fail before writing a frame");

    assert_eq!(encoded_frames.load(Ordering::SeqCst), 0);
}

#[test]
fn connection_bootstrap_fixes_exact_artifact_path_and_db_transport() {
    let artifact_path = std::env::temp_dir().join(format!(
        "skiff-runtime-bootstrap-positive-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let config_snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
        artifact_path.join("runtime-config"),
    )
    .expect("test config snapshot store should open");
    let typed = TypedEnvelope {
        envelope_type: "router.bootstrap".to_string(),
        rest: serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "artifactsPath": artifact_path,
            "serviceDb": { "mongoUrl": "mongodb://router-owned" },
            "activation": {
                "environment": "test",
                "generation": 7,
                "assembly": {
                    "assemblyIdentity": format!(
                        "skiff-runtime-assembly-v3:sha256:{}",
                        "a".repeat(64)
                    )
                },
                "configSnapshot": {
                    "snapshotId": format!(
                        "skiff-runtime-config-snapshot-v1:{}",
                        "a".repeat(32)
                    )
                }
            },
            "http": { "maxResponseBytes": 67108864 }
        }))
        .expect("bootstrap fields should decode"),
    };

    let bootstrap =
        super::decode_connection_bootstrap(typed, &[]).expect("bootstrap should install");

    assert_eq!(
        bootstrap.resolver.store().root(),
        artifact_path
            .canonicalize()
            .expect("test artifact root should canonicalize")
    );
    assert_eq!(
        bootstrap.service_db.mongo_url,
        "mongodb://router-owned".to_string()
    );
    assert_eq!(
        bootstrap.config_snapshot_store.root(),
        config_snapshot_store.root()
    );
    assert_eq!(bootstrap.activation.environment, "test");
    assert_eq!(bootstrap.activation.generation, 7);
    assert_eq!(
        bootstrap.activation.assembly.assembly_identity.as_str(),
        format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64))
    );
    assert_eq!(bootstrap.max_response_bytes, 67_108_864);
}

#[tokio::test]
async fn runtime_health_frame_reports_loop_risk_counters() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let stream_baseline = crate::capability_context::stream_runtime_streams_active();
    let flag_waiter_baseline =
        skiff_runtime_capability_context::flag_backed_cancel_waiters_active();

    let counters = host.runtime_health_counters().await;
    host.queue_runtime_health_with_counters(&sender, "runtime-health-zero", counters)
        .await
        .expect("runtime.health should encode");

    let frame = match receiver
        .recv()
        .await
        .expect("runtime.health frame should be queued")
    {
        super::super::RouterWriterMessage::Binary(frame) => frame,
        other => panic!("expected binary runtime.health frame, got {other:?}"),
    };
    let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("runtime.health should decode");

    assert!(payload.is_empty());
    assert_eq!(header.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(header.envelope_type, "runtime.health");
    assert_eq!(header.runtime_id, "runtime-health-zero");
    assert_eq!(header.counters.outbound_requests_pending, 0);
    assert_eq!(header.counters.outbound_stream_leases_active, 0);
    assert_eq!(
        header.counters.stream_runtime_streams_active,
        stream_baseline
    );
    assert_eq!(
        header.counters.flag_backed_cancel_waiters_active,
        flag_waiter_baseline
    );
    assert_eq!(header.counters.spawned_tasks_active, 0);
}

#[tokio::test]
async fn runtime_health_reporter_sends_immediate_zero_transition() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut reporter = RuntimeHealthReporter::default();
    reporter
        .registered_runtime_ids
        .insert("runtime-health-zero-transition".to_string());

    reporter
        .send_counters(
            &host,
            &sender,
            runtime_health_counters_for_test(1, 1, 0, 0, 0),
        )
        .await
        .expect("nonzero runtime.health should send");
    let nonzero = recv_runtime_health(&mut receiver).await;
    assert_eq!(nonzero.runtime_id, "runtime-health-zero-transition");
    assert_eq!(nonzero.counters.outbound_requests_pending, 1);
    assert!(reporter.should_probe_zero_transition());

    let sent = reporter
        .send_zero_transition_for_counters(
            &host,
            &sender,
            runtime_health_counters_for_test(0, 0, 0, 0, 0),
        )
        .await
        .expect("zero transition runtime.health should send");
    assert!(sent);
    let zero = recv_runtime_health(&mut receiver).await;
    assert_eq!(zero.runtime_id, "runtime-health-zero-transition");
    assert_eq!(zero.counters.outbound_requests_pending, 0);
    assert_eq!(zero.counters.outbound_stream_leases_active, 0);
    assert!(!reporter.should_probe_zero_transition());
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn runtime_health_reporter_sends_final_frame_before_session_close() {
    let host = test_host();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut reporter = RuntimeHealthReporter::default();
    reporter
        .registered_runtime_ids
        .insert("runtime-health-final".to_string());

    reporter
        .send_final(&host, &sender)
        .await
        .expect("final runtime.health should send before session close");
    let final_health = recv_runtime_health(&mut receiver).await;
    assert_eq!(final_health.runtime_id, "runtime-health-final");
    assert_eq!(final_health.envelope_type, "runtime.health");
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn binary_runtime_registered_with_empty_payload_is_accepted() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut bootstrap = Some(super::test_connection_bootstrap("registered-ack-accepted").unwrap());
    let mut handshake = super::handshake::ClientHandshake::register_sent();
    let frame = encode_binary_frame(
        &RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: "runtime-base".to_string(),
        },
        &[],
    )
    .expect("runtime.registered frame should encode");

    super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect("binary runtime.registered should be accepted");
}

async fn recv_runtime_health(
    receiver: &mut mpsc::UnboundedReceiver<super::super::RouterWriterMessage>,
) -> RuntimeHealthFrameHeader {
    match receiver
        .recv()
        .await
        .expect("runtime.health frame should be queued")
    {
        super::super::RouterWriterMessage::Binary(frame) => {
            let (header, payload): (RuntimeHealthFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("runtime.health should decode");
            assert!(payload.is_empty());
            header
        }
        other => panic!("expected binary runtime.health frame, got {other:?}"),
    }
}

fn runtime_health_counters_for_test(
    outbound_requests_pending: usize,
    outbound_stream_leases_active: usize,
    stream_runtime_streams_active: usize,
    flag_backed_cancel_waiters_active: usize,
    spawned_tasks_active: usize,
) -> RuntimeHealthCountersFrameHeader {
    RuntimeHealthCountersFrameHeader {
        outbound_requests_pending,
        outbound_stream_leases_active,
        stream_runtime_streams_active,
        flag_backed_cancel_waiters_active,
        spawned_tasks_active,
    }
}

#[tokio::test]
async fn binary_runtime_registered_rejects_non_empty_payload() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut bootstrap = Some(super::test_connection_bootstrap("registered-ack-payload").unwrap());
    let mut handshake = super::handshake::ClientHandshake::register_sent();
    let frame = encode_binary_frame(
        &RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: "runtime-base".to_string(),
        },
        b"unexpected",
    )
    .expect("runtime.registered frame should encode");

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("non-empty runtime.registered payload should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("runtime.registered binary frame payload must be empty"));
}

#[tokio::test]
async fn binary_runtime_registered_identity_change_is_terminal() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut bootstrap = Some(super::test_connection_bootstrap("registered-ack-identity").unwrap());
    let mut handshake = super::handshake::ClientHandshake::register_sent();
    let frame = encode_binary_frame(
        &RuntimeRegisteredFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "runtime.registered".to_string(),
            runtime_id: "runtime-other-replica".to_string(),
        },
        &[],
    )
    .expect("runtime.registered frame should encode");

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("mismatched ACK identity must fail");

    assert!(error.to_string().contains("IdentityChange"));
}

#[tokio::test]
async fn binary_router_control_is_rejected_before_legacy_payload_decode() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &RouterControlFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "router.control".to_string(),
            artifact_roots: vec!["/tmp/skiff-runtime-router-control".into()],
            dev_reload: None,
            mode: None,
            generation: None,
            fingerprint: None,
            service_config: Vec::new(),
            telemetry: None,
            file_backend: None,
        },
        b"unexpected",
    )
    .expect("router.control frame should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("legacy router.control should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("router.control artifactRoots/serviceConfig reload is not supported"));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[tokio::test]
async fn binary_router_control_decode_error_propagates() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    let frame = encode_binary_frame(
        &json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "router.control",
            "artifactRoots": 123,
        }),
        &[],
    )
    .expect("invalid router.control frame should encode");

    let error = dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect_err("invalid binary router.control should fail");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(control.is_none());
    assert!(artifact_fingerprint.is_none());
}

#[test]
fn binary_assembly_activation_command_uses_router_to_runtime_codec() {
    let activation = assembly_activation_control("prepare");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("router activation command should encode");

    assert_eq!(
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RouterToRuntime, &frame)
            .expect("Router activation command should decode"),
        activation
    );
}

#[test]
fn assembly_activation_frame_type_is_identified_without_applying_production_behavior() {
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &assembly_activation_control("prepare"),
    )
    .expect("router activation command should encode");
    assert_eq!(
        super::activation::router_binary_frame_type(&frame).unwrap(),
        skiff_runtime_transport::assembly_activation::ASSEMBLY_ACTIVATION_FRAME_TYPE
    );
}

#[tokio::test]
async fn assembly_activation_fails_closed_before_connection_bootstrap() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &assembly_activation_control("prepare"),
    )
    .expect("router activation command should encode");
    let mut bootstrap = None;
    let mut handshake = super::handshake::ClientHandshake::registered();

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("activation before bootstrap must fail");

    assert!(error
        .to_string()
        .contains("assembly activation requires router.bootstrap first"));
}

#[tokio::test]
async fn duplicate_connection_bootstrap_fails_closed() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let artifact_path = std::env::temp_dir().join("skiff-runtime-bootstrap-duplicate");
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let config_snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
        artifact_path.join("runtime-config"),
    )
    .expect("test config snapshot store should open");
    let mut bootstrap = Some(super::ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .expect("test resolver should open"),
        config_snapshot_store,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        activation: super::test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    });
    let frame = encode_binary_frame(
        &json!({
            "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
            "type": "router.bootstrap",
            "artifactsPath": artifact_path,
            "serviceDb": { "mongoUrl": "mongodb://127.0.0.1:27017" },
            "activation": {
                "environment": "test",
                "generation": 0,
                "assembly": {
                    "assemblyIdentity": format!(
                        "skiff-runtime-assembly-v3:sha256:{}",
                        "a".repeat(64)
                    )
                },
                "configSnapshot": {
                    "snapshotId": format!(
                        "skiff-runtime-config-snapshot-v1:{}",
                        "a".repeat(32)
                    )
                }
            },
            "http": { "maxResponseBytes": 67108864 }
        }),
        &[],
    )
    .expect("bootstrap frame should encode");
    let mut handshake = super::handshake::ClientHandshake::registered();

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("duplicate bootstrap must fail");

    assert!(error
        .to_string()
        .contains("router.bootstrap must appear exactly once per connection"));
}

#[tokio::test]
async fn activation_rejects_superseded_transient_service_db_wire() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let artifact_path = std::env::temp_dir().join("skiff-runtime-bootstrap-service-db");
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let config_snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
        artifact_path.join("runtime-config"),
    )
    .expect("test config snapshot store should open");
    let mut bootstrap = Some(super::ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .expect("test resolver should open"),
        config_snapshot_store,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://bootstrap-owner".to_string(),
        },
        activation: super::test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    });
    let mut activation = serde_json::to_value(assembly_activation_control("prepare"))
        .expect("activation should encode as JSON");
    activation
        .as_object_mut()
        .expect("activation should be an object")
        .insert(
            "serviceDb".to_string(),
            json!({ "mongoUrl": "mongodb://transient-owner" }),
        );
    let activation: skiff_artifact_model::AssemblyActivationControl =
        serde_json::from_value(activation).expect("legacy activation wire should decode");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("activation frame should encode");
    let mut handshake = super::handshake::ClientHandshake::registered();

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("transient serviceDb must fail");

    assert!(error
        .to_string()
        .contains("assembly activation serviceDb is not supported"));
}

#[tokio::test]
async fn activation_rejects_environment_other_than_runtime_trust_domain_before_resolution() {
    let host = test_host();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let artifact_path = std::env::temp_dir().join(format!(
        "skiff-runtime-bootstrap-environment-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_path).expect("test artifact root should exist");
    let config_snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
        artifact_path.join("runtime-config"),
    )
    .expect("test config snapshot store should open");
    let mut bootstrap = Some(super::ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .expect("test resolver should open"),
        config_snapshot_store,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://bootstrap-owner".to_string(),
        },
        activation: super::test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    });
    let mut activation = serde_json::to_value(assembly_activation_control("prepare"))
        .expect("activation should encode as JSON");
    activation
        .as_object_mut()
        .expect("activation should be an object")
        .insert("environment".to_string(), json!("prod"));
    let activation: skiff_artifact_model::AssemblyActivationControl =
        serde_json::from_value(activation).expect("activation control should decode");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        &activation,
    )
    .expect("activation frame should encode");
    let mut handshake = super::handshake::ClientHandshake::registered();

    let error = super::dispatch_router_binary_frame_inner(
        &host,
        "skiff-router-session-v1:opaque:test-session",
        &frame,
        &sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
    .expect_err("foreign activation environment must fail before snapshot resolution");

    assert!(error
        .to_string()
        .contains("does not match Runtime trusted environment"));
}

#[test]
fn assembly_activation_reply_uses_runtime_to_router_codec() {
    let activation = assembly_activation_control("prepared");
    let frame = encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RuntimeToRouter,
        &activation,
    )
    .expect("runtime activation reply should encode");
    let decoded =
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &frame)
            .expect("runtime activation reply should decode in runtime-to-router direction");

    assert_eq!(decoded, activation);
}

fn assembly_activation_control(
    control_type: &str,
) -> skiff_artifact_model::AssemblyActivationControl {
    serde_json::from_value(json!({
        "type": control_type,
        "environment": "test",
        "activationId": "activation-42",
        "expectedGeneration": 41,
        "candidateGeneration": 42,
        "assembly": {
            "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "configSnapshot": {
            "snapshotId": "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "replicaId": "runtime-base"
    }))
    .expect("assembly activation control fixture should decode")
}

#[tokio::test]
async fn text_json_request_start_is_rejected_on_runtime_websocket() {
    let error = reject_router_text_message(
        &json!({
            "type": "request.start",
            "requestId": "request-legacy-text",
            "mode": "unary",
            "target": "service.test.Api.hello",
            "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "serviceProtocolIdentity": "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "trace": {
                "traceId": "trace-legacy-text",
                "spanId": "span-legacy-text"
            },
            "args": {
                "name": "Ada"
            }
        })
        .to_string(),
    )
    .expect_err("text protocol request.start should fail closed");

    assert!(matches!(error, RuntimeError::Decode(_)));
    assert!(error
        .to_string()
        .contains("text protocol messages are not supported on runtime WebSocket"));
}
