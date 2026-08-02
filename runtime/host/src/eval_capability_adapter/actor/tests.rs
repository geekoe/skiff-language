use super::*;
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, AssemblyIdentity,
    DeploymentRevision,
};
use skiff_runtime_capability_context::{
    ActorInvocationCancellation, ActorInvocationDeadline, ActorInvocationDeclarationOwner,
    ActorInvocationIdentity, ActorInvocationOutcome, ActorInvocationOwnerFile,
    ActorInvocationOwnerUnit, ActorInvocationRequest, ActorKeyControlMetadata,
    OutboundControlMessage, RouterWriterMessage, SpawnCallerKind, SpawnSubmitControlMessage,
};
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, ActorMethodCancelReason, ActorMethodFrame,
};
use skiff_runtime_transport::protocol::decode_binary_frame;
use tokio::time::{timeout, Duration};

const BUILD_ID: &str =
        "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn test_aware_spawn_submit_encodes_only_caller_request_id() {
    let (mut parts, _, mut router_receiver, _) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-control-test-authority",
    );
    parts.activation_identity = Some(test_activation_identity());
    parts.test_case_capability = Some("case:opaque-authority".to_string());
    let borrowed = parts.clone();
    let context = RuntimeActorCapabilityContext {
        actor_context: concrete_actor_context_from_owned(&borrowed),
        request_context: concrete_request_context_from_owned(&borrowed),
        owned: parts,
    };

    let spawn = capture_spawn_submit(&context, &mut router_receiver).await;
    assert_eq!(
        spawn.request.caller_request_id.as_deref(),
        Some("request-test")
    );
    assert_spawn_submit_wire_omits_test_authority(spawn);

    let get_or_create = capture_actor_get_or_create(&context, &mut router_receiver).await;
    assert_eq!(
        get_or_create.test_case_capability.as_deref(),
        Some("case:opaque-authority")
    );
    assert_eq!(
        get_or_create.test_case_parent_request_id.as_deref(),
        Some("request-test")
    );
}

#[tokio::test]
async fn ordinary_spawn_submit_encodes_only_caller_request_id() {
    let (mut parts, _, mut router_receiver, _) =
        actor_invocation_fixture(30_000, CancellationToken::new(), "actor-control-production");
    parts.activation_identity = Some(test_activation_identity());
    let context = RuntimeOwnedRequestCapabilityContext(parts);

    let spawn = capture_spawn_submit(&context, &mut router_receiver).await;
    assert_eq!(
        spawn.request.caller_request_id.as_deref(),
        Some("request-test")
    );
    assert_spawn_submit_wire_omits_test_authority(spawn);

    let get_or_create = capture_actor_get_or_create(&context, &mut router_receiver).await;
    assert_eq!(get_or_create.test_case_capability, None);
    assert_eq!(get_or_create.test_case_parent_request_id, None);
}

#[tokio::test]
async fn actor_invocation_spawn_submit_encodes_closed_actor_invocation_caller_kind() {
    let (mut parts, _, mut router_receiver, _) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invocation-spawn-parent",
    );
    parts.activation_identity = Some(test_activation_identity());
    parts.spawn_caller_kind = SpawnCallerKind::ActorInvocation;
    let borrowed = parts.clone();
    let context = RuntimeActorCapabilityContext {
        actor_context: concrete_actor_context_from_owned(&borrowed),
        request_context: concrete_request_context_from_owned(&borrowed),
        owned: parts,
    };

    let spawn = capture_spawn_submit(&context, &mut router_receiver).await;
    assert_eq!(spawn.caller_kind, SpawnCallerKind::ActorInvocation);
    let frame = crate::host::router_session::spawn_submit::encode_spawn_submit_wire_message(spawn)
        .expect("canonical spawn submit must encode");
    let wire = decode_binary_frame(&frame).expect("spawn submit wire must decode");
    assert_eq!(wire.header["callerKind"], "actorInvocation");
    assert_eq!(wire.header["callerRequestId"], "request-test");
}

