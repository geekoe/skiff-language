use std::{future::Future, sync::Arc, time::Instant};

use serde_json::{json, Value};
use skiff_artifact_model::{
    AssemblyIdentity, DeploymentRevision, InstructionSourceSite, SyntheticInstructionSiteReason,
};
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorGetOrCreateControlRequest, ActorInvocationDeclarationOwner,
    ActorInvocationOwnerFile, ActorInvocationOwnerUnit, ActorKeyControlMetadata,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationSource, ExecutionScope,
    OutboundResponse, ResponseError, SpawnSubmitControlRequest,
};

use super::*;

const BUILD_ID: &str =
    "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn f445h_i6_actor_scope_control_four_entries_drop_waiter_and_fence_late_response() {
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let activation_identity = activation_identity();
    let context = ActorClientContext::from_parts(
        "runtime-test",
        "request-test",
        Some(&activation_identity),
        Some(&router_sender),
        outbound_requests.as_ref(),
        CancellationToken::new(),
    );
    let client = ActorClient::new(context);

    let cancellation = CancellationSource::new();
    assert_scoped_control_cancel(
        client.get_or_create_in_scope(
            get_or_create_request(),
            Vec::new(),
            ExecutionScope::request(cancellation.token(), None),
        ),
        "actor.getOrCreate",
        cancellation,
        &mut router_receiver,
        outbound_requests.as_ref(),
    )
    .await;

    let cancellation = CancellationSource::new();
    assert_scoped_control_cancel(
        client.replace_in_scope(
            replace_request(),
            Vec::new(),
            ExecutionScope::request(cancellation.token(), None),
        ),
        "actor.replace",
        cancellation,
        &mut router_receiver,
        outbound_requests.as_ref(),
    )
    .await;

    let cancellation = CancellationSource::new();
    assert_scoped_control_cancel(
        client.find_in_scope(
            find_request(),
            ExecutionScope::request(cancellation.token(), None),
        ),
        "actor.find",
        cancellation,
        &mut router_receiver,
        outbound_requests.as_ref(),
    )
    .await;

    let cancellation = CancellationSource::new();
    assert_scoped_control_cancel(
        client.remove_in_scope(
            remove_request(),
            ExecutionScope::request(cancellation.token(), None),
        ),
        "actor.remove",
        cancellation,
        &mut router_receiver,
        outbound_requests.as_ref(),
    )
    .await;
}

#[tokio::test]
async fn f445h_i6_actor_scope_control_committed_response_beats_ready_scope_deadline() {
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let activation_identity = activation_identity();
    let context = ActorClientContext::from_parts(
        "runtime-test",
        "request-test",
        Some(&activation_identity),
        Some(&router_sender),
        outbound_requests.as_ref(),
        CancellationToken::new(),
    );
    let root = ExecutionScope::request(CancellationToken::new(), None);
    let scope = root
        .derive(
            Instant::now() + std::time::Duration::from_millis(20),
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        )
        .expect("test scope should derive");
    let lifecycle = scope.clone();
    let client = ActorClient::new(context);
    let mut find = Box::pin(client.find_in_scope(find_request(), scope));
    let rpc_id = next_control_request(&mut router_receiver, &mut find, "actor.find").await;

    outbound_requests
        .take_terminal_sender(&rpc_id)
        .expect("find response should still be pending")
        .send(typed_response(json!({
            "schemaVersion": "skiff-runtime-frame-v1",
            "type": "actor.find.response",
            "rpcId": rpc_id,
            "found": false
        })))
        .expect("committed find response should be delivered");
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    assert_eq!(
        find.as_mut().await.expect("committed response must win"),
        None
    );
    assert_eq!(outbound_requests.pending_count(), 0);
    assert_eq!(outbound_requests.active_lease_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        router_receiver.try_recv().is_err(),
        "winning response must not emit a cancellation hint"
    );
}

#[tokio::test]
async fn spawn_submit_accepts_correlated_receipt_and_preserves_activation_identity() {
    let expected_activation = activation_identity();
    let (result, sent_request) = submit_with_response(|rpc_id| {
        typed_response(json!({
            "schemaVersion": "skiff-runtime-frame-v1",
            "type": "spawn.submit.response",
            "rpcId": rpc_id,
            "spawnId": "spawn-7",
            "requestId": "spawn-request-11",
            "status": "submitted"
        }))
    })
    .await;

    let response = result.expect("canonical submitted receipt should succeed");
    assert_eq!(response.spawn_id, "spawn-7");
    assert_eq!(response.request_id, "spawn-request-11");
    assert_eq!(sent_request.rpc_id, response.rpc_id);
    assert_eq!(sent_request.runtime_id, "runtime-test");
    assert_eq!(sent_request.activation_identity, expected_activation);
}

