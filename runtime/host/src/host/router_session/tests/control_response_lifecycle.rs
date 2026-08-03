use std::time::{Duration, Instant};

use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorFindControlRequest, ActorKeyControlMetadata, CancellationToken,
    ExecutionScope, OutboundControlMessage, RouterWriterMessage, TaskSubmitControlRequest,
    TaskSubmitTimingControl,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, ActorFindResponseFrameHeader, ActorTaskRuntimeErrorFrameHeader,
    RuntimeErrorFramePayload, TaskRef, TaskSubmitResponseFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::{sync::mpsc, time::timeout};

use crate::capability_context::{
    ActorClient, ActorClientContext, RequestClient, RequestClientContext,
};

use super::*;

const BUILD_ID: &str =
    "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn scoped_actor_response_dispatch_commits_before_ready_deadline() {
    let host = test_host();
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let activation = activation_identity();
    let context = actor_context(&host, &router_sender, &activation);
    let client = ActorClient::new(ActorClientContext::from(&context));
    let deadline = Instant::now() + Duration::from_millis(40);
    let scope = ExecutionScope::request(CancellationToken::new(), Some(deadline));
    let lifecycle = scope.clone();
    let find = client.find_in_scope(find_request(), scope);
    tokio::pin!(find);

    let rpc_id = tokio::select! {
        result = &mut find => panic!("scoped Actor find completed before dispatch: {result:?}"),
        message = router_receiver.recv() => {
            let Some(RouterWriterMessage::Control(OutboundControlMessage::ActorFind { request })) = message else {
                panic!("Actor find must emit its control request")
            };
            request.rpc_id
        }
    };
    let frame = encode_binary_frame(
        &ActorFindResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.response".to_string(),
            rpc_id,
            found: false,
            actor_ref: None,
        },
        &[],
    )
    .expect("Actor find response frame");
    dispatch_frame(&host, &frame).await;

    tokio::time::sleep_until(deadline.into()).await;
    assert_eq!(
        timeout(Duration::from_millis(100), &mut find)
            .await
            .expect("dispatcher must release the scoped response waiter")
            .expect("committed response must beat the ready deadline"),
        None
    );
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        router_receiver.try_recv().is_err(),
        "winning response must not emit request.cancel"
    );
}

#[tokio::test]
async fn scoped_task_submit_response_dispatch_reaches_the_caller() {
    let host = test_host();
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let activation = activation_identity();
    let scope = ExecutionScope::request(CancellationToken::new(), None);
    let lifecycle = scope.clone();
    let context = actor_context(&host, &router_sender, &activation);
    let client = RequestClient::new(context);
    let submit = client.submit_task_in_scope(
        task_submit_request(),
        Vec::new(),
        scope,
        skiff_runtime_request::TaskCallerKind::Request,
    );
    tokio::pin!(submit);

    let rpc_id = tokio::select! {
        result = &mut submit => panic!("scoped task submit completed before dispatch: {result:?}"),
        message = router_receiver.recv() => {
            let Some(RouterWriterMessage::TaskSubmit(message)) = message else {
                panic!("task submit must emit its canonical writer message")
            };
            message.request.rpc_id
        }
    };
    let frame = encode_binary_frame(
        &TaskSubmitResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "task.submit.response".to_string(),
            rpc_id: rpc_id.clone(),
            task_ref: TaskRef::new("task-7", "example.com/docs").expect("task ref"),
            task_id: "task-7".to_string(),
            request_id: "task-request-11".to_string(),
            status: "submitted".to_string(),
        },
        &[],
    )
    .expect("task submit response frame");
    dispatch_frame(&host, &frame).await;

    let response = timeout(Duration::from_millis(100), &mut submit)
        .await
        .expect("dispatcher must release the scoped task waiter")
        .expect("task submit response must reach its caller");
    assert_eq!(response.rpc_id, rpc_id);
    assert_eq!(response.task_id, "task-7");
    assert_eq!(response.request_id, "task-request-11");
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        router_receiver.try_recv().is_err(),
        "winning task response must not emit request.cancel"
    );
}