#[tokio::test]
async fn direct_actor_invoke_carries_exact_test_parent_authority() {
    let (mut parts, request, mut router_receiver, _outbound) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invoke-test-authority",
    );
    parts.test_case_capability = Some("case:opaque-authority".to_string());
    let invocation = invoke_actor_method(parts, request, test_execution_control());
    tokio::pin!(invocation);

    let message = tokio::select! {
        result = &mut invocation => panic!("actor invocation completed before frame: {result:?}"),
        message = router_receiver.recv() => message.expect("actor invoke frame"),
    };
    let concrete::RouterWriterMessage::Binary(frame) = message else {
        panic!("actor invocation must use binary transport")
    };
    let ActorMethodFrame::Invoke(header, _) =
        decode_actor_method_frame(&frame).expect("actor invoke frame decodes")
    else {
        panic!("expected Actor invoke frame")
    };
    assert_eq!(
        header.test_case_capability.as_deref(),
        Some("case:opaque-authority")
    );
    assert_eq!(
        header.test_case_parent_request_id.as_deref(),
        Some("request-test")
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_request_cancel_releases_lease() {
    let cancellation = CancellationToken::new();
    let (parts, request, mut router_receiver, outbound) =
        actor_invocation_fixture(30_000, cancellation.clone(), "actor-invoke-cancel");
    let invocation = invoke_actor_method(parts, request, test_execution_control());
    tokio::pin!(invocation);

    assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
    cancellation.cancel();
    let outcome = timeout(Duration::from_secs(1), &mut invocation)
        .await
        .expect("actor cancellation must wake the pending invocation")
        .expect("actor cancellation is an internal outcome");
    assert_eq!(
        outcome,
        ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::Cancelled)
    );
    assert_actor_cancel_frame(&mut router_receiver, ActorMethodCancelReason::Cancelled).await;
    assert_eq!(
        outbound.cancellation_correlation("actor-invoke-cancel"),
        None,
        "terminal owner must release the actor invocation lease"
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_primitive_deadline_remains_distinct() {
    let (parts, request, mut router_receiver, outbound) =
        actor_invocation_fixture(1, CancellationToken::new(), "actor-invoke-deadline");
    let invocation = invoke_actor_method(parts, request, test_execution_control());
    tokio::pin!(invocation);

    assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
    let outcome = timeout(Duration::from_secs(1), &mut invocation)
        .await
        .expect("actor deadline must wake the pending invocation")
        .expect("actor deadline is a typed outcome");
    assert_eq!(
        outcome,
        ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::DeadlineExceeded)
    );
    assert_actor_cancel_frame(
        &mut router_receiver,
        ActorMethodCancelReason::DeadlineExceeded,
    )
    .await;
    assert_eq!(
        outbound.cancellation_correlation("actor-invoke-deadline"),
        None,
        "deadline owner must release the actor invocation lease"
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_request_cancel_beats_primitive_deadline() {
    let cancellation = CancellationToken::new();
    let (parts, request, mut router_receiver, outbound) =
        actor_invocation_fixture(1, cancellation.clone(), "actor-invoke-biased");
    let invocation = invoke_actor_method(parts, request, test_execution_control());
    tokio::pin!(invocation);

    assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    cancellation.cancel();
    let outcome = invocation
        .await
        .expect("ancestor cancellation is a typed internal outcome");
    assert_eq!(
        outcome,
        ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::Cancelled)
    );
    assert_actor_cancel_message(
        router_receiver
            .recv()
            .await
            .expect("cancel frame must settle the invocation"),
        ActorMethodCancelReason::Cancelled,
    );
    assert_eq!(
        outbound.cancellation_correlation("actor-invoke-biased"),
        None
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_current_deadline_drops_waiter_and_fences_late_outcome() {
    let (execution, _ancestor, scope) = scoped_test_execution_control(Duration::from_millis(50));
    let lifecycle = scope.clone();
    let (parts, request, mut router_receiver, outbound) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invoke-current-deadline",
    );
    let invocation = invoke_actor_method(parts, request, execution);
    tokio::pin!(invocation);

    let wire_timeout_ms = assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
    assert!(
        (1..=50).contains(&wire_timeout_ms),
        "wire hint must use min(current remaining, 30s primitive), got {wire_timeout_ms}"
    );
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_actor_cancel_frame_while_pending(
        &mut router_receiver,
        &mut invocation,
        ActorMethodCancelReason::DeadlineExceeded,
    )
    .await;

    assert_eq!(outbound.pending_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        !outbound.complete(
            "actor-invoke-current-deadline",
            ActorInvocationOutcome::Returned(vec![1])
        ),
        "late and duplicate Actor outcomes must remain fenced"
    );
    assert!(
        timeout(Duration::from_millis(1), &mut invocation)
            .await
            .is_err(),
        "current scope terminal must stay on the internal control lane"
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_committed_outcome_beats_ready_scope_deadline() {
    let (execution, _ancestor, scope) = scoped_test_execution_control(Duration::from_millis(50));
    let lifecycle = scope.clone();
    let (parts, request, mut router_receiver, outbound) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invoke-response-first",
    );
    let invocation = invoke_actor_method(parts, request, execution);
    tokio::pin!(invocation);
    assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;

    assert!(outbound.complete(
        "actor-invoke-response-first",
        ActorInvocationOutcome::Returned(vec![7])
    ));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        invocation.await.expect("committed Actor outcome must win"),
        ActorInvocationOutcome::Returned(vec![7])
    );
    assert_eq!(outbound.pending_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        router_receiver.try_recv().is_err(),
        "winning response must not emit a cancellation hint"
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_ancestor_stop_is_internal_and_releases_owners() {
    let (execution, ancestor, scope) = scoped_test_execution_control(Duration::from_secs(30));
    let lifecycle = scope.clone();
    let (parts, request, mut router_receiver, outbound) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invoke-ancestor-stop",
    );
    let invocation = invoke_actor_method(parts, request, execution);
    tokio::pin!(invocation);
    assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;

    ancestor.cancel();
    assert_actor_cancel_frame_while_pending(
        &mut router_receiver,
        &mut invocation,
        ActorMethodCancelReason::Cancelled,
    )
    .await;
    assert_eq!(outbound.pending_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        timeout(Duration::from_millis(5), &mut invocation)
            .await
            .is_err(),
        "ancestor stop must not materialize through the ordinary Actor result"
    );
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_outer_deadline_keeps_post_await_owner() {
    let (execution, scope) = scoped_test_execution_with_outer_deadline(
        Duration::from_millis(50),
        Duration::from_secs(30),
    );
    let lifecycle = scope.clone();
    let (parts, request, mut router_receiver, outbound) = actor_invocation_fixture(
        30_000,
        CancellationToken::new(),
        "actor-invoke-outer-deadline",
    );
    let invocation = invoke_actor_method(parts, request, execution);
    tokio::pin!(invocation);

    let wire_timeout_ms = assert_actor_invoke_frame(&mut router_receiver, &mut invocation).await;
    assert!((1..=50).contains(&wire_timeout_ms));
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_actor_cancel_frame_while_pending(
        &mut router_receiver,
        &mut invocation,
        ActorMethodCancelReason::DeadlineExceeded,
    )
    .await;
    assert_eq!(outbound.pending_count(), 0);
    assert_eq!(
        lifecycle.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(
        timeout(Duration::from_millis(5), &mut invocation)
            .await
            .is_err(),
        "outer deadline must be projected by the existing post-await checkpoint"
    );
}

async fn capture_spawn_submit<C>(
    context: &C,
    router_receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
) -> SpawnSubmitControlMessage
where
    C: capability_contract::RequestCapabilityApi + ?Sized,
{
    let submit = capability_contract::RequestCapabilityApi::submit_spawn(
        context,
        spawn_submit_request_with_untrusted_caller(),
        Vec::new(),
        test_execution_control(),
    );
    tokio::pin!(submit);
    loop {
        tokio::select! {
            result = &mut submit => panic!("spawn submit completed before control dispatch: {result:?}"),
            message = router_receiver.recv() => match message {
                Some(RouterWriterMessage::SpawnSubmit(message)) => {
                    break message;
                }
                Some(RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { .. })) => {
                    continue;
                }
                Some(message) => panic!("unexpected control before spawn submit: {message:?}"),
                None => panic!("router writer closed before spawn submit control dispatch"),
            }
        }
    }
}

fn assert_spawn_submit_wire_omits_test_authority(message: SpawnSubmitControlMessage) {
    let frame =
        crate::host::router_session::spawn_submit::encode_spawn_submit_wire_message(message)
            .expect("canonical spawn submit must encode");
    let wire = decode_binary_frame(&frame).expect("spawn submit wire must decode");
    assert_eq!(wire.header["callerKind"], "request");
    assert_eq!(wire.header["callerRequestId"], "request-test");
    assert!(wire.header.get("testCaseCapability").is_none());
    assert!(wire.header.get("testCaseParentRequestId").is_none());
}

async fn capture_actor_get_or_create<C>(
    context: &C,
    router_receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
) -> ActorGetOrCreateControlRequest
where
    C: capability_contract::ActorCapabilityApi + ?Sized,
{
    let get_or_create = capability_contract::ActorCapabilityApi::get_or_create_actor(
        context,
        actor_get_or_create_request_with_untrusted_authority(),
        Vec::new(),
        test_execution_control(),
    );
    tokio::pin!(get_or_create);
    loop {
        tokio::select! {
            result = &mut get_or_create => {
                panic!("Actor get-or-create completed before control dispatch: {result:?}")
            }
            message = router_receiver.recv() => match message {
                Some(RouterWriterMessage::Control(OutboundControlMessage::ActorGetOrCreate { request, .. })) => {
                    break request;
                }
                Some(RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { .. })) => {
                    continue;
                }
                Some(message) => {
                    panic!("unexpected control before Actor get-or-create: {message:?}")
                }
                None => panic!("router writer closed before Actor get-or-create control dispatch"),
            }
        }
    }
}

fn spawn_submit_request_with_untrusted_caller() -> SpawnSubmitControlRequest {
    SpawnSubmitControlRequest {
        rpc_id: "caller-rpc".to_string(),
        runtime_id: "caller-runtime".to_string(),
        target_kind: "function".to_string(),
        service_id: "example.com/worker".to_string(),
        service_version: "v1".to_string(),
        service_protocol_identity: "protocol-test".to_string(),
        target: "function:program.test".to_string(),
        spawn_id: None,
        build_id: Some(BUILD_ID.to_string()),
        activation_identity: test_activation_identity(),
        caller_request_id: Some("request:caller-must-not-authorize".to_string()),
        trace_id: None,
        caller_target: Some("program.test".to_string()),
        max_queue_wait_ms: None,
        actor_method: None,
    }
}

fn actor_get_or_create_request_with_untrusted_authority() -> ActorGetOrCreateControlRequest {
    ActorGetOrCreateControlRequest {
        rpc_id: "caller-rpc".to_string(),
        runtime_id: "caller-runtime".to_string(),
        activation_identity: test_activation_identity(),
        actor_key: ActorKeyControlMetadata {
            service_id: "service-test".to_string(),
            actor_type_identity: "actor-type-test".to_string(),
            actor_id_type_identity: "actor-id-type-test".to_string(),
            actor_id_encoding_version: "skiff-actor-id-v1".to_string(),
            canonical_actor_id_key_bytes_base64: "AQ==".to_string(),
            actor_id_hash: Some(format!("sha256:{}", "d".repeat(64))),
        },
        actor_abi_identity: format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)),
        actor_implementation_identity: format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "b".repeat(64)
        ),
        bootstrap_encoding_version: "skiff-actor-bootstrap-v1".to_string(),
        declaration_owner: ActorInvocationDeclarationOwner {
            unit: ActorInvocationOwnerUnit::Service,
            file: ActorInvocationOwnerFile::LoadedFileIndex(0),
            actor_symbol: "TestActor".to_string(),
        },
        deadline: None,
        test_case_capability: Some("case:caller-must-not-authorize".to_string()),
        test_case_parent_request_id: Some("request:caller-must-not-authorize".to_string()),
    }
}

fn test_activation_identity() -> ActivationIdentityControl {
    ActivationIdentityControl {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "e".repeat(64)
        )),
        generation: 7,
        runtime_replica_id: "runtime-replica-test".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-revision-test"),
    }
}