async fn assert_scoped_control_cancel<T, F>(
    future: F,
    expected_target: &str,
    cancellation: CancellationSource,
    router_receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    outbound_requests: &OutboundRequestRegistry,
) where
    F: Future<Output = Result<T>>,
{
    let mut future = Box::pin(future);
    let rpc_id = next_control_request(router_receiver, &mut future, expected_target).await;
    assert_eq!(outbound_requests.pending_count(), 1);
    assert_eq!(outbound_requests.active_lease_count(), 1);

    cancellation.cancel();
    assert!(matches!(
        future.as_mut().await,
        Err(RuntimeError::Cancelled)
    ));
    let cancel = router_receiver
        .recv()
        .await
        .expect("scope terminal must emit a best-effort cancel");
    let RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { request }) = cancel
    else {
        panic!("scope terminal must emit request.cancel")
    };
    assert_eq!(request.request_id, rpc_id);
    assert_eq!(request.reason, "caller_cancel");
    drop(future);

    assert_eq!(outbound_requests.pending_count(), 0);
    assert_eq!(outbound_requests.active_lease_count(), 0);
    assert!(
        outbound_requests.take_terminal_sender(&rpc_id).is_none(),
        "late and duplicate responses must be fenced"
    );
}

async fn next_control_request<T, F>(
    router_receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    future: &mut std::pin::Pin<Box<F>>,
    expected_target: &str,
) -> String
where
    F: Future<Output = Result<T>>,
{
    tokio::select! {
        result = future.as_mut() => {
            let _ = result;
            panic!("Actor control completed before its request was sent")
        }
        message = router_receiver.recv() => {
            let RouterWriterMessage::Control(message) =
                message.expect("Actor control request should be sent")
            else {
                panic!("Actor control must use a control message")
            };
            let (target, rpc_id) = match message {
                OutboundControlMessage::ActorGetOrCreate { request, .. } => {
                    ("actor.getOrCreate", request.rpc_id)
                }
                OutboundControlMessage::ActorReplace { request, .. } => {
                    ("actor.replace", request.rpc_id)
                }
                OutboundControlMessage::ActorFind { request } => {
                    ("actor.find", request.rpc_id)
                }
                OutboundControlMessage::ActorRemove { request } => {
                    ("actor.remove", request.rpc_id)
                }
                other => panic!("unexpected Actor control request: {other:?}"),
            };
            assert_eq!(target, expected_target);
            rpc_id
        }
    }
}

#[tokio::test]
async fn spawn_submit_rejects_uncorrelated_status_and_identity_receipts() {
    let cases = [
        InvalidReceipt::WrongRpcId,
        InvalidReceipt::BadStatus,
        InvalidReceipt::MissingSpawnId,
        InvalidReceipt::EmptySpawnId,
        InvalidReceipt::InvalidSpawnId,
        InvalidReceipt::MissingRequestId,
        InvalidReceipt::EmptyRequestId,
        InvalidReceipt::InvalidRequestId,
    ];

    for case in cases {
        let (result, _) =
            submit_with_response(|rpc_id| typed_response(case.response(rpc_id))).await;
        let error = result.expect_err(case.name());
        assert!(
            error.to_string().contains(case.expected_error()),
            "{} returned unexpected error: {error}",
            case.name()
        );
    }
}

#[tokio::test]
async fn spawn_submit_preserves_typed_router_error_as_failure() {
    let (result, _) = submit_with_response(|_| {
        OutboundResponse::Error(ResponseError {
            code: "SpawnRejected".to_string(),
            message: "spawn queue rejected the request".to_string(),
            status: Some(409),
            details: None,
        })
    })
    .await;

    let error = result.expect_err("typed router error must fail closed");
    assert!(matches!(
        error,
        RuntimeError::ProviderUnavailable { target, reason }
            if target == SPAWN_SUBMIT_TARGET && reason == "spawn queue rejected the request"
    ));
}

async fn submit_with_response(
    response: impl FnOnce(&str) -> OutboundResponse,
) -> (
    Result<SpawnSubmitResponseFrameHeader>,
    SpawnSubmitControlRequest,
) {
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let activation_identity = activation_identity();
    let context = RequestClientContext::from_parts(
        "runtime-test",
        "service-test",
        "v1",
        "request-test",
        "program.test",
        BUILD_ID,
        "protocol-test",
        Some("protocol-test"),
        Some(&activation_identity),
        None,
        Some(&router_sender),
        outbound_requests.as_ref(),
        CancellationToken::new(),
    );
    let client = RequestClient::new(context);
    let submit = client.submit_spawn(spawn_submit_request(), Vec::new());
    tokio::pin!(submit);

    let sent_request = tokio::select! {
        result = &mut submit => panic!("spawn submit completed before response: {result:?}"),
        message = router_receiver.recv() => match message.expect("spawn.submit request should be sent") {
            RouterWriterMessage::Control(
                OutboundControlMessage::SpawnSubmit { request, .. }
            ) => request,
            other => panic!("unexpected router message: {other:?}"),
        }
    };
    outbound_requests
        .take_terminal_sender(&sent_request.rpc_id)
        .expect("spawn submit response should be pending")
        .send(response(&sent_request.rpc_id))
        .expect("spawn submit response should be delivered");

    (submit.await, sent_request)
}

fn typed_response(value: Value) -> OutboundResponse {
    OutboundResponse::End {
        payload: serde_json::to_vec(&value).expect("test response should serialize"),
    }
}

