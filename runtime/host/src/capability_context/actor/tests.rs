use std::sync::Arc;

use serde_json::{json, Value};
use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_capability_context::{
    OutboundResponse, ResponseError, SpawnSubmitControlRequest,
};

use super::*;

const BUILD_ID: &str =
    "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn spawn_submit_accepts_correlated_receipt_and_preserves_activation_identity() {
    let expected_activation = activation_identity();
    let (result, sent_request) = submit_with_response(|rpc_id| {
        typed_response(json!({
            "schemaVersion": "skiff-runtime-frame-v1",
            "type": "spawn.submit.response",
            "rpcId": rpc_id,
            "spawnId": "spawn-7",
            "itemId": "spawn-item-11",
            "status": "submitted"
        }))
    })
    .await;

    let response = result.expect("canonical submitted receipt should succeed");
    assert_eq!(response.spawn_id, "spawn-7");
    assert_eq!(response.item_id, "spawn-item-11");
    assert_eq!(sent_request.rpc_id, response.rpc_id);
    assert_eq!(sent_request.runtime_id, "runtime-test");
    assert_eq!(sent_request.activation_identity, expected_activation);
}

#[tokio::test]
async fn spawn_submit_rejects_uncorrelated_status_and_identity_receipts() {
    let cases = [
        InvalidReceipt::WrongRpcId,
        InvalidReceipt::BadStatus,
        InvalidReceipt::MissingSpawnId,
        InvalidReceipt::EmptySpawnId,
        InvalidReceipt::InvalidSpawnId,
        InvalidReceipt::MissingItemId,
        InvalidReceipt::EmptyItemId,
        InvalidReceipt::InvalidItemId,
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
    let context = ActorClientContext::from_parts(
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
    let client = ActorClient::new(context);
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
        .complete_for_test(&sent_request.rpc_id)
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
            "skiff-runtime-assembly-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
    }
}

#[derive(Clone, Copy)]
enum InvalidReceipt {
    WrongRpcId,
    BadStatus,
    MissingSpawnId,
    EmptySpawnId,
    InvalidSpawnId,
    MissingItemId,
    EmptyItemId,
    InvalidItemId,
}

impl InvalidReceipt {
    fn response(self, rpc_id: &str) -> Value {
        let mut response = json!({
            "schemaVersion": "skiff-runtime-frame-v1",
            "type": "spawn.submit.response",
            "rpcId": rpc_id,
            "spawnId": "spawn-7",
            "itemId": "spawn-item-11",
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
            Self::MissingItemId => {
                response
                    .as_object_mut()
                    .expect("response object")
                    .remove("itemId");
            }
            Self::EmptyItemId => response["itemId"] = json!(""),
            Self::InvalidItemId => response["itemId"] = json!("item\u{7f}id"),
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
            Self::MissingItemId => "missing itemId",
            Self::EmptyItemId => "empty itemId",
            Self::InvalidItemId => "invalid itemId",
        }
    }

    fn expected_error(self) -> &'static str {
        match self {
            Self::WrongRpcId => "does not match request",
            Self::BadStatus => "status must be submitted",
            Self::MissingSpawnId => "missing field `spawnId`",
            Self::EmptySpawnId | Self::InvalidSpawnId => "spawnId must be an ASCII visible token",
            Self::MissingItemId => "missing field `itemId`",
            Self::EmptyItemId | Self::InvalidItemId => "itemId must be an ASCII visible token",
        }
    }
}