fn actor_invocation_fixture(
    timeout_ms: u64,
    cancellation: CancellationToken,
    invocation_id: &str,
) -> (
    RuntimeOwnedRequestParts,
    ActorInvocationRequest,
    mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
    Arc<ActorMethodOutboundRegistry>,
) {
    let (router_sender, router_receiver) = mpsc::unbounded_channel();
    let actor_method_outbound = Arc::new(ActorMethodOutboundRegistry::default());
    let implementation_identity = ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        "b".repeat(64)
    ));
    let parts = RuntimeOwnedRequestParts {
        runtime_id: "runtime-test".to_string(),
        service_id: "service-test".to_string(),
        service_version: "v1".to_string(),
        request_id: "request-test".to_string(),
        request_target: "program.test".to_string(),
        request_build_id: BUILD_ID.to_string(),
        request_service_protocol_identity: "protocol-test".to_string(),
        operation_service_protocol_identity: Some("protocol-test".to_string()),
        activation_identity: None,
        trace_id: Some("trace:actor-invoke".to_string()),
        test_case_capability: None,
        spawn_caller_kind: SpawnCallerKind::Request,
        router_sender: Some(router_sender),
        outbound_requests: Arc::new(OutboundRequestRegistry::default()),
        actor_method_outbound: actor_method_outbound.clone(),
        cancellation,
    };
    let request = ActorInvocationRequest {
        actor_ref: ActorRef::new(
            "service-test",
            "actor-type-test",
            "actor-id-type-test",
            "skiff-actor-id-v1",
            vec![1],
            format!("sha256:{}", "d".repeat(64)),
            Some(7),
        ),
        declaration_owner: ActorInvocationDeclarationOwner {
            unit: ActorInvocationOwnerUnit::Service,
            file: ActorInvocationOwnerFile::LoadedFileIndex(0),
            actor_symbol: "TestActor".to_string(),
        },
        identity: ActorInvocationIdentity {
            invocation_id: invocation_id.to_string(),
            expected_epoch: 7,
            actor_abi_identity: ActorAbiIdentity::new(format!(
                "skiff-actor-abi-v1:sha256:{}",
                "a".repeat(64)
            )),
            requested_implementation_identity: implementation_identity,
            method_identity: ActorMethodIdentity::new(format!(
                "skiff-actor-method-v1:sha256:{}",
                "c".repeat(64)
            )),
            cancellation_correlation: format!("{invocation_id}:cancel"),
        },
        deadline: ActorInvocationDeadline { timeout_ms },
        arguments_payload: b"[]".to_vec(),
    };
    (parts, request, router_receiver, actor_method_outbound)
}