fn activation_identity() -> ActivationIdentityControl {
    ActivationIdentityControl {
        assembly_identity: AssemblyIdentity::new(
            "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        generation: 7,
        runtime_replica_id: "runtime-replica-7".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-revision-7"),
    }
}

fn spawn_submit_request() -> SpawnSubmitControlRequest {
    let mut untrusted_identity = activation_identity();
    untrusted_identity.generation = 99;
    SpawnSubmitControlRequest {
        rpc_id: "caller-supplied-rpc".to_string(),
        runtime_id: "caller-supplied-runtime".to_string(),
        target_kind: "function".to_string(),
        service_id: "service-test".to_string(),
        service_version: "v1".to_string(),
        service_protocol_identity: "protocol-test".to_string(),
        target: "function:program.test".to_string(),
        spawn_id: None,
        build_id: Some(BUILD_ID.to_string()),
        activation_identity: untrusted_identity,
        caller_request_id: Some("request-test".to_string()),
        trace_id: None,
        caller_target: Some("program.test".to_string()),
        max_queue_wait_ms: None,
        actor_method: None,
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

fn get_or_create_request() -> ActorGetOrCreateControlRequest {
    ActorGetOrCreateControlRequest {
        rpc_id: String::new(),
        runtime_id: String::new(),
        activation_identity: activation_identity(),
        actor_key: actor_key(),
        actor_abi_identity: format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)),
        actor_implementation_identity: format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "b".repeat(64)
        ),
        bootstrap_encoding_version: "skiff-actor-bootstrap-v1".to_string(),
        declaration_owner: declaration_owner(),
        deadline: None,
        test_case_capability: None,
        test_case_parent_request_id: None,
    }
}

fn replace_request() -> ActorReplaceControlRequest {
    ActorReplaceControlRequest {
        rpc_id: String::new(),
        runtime_id: String::new(),
        activation_identity: activation_identity(),
        actor_key: actor_key(),
        actor_abi_identity: format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)),
        actor_implementation_identity: format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "b".repeat(64)
        ),
        bootstrap_encoding_version: "skiff-actor-bootstrap-v1".to_string(),
        declaration_owner: declaration_owner(),
        deadline: None,
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

fn remove_request() -> ActorRemoveControlRequest {
    ActorRemoveControlRequest {
        rpc_id: String::new(),
        runtime_id: String::new(),
        activation_identity: activation_identity(),
        actor_key: actor_key(),
    }
}

#[derive(Clone, Copy)]
enum InvalidReceipt {
    WrongRpcId,
    BadStatus,
    MissingSpawnId,
    EmptySpawnId,
    InvalidSpawnId,
    MissingRequestId,
    EmptyRequestId,
    InvalidRequestId,
}

impl InvalidReceipt {
    fn response(self, rpc_id: &str) -> Value {
        let mut response = json!({
            "schemaVersion": "skiff-runtime-frame-v1",
            "type": "spawn.submit.response",
            "rpcId": rpc_id,
            "spawnId": "spawn-7",
            "requestId": "spawn-request-11",
            "status": "submitted"
        });
        match self {
            Self::WrongRpcId => response["rpcId"] = json!("another-rpc"),
            Self::BadStatus => response["status"] = json!("queued"),
            Self::MissingSpawnId => {
                response
                    .as_object_mut()
                    .expect("response object")
                    .remove("spawnId");
            }
            Self::EmptySpawnId => response["spawnId"] = json!(""),
            Self::InvalidSpawnId => response["spawnId"] = json!("spawn id"),
            Self::MissingRequestId => {
                response
                    .as_object_mut()
                    .expect("response object")
                    .remove("requestId");
            }
            Self::EmptyRequestId => response["requestId"] = json!(""),
            Self::InvalidRequestId => response["requestId"] = json!("request\u{7f}id"),
        }
        response
    }

    fn name(self) -> &'static str {
        match self {
            Self::WrongRpcId => "wrong rpcId",
            Self::BadStatus => "bad status",
            Self::MissingSpawnId => "missing spawnId",
            Self::EmptySpawnId => "empty spawnId",
            Self::InvalidSpawnId => "invalid spawnId",
            Self::MissingRequestId => "missing requestId",
            Self::EmptyRequestId => "empty requestId",
            Self::InvalidRequestId => "invalid requestId",
        }
    }

    fn expected_error(self) -> &'static str {
        match self {
            Self::WrongRpcId => "does not match request",
            Self::BadStatus => "status must be submitted",
            Self::MissingSpawnId => "missing field `spawnId`",
            Self::EmptySpawnId | Self::InvalidSpawnId => "spawnId must be an ASCII visible token",
            Self::MissingRequestId => "missing field `requestId`",
            Self::EmptyRequestId | Self::InvalidRequestId => {
                "requestId must be an ASCII visible token"
            }
        }
    }
}

fn declaration_owner() -> ActorInvocationDeclarationOwner {
    ActorInvocationDeclarationOwner {
        unit: ActorInvocationOwnerUnit::Service,
        file: ActorInvocationOwnerFile::FileIrIdentity("file:actor-1".to_string()),
        actor_symbol: "Counter".to_string(),
    }
}