#[tokio::test]
async fn scoped_actor_error_dispatch_reaches_the_caller() {
    let host = test_host();
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let activation = activation_identity();
    let context = actor_context(&host, &router_sender, &activation);
    let client = ActorClient::new(ActorClientContext::from(&context));
    let scope = ExecutionScope::request(CancellationToken::new(), None);
    let find = client.find_in_scope(find_request(), scope);
    tokio::pin!(find);

    let rpc_id = tokio::select! {
        result = &mut find => panic!("scoped Actor find completed before dispatch: {result:?}"),
        message = router_receiver.recv() => {
            let Some(RouterWriterMessage::Control(OutboundControlMessage::ActorFind { request })) = message else {
                panic!("Actor find must emit its control request")
            };
            request.rpc_id
        }
    };
    let frame = encode_binary_frame(
        &ActorTaskRuntimeErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.error".to_string(),
            rpc_id,
            error: RuntimeErrorFramePayload {
                code: "ActorLookupFailed".to_string(),
                message: "Actor lookup failed".to_string(),
                status: Some(503),
                details: None,
            },
        },
        &[],
    )
    .expect("Actor find error frame");
    dispatch_frame(&host, &frame).await;

    assert!(matches!(
        timeout(Duration::from_millis(100), &mut find)
            .await
            .expect("error dispatcher must release the scoped response waiter"),
        Err(crate::error::RuntimeError::ProviderUnavailable { target, reason })
            if target == "actor.find" && reason == "Actor lookup failed"
    ));
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
    assert!(
        router_receiver.try_recv().is_err(),
        "winning error must not emit request.cancel"
    );
}

async fn dispatch_frame(host: &crate::host::RuntimeHost, frame: &[u8]) {
    let (sender, _receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut artifact_fingerprint = None;
    dispatch_router_binary_frame(
        host,
        frame,
        &sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
    .expect("Router response frame must dispatch");
}

fn actor_context<'a>(
    host: &'a crate::host::RuntimeHost,
    router_sender: &'a mpsc::UnboundedSender<RouterWriterMessage>,
    activation: &'a ActivationIdentityControl,
) -> RequestClientContext<'a> {
    RequestClientContext::from_parts(
        "runtime-base",
        "service-test",
        "v1",
        "request-test",
        "program.test",
        BUILD_ID,
        "protocol-test",
        Some("protocol-test"),
        Some(activation),
        None,
        Some(router_sender),
        host.outbound_requests.as_ref(),
        CancellationToken::new(),
    )
}

fn activation_identity() -> ActivationIdentityControl {
    ActivationIdentityControl {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "a".repeat(64)
        )),
        generation: 7,
        runtime_replica_id: "runtime-replica-7".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-revision-7"),
    }
}

fn actor_key() -> ActorKeyControlMetadata {
    ActorKeyControlMetadata {
        service_id: "service-test".to_string(),
        actor_type_identity: "actor-type-test".to_string(),
        actor_id_type_identity: "actor-id-type-test".to_string(),
        actor_id_encoding_version: "skiff-actor-id-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "AQ==".to_string(),
        actor_id_hash: Some(format!("sha256:{}", "d".repeat(64))),
    }
}

fn find_request() -> ActorFindControlRequest {
    ActorFindControlRequest {
        rpc_id: String::new(),
        runtime_id: String::new(),
        activation_identity: activation_identity(),
        actor_key: actor_key(),
    }
}

fn task_submit_request() -> TaskSubmitControlRequest {
    TaskSubmitControlRequest {
        rpc_id: String::new(),
        runtime_id: String::new(),
        target_kind: "function".to_string(),
        service_id: "service-test".to_string(),
        service_version: "v1".to_string(),
        service_protocol_identity: "protocol-test".to_string(),
        target: "function:program.test".to_string(),
        task_id: None,
        build_id: Some(BUILD_ID.to_string()),
        activation_identity: activation_identity(),
        caller_request_id: Some("request-test".to_string()),
        timing: TaskSubmitTimingControl::Immediate,
        trace_id: None,
        caller_target: Some("program.test".to_string()),
        max_queue_wait_ms: None,
        actor_method: None,
    }
}