async fn assert_actor_invoke_frame<F>(
    router_receiver: &mut mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
    invocation: &mut Pin<&mut F>,
) -> u64
where
    F: Future<
        Output = capability_contract::CapabilityResult<capability_contract::ActorInvocationOutcome>,
    >,
{
    tokio::select! {
        result = invocation.as_mut() => {
            panic!("actor invocation completed before its invoke frame: {result:?}")
        }
        message = router_receiver.recv() => {
            actor_invoke_timeout_ms(
                message.expect("actor method invoke frame must be sent")
            )
        }
    }
}

async fn assert_actor_cancel_frame(
    router_receiver: &mut mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
    expected_reason: ActorMethodCancelReason,
) {
    let message = timeout(Duration::from_secs(1), router_receiver.recv())
        .await
        .expect("actor cancel frame must be emitted")
        .expect("router writer must remain open");
    assert_actor_cancel_message(message, expected_reason);
}

async fn assert_actor_cancel_frame_while_pending<F>(
    router_receiver: &mut mpsc::UnboundedReceiver<concrete::RouterWriterMessage>,
    invocation: &mut Pin<&mut F>,
    expected_reason: ActorMethodCancelReason,
) where
    F: Future<
        Output = capability_contract::CapabilityResult<capability_contract::ActorInvocationOutcome>,
    >,
{
    let message = tokio::select! {
        result = invocation.as_mut() => {
            panic!("scope-terminal Actor invocation must remain pending, got {result:?}")
        }
        message = router_receiver.recv() => {
            message.expect("router writer must remain open")
        }
    };
    assert_actor_cancel_message(message, expected_reason);
}

fn actor_invoke_timeout_ms(message: concrete::RouterWriterMessage) -> u64 {
    let concrete::RouterWriterMessage::Binary(frame) = message else {
        panic!("actor invocation must use a binary frame")
    };
    let ActorMethodFrame::Invoke(header, _) =
        decode_actor_method_frame(&frame).expect("actor invoke frame must decode")
    else {
        panic!("expected Actor method invoke frame")
    };
    assert_eq!(
        header.trace_id.as_deref(),
        Some("trace:actor-invoke"),
        "direct Actor invocation must preserve the caller request trace id"
    );
    header.deadline.timeout_ms
}

fn assert_actor_cancel_message(
    message: concrete::RouterWriterMessage,
    expected_reason: ActorMethodCancelReason,
) {
    let concrete::RouterWriterMessage::Binary(frame) = message else {
        panic!("actor cancellation must use a binary frame")
    };
    let ActorMethodFrame::Cancel(cancel) =
        decode_actor_method_frame(&frame).expect("actor cancel frame must decode")
    else {
        panic!("expected actor method cancel frame")
    };
    assert_eq!(cancel.reason, expected_reason);
}

fn test_execution_control() -> capability_contract::OwnedExecutionControl {
    use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

    let budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig::disabled(),
        None,
    ));
    let execution = skiff_runtime_request::ExecutionControl::new(CancellationToken::new(), &budget);
    super::super::execution_control(execution).owned()
}

fn scoped_test_execution_control(
    deadline_after: Duration,
) -> (
    capability_contract::OwnedExecutionControl,
    capability_contract::CancellationSource,
    capability_contract::ExecutionScope,
) {
    use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
    use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

    let cancellation = capability_contract::CancellationSource::new();
    let budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig::disabled(),
        None,
    ));
    let execution = skiff_runtime_request::ExecutionControl::new(cancellation.token(), &budget);
    let owned = super::super::execution_control(execution).owned();
    let current = owned
        .derive_scope(
            tokio::time::Instant::now().into_std() + deadline_after,
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        )
        .expect("test current scope should derive");
    let scope = current
        .execution_scope()
        .expect("derived test control must expose its current scope");
    (current, cancellation, scope)
}

fn scoped_test_execution_with_outer_deadline(
    outer_deadline_after: Duration,
    local_deadline_after: Duration,
) -> (
    capability_contract::OwnedExecutionControl,
    capability_contract::ExecutionScope,
) {
    use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
    use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

    let now = tokio::time::Instant::now().into_std();
    let budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig::disabled(),
        Some(now + outer_deadline_after),
    ));
    let execution = skiff_runtime_request::ExecutionControl::new(CancellationToken::new(), &budget);
    let owned = super::super::execution_control(execution).owned();
    let current = owned
        .derive_scope(
            now + local_deadline_after,
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        )
        .expect("test current scope should derive under the outer deadline");
    let scope = current
        .execution_scope()
        .expect("derived test control must expose its current scope");
    (current, scope)
}
