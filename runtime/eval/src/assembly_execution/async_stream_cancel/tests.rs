use std::{
    collections::BTreeSet,
    task::{Context, Poll, Wake, Waker},
};

use skiff_artifact_model::{
    AssemblyIdentity, BoundaryFeatureUnavailableReason, DeploymentArtifactIdentity,
    DeploymentRevision, PackageBuildId, PackageSchemaCanonicalDescriptor, PackageSchemaTypeId,
    PackageSchemaTypeRecord, ServiceDeploymentRef,
};
use skiff_runtime_activation::{ActivationContext, ActivationIdentity, RequestActivationContext};
use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
use skiff_runtime_capability_context::{
    ExecutionDeadlineSource, ExecutionScope, ExecutionScopeTerminal, StreamCancelSignalApi,
    StreamPoll, StreamRuntime,
};
use skiff_runtime_linked_program::{LinkedCallTarget, LinkedExprIr};
use skiff_runtime_model::{
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields},
    type_plan::RuntimeTypeNode,
};

use crate::{
    assembly_execution::{
        ordinary::tests::{
            service_error_consumer::{
                ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture,
            },
            test_runtime,
        },
        service_error_channel::{
            start_restricted_service_diagnostic_probe_for_test,
            take_restricted_service_diagnostics_for_test,
        },
        RuntimeAssemblyExecutionProjection,
    },
    runtime_ops::runtime_to_wire_required_plan,
    Interpreter,
};

use super::*;

#[derive(Debug)]
struct TestStreamCancel(CancellationToken);

impl StreamCancelSignalApi for TestStreamCancel {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.0.wait_cancelled().await })
    }
}

#[derive(Debug)]
struct TestLifetimeDrop(Arc<AtomicUsize>);

impl Drop for TestLifetimeDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

impl StreamLifetimeGuardApi for TestLifetimeDrop {}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn assert_provider_stream_depth_borrow_sites() {
    let source = include_str!("../async_stream_cancel.rs");
    let entry = source
        .split_once("async fn run_provider_stream")
        .expect("provider stream task body remains present")
        .1
        .split_once("#[allow(clippy::too_many_arguments)]")
        .expect("provider callable boundary remains present")
        .0;
    assert!(
        entry.contains("producer.provider_context.borrow_for_scheduled_task()"),
        "the independent provider task must reset inherited program-call depth"
    );
    assert!(
        !entry.contains("producer.provider_context.borrow()"),
        "the provider callable entry must not inherit the caller task depth"
    );

    let finish = source
        .split_once("async fn finish_provider_stream")
        .expect("provider terminal owner remains present")
        .1
        .split_once("async fn publish_provider_deadline_terminal")
        .expect("provider terminal owner remains bounded")
        .0;
    assert!(
        finish.contains("producer.provider_context.borrow()"),
        "post-call error export remains an ordinary borrow"
    );

    let unary = include_str!("../async_stream_cancel/prepared_unary.rs");
    assert!(
        unary.contains("provider_context.borrow()"),
        "the original-chain unary continuation remains an ordinary borrow"
    );
    assert!(
        !unary.contains("borrow_for_scheduled_task"),
        "the unary continuation must not reset active program-call depth"
    );
}

#[test]
fn provider_stream_scheduler_entry_uses_fresh_depth_borrow() {
    assert_provider_stream_depth_borrow_sites();
}

fn assert_inherited_request_deadline(
    error: &RuntimeError,
    request_scope: &ExecutionScope,
    expected_deadline: Instant,
) {
    let RuntimeError::ScopeTerminal(carrier) = error else {
        panic!("request deadline must remain an internal scope terminal, got {error:?}");
    };
    let ExecutionScopeTerminal::InheritedDeadlineExceeded(deadline) = carrier.terminal() else {
        panic!(
            "request deadline must remain inherited rather than locally owned, got {:?}",
            carrier.terminal()
        );
    };
    assert_eq!(deadline.at(), expected_deadline);
    assert_eq!(deadline.source(), &ExecutionDeadlineSource::Request);
    assert_eq!(deadline.nesting(), 0);
    assert_eq!(request_scope.nesting(), 0);
    assert_eq!(request_scope.effective_deadline(), Some(deadline));
    assert!(
        !carrier.is_owned_by(request_scope),
        "a request deadline is not owned by a lexical timeout scope"
    );
}

fn test_stream_cancel() -> (CancellationToken, StreamCancelSignal) {
    let token = CancellationToken::new();
    (
        token.clone(),
        StreamCancelSignal::new(TestStreamCancel(token)),
    )
}

#[test]
fn in_process_stream_spawn_matrix_is_exhaustive() {
    let variants = BTreeSet::from([
        AsyncStreamSpawn::ProviderUnary,
        AsyncStreamSpawn::ProviderStreamProducer,
    ]);
    assert_eq!(variants.len(), 2);
}

#[test]
fn unsupported_callback_contract_remains_a_typed_runtime_error() {
    let error = validate_supported_callback_contract(
        &skiff_artifact_model::ContractOperationId::new("operation:unsupported-callback"),
        &BoundaryCallbackContract::Unsupported {
            reason: BoundaryFeatureUnavailableReason::UnknownSemantics,
        },
    )
    .expect_err("unsupported callback semantics must fail closed");
    assert!(matches!(error, RuntimeError::Unsupported(_)));
}

#[tokio::test]
async fn ready_provider_unary_returns_without_forced_yield() {
    let execution = test_runtime::execution_control();
    let request = RequestActivationContext::begin(activation("ready", "ready-build")).unwrap();
    let value = await_provider_unary(&execution, &request, async { Ok(RuntimeValue::Bool(true)) })
        .await
        .into_result()
        .expect("ready provider should return during the initial poll");
    assert_eq!(value, RuntimeValue::Bool(true));
    assert!(request.open_stream().is_some());
}

#[tokio::test]
async fn pending_provider_unary_wakes_from_provider_completion() {
    let execution = test_runtime::execution_control();
    let request =
        RequestActivationContext::begin(activation("provider", "provider-build")).unwrap();
    let (complete, completed) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn({
        let execution = execution.clone();
        let request = request.clone();
        async move {
            await_provider_unary(&execution, &request, async move {
                completed.await.expect("provider completion sender");
                Ok(RuntimeValue::Bool(true))
            })
            .await
            .into_result()
        }
    });
    tokio::task::yield_now().await;
    complete.send(()).unwrap();
    assert_eq!(waiter.await.unwrap().unwrap(), RuntimeValue::Bool(true));
}

#[tokio::test]
async fn pending_provider_unary_wakes_from_request_cancellation() {
    let execution = test_runtime::execution_control();
    let cancellation = execution.cancellation_token();
    let request = RequestActivationContext::begin(activation("cancel", "cancel-build")).unwrap();
    let waiter = tokio::spawn({
        let execution = execution.clone();
        let request = request.clone();
        async move {
            await_provider_unary(&execution, &request, std::future::pending())
                .await
                .into_result()
        }
    });

    cancellation.cancel();

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("cooperative cancellation should wake the pending provider")
        .expect("provider waiter should not panic")
        .expect_err("pending provider should terminate as cancelled");
    assert!(error.is_cancelled());
    assert!(
        request.open_stream().is_none(),
        "caller cancellation must cancel the provider request"
    );
}

#[tokio::test]
async fn f445h_e4r7_stream_deadline_pending_unary_preserves_inherited_request_carrier() {
    let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
    let execution = test_runtime::execution_control_with_deadline(Some(request_deadline));
    let request_scope = execution
        .execution_scope()
        .expect("test execution exposes its current request scope");
    let request =
        RequestActivationContext::begin(activation("deadline", "deadline-build")).unwrap();
    let error = await_provider_unary(&execution, &request, std::future::pending())
        .await
        .into_result()
        .expect_err("expired deadline should wake the pending provider");
    assert_inherited_request_deadline(&error, &request_scope, request_deadline);
    assert!(
        request.open_stream().is_none(),
        "deadline must emit the provider request cancellation signal"
    );
}

#[tokio::test]
async fn request_cancel_precedes_expired_deadline_and_ready_provider() {
    let execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ));
    let cancellation = execution.cancellation_token();
    cancellation.cancel();
    let request = RequestActivationContext::begin(activation("race", "race-build")).unwrap();
    let error = await_provider_unary(&execution, &request, async {
        Err(RuntimeError::FileError {
            message: "ready provider failure".to_string(),
        })
    })
    .await
    .into_result()
    .expect_err("pre-cancelled request should win the biased select");
    assert!(matches!(error, RuntimeError::Cancelled));
}

#[test]
fn selected_deadline_does_not_downgrade_after_late_cancellation() {
    let execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ));
    execution.cancellation_token().cancel();

    assert!(matches!(
        deadline_error(&execution),
        RuntimeError::ExecutionBudgetExceeded {
            reason: crate::error::BudgetReason::DeadlineExceeded,
            ..
        }
    ));
}

#[tokio::test]
async fn provider_stream_normal_completion_remains_provider_terminal() {
    let diagnostic_generation = u64::MAX - 1;
    start_restricted_service_diagnostic_probe_for_test(diagnostic_generation);
    let (_consumer_cancel, stream_cancel) = test_stream_cancel();
    let execution = test_runtime::execution_control().owned();
    let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
        Ok(RuntimeValue::Bool(true))
    })
    .await;

    assert!(matches!(
        terminal,
        ProviderTerminal::Provider(Ok(RuntimeValue::Bool(true)))
    ));
    assert!(
        take_restricted_service_diagnostics_for_test(diagnostic_generation).is_empty(),
        "successful provider completion must not submit a failure diagnostic"
    );
}

#[tokio::test]
async fn provider_stream_consumer_cancel_is_control_terminal() {
    let (consumer_cancel, stream_cancel) = test_stream_cancel();
    let execution = test_runtime::execution_control().owned();
    consumer_cancel.cancel();
    let terminal =
        await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending()).await;

    assert!(matches!(terminal, ProviderTerminal::ConsumerCancelled));
}

#[tokio::test]
async fn provider_stream_request_cancel_is_control_terminal() {
    let (_consumer_cancel, stream_cancel) = test_stream_cancel();
    let execution = test_runtime::execution_control().owned();
    execution.cancellation_token().cancel();
    let terminal =
        await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending()).await;

    assert!(matches!(terminal, ProviderTerminal::RequestCancelled));
}

#[tokio::test]
async fn provider_stream_control_ordering_precedes_ready_provider_error() {
    let (consumer_cancel, stream_cancel) = test_stream_cancel();
    let execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ))
    .owned();
    consumer_cancel.cancel();
    execution.cancellation_token().cancel();
    let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
        Err(RuntimeError::FileError {
            message: "ready provider error".to_string(),
        })
    })
    .await;
    assert!(matches!(terminal, ProviderTerminal::ConsumerCancelled));

    let (_consumer_cancel, stream_cancel) = test_stream_cancel();
    let execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ))
    .owned();
    execution.cancellation_token().cancel();
    let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
        Err(RuntimeError::FileError {
            message: "ready provider error".to_string(),
        })
    })
    .await;
    assert!(matches!(terminal, ProviderTerminal::RequestCancelled));
}

#[tokio::test]
async fn f445h_e4r7_stream_deadline_helper_matrix_preserves_carrier_before_raw_boundary() {
    let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
    let execution = test_runtime::execution_control_with_deadline(Some(request_deadline)).owned();
    let request_scope = execution
        .execution_scope()
        .expect("test execution exposes its current request scope");
    let item_request =
        RequestActivationContext::begin(activation("stream-deadline", "stream-build")).unwrap();
    let (_consumer_cancel, stream_cancel) = test_stream_cancel();

    let terminal =
        await_provider_stream_terminal(&execution, &stream_cancel, std::future::pending()).await;
    let ProviderTerminal::DeadlineExceeded(error) = terminal else {
        panic!("provider terminal must preserve the request deadline carrier");
    };
    assert_inherited_request_deadline(&error, &request_scope, request_deadline);

    let item_error = await_stream_item_publication(
        &execution,
        &item_request,
        std::future::pending::<StreamRuntimeResult<()>>(),
    )
    .await
    .expect_err("stream item publication must wake on deadline");
    assert!(matches!(item_error, StreamRuntimeError::Cancelled));
    assert!(
        item_request.open_stream().is_none(),
        "stream item deadline must cancel the provider request"
    );

    let publication_request =
        RequestActivationContext::begin(activation("publication-deadline", "stream-build"))
            .unwrap();
    let publication = await_provider_publication(
        &execution,
        &stream_cancel,
        &publication_request,
        std::future::pending(),
    )
    .await;
    let ProviderPublication::DeadlineExceeded(error) = publication else {
        panic!("terminal publication must preserve the request deadline carrier");
    };
    assert_inherited_request_deadline(&error, &request_scope, request_deadline);
    assert!(
        publication_request.open_stream().is_none(),
        "stream terminal publication deadline must cancel the provider request"
    );
}

#[tokio::test]
async fn stream_item_and_publication_request_cancel_precede_expired_deadline() {
    let execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ))
    .owned();
    execution.cancellation_token().cancel();
    let item_request =
        RequestActivationContext::begin(activation("item-cancel-race", "stream-build")).unwrap();

    let item_error = await_stream_item_publication(
        &execution,
        &item_request,
        std::future::pending::<StreamRuntimeResult<()>>(),
    )
    .await
    .expect_err("request cancellation must wake a pending item publication");
    assert!(matches!(
        RuntimeError::from(item_error),
        RuntimeError::Cancelled
    ));
    assert!(
        item_request.open_stream().is_none(),
        "item cancellation must propagate to the provider request"
    );

    let publication_request =
        RequestActivationContext::begin(activation("publication-cancel-race", "stream-build"))
            .unwrap();
    let (_consumer_cancel, stream_cancel) = test_stream_cancel();
    let publication = await_provider_publication(
        &execution,
        &stream_cancel,
        &publication_request,
        std::future::pending(),
    )
    .await;
    assert!(matches!(publication, ProviderPublication::RequestCancelled));
    assert!(
        publication_request.open_stream().is_none(),
        "publication cancellation must propagate to the provider request"
    );
}

#[tokio::test]
async fn f445h_e4r7_stream_deadline_provider_terminal_reaches_raw_consumer_as_cancelled() {
    let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
    stream_runtime.cancel(&stream_value);
    let lifetime_drops = Arc::new(AtomicUsize::new(0));
    let (stream_value, sink) = stream_runtime.channel_stream_with_lifetime(
        StreamLifetimeGuard::new(TestLifetimeDrop(Arc::clone(&lifetime_drops))),
    );
    task.stream_value = stream_value.clone();
    task.stream_cancel = sink.cancel_signal();
    task.sink = sink;
    let request_deadline = Instant::now() - std::time::Duration::from_millis(1);
    task.execution = test_runtime::execution_control_with_deadline(Some(request_deadline)).owned();
    let request_scope = task
        .execution
        .execution_scope()
        .expect("test execution exposes its current request scope");
    let provider_request = task.request.clone();
    let mut consumer = Box::pin(stream_runtime.next(&stream_value));
    assert!(
        matches!(first_poll(consumer.as_mut()), Poll::Pending),
        "raw consumer must attach and enter a real Pending poll before the terminal"
    );

    let terminal = await_provider_stream_terminal(
        &task.execution,
        &task.stream_cancel,
        std::future::pending(),
    )
    .await;
    let ProviderTerminal::DeadlineExceeded(error) = &terminal else {
        panic!("provider wait must preserve the request deadline carrier");
    };
    assert_inherited_request_deadline(error, &request_scope, request_deadline);
    finish_provider_stream(task, terminal).await;

    let error = consumer
        .await
        .expect_err("raw stream boundary must project the deadline carrier to cancellation");
    assert!(
        matches!(error, StreamRuntimeError::Cancelled),
        "unexpected stream terminal: {error:?}"
    );
    assert!(
        provider_request.open_stream().is_none(),
        "deadline terminal must cancel the provider request"
    );
    assert_eq!(
        lifetime_drops.load(Ordering::Acquire),
        1,
        "raw cancellation consumption must release the stream lifetime exactly once"
    );
}

#[tokio::test]
async fn f445h_e4r7_stream_deadline_item_publication_reaches_attached_raw_consumer_as_cancelled() {
    let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
    let provider_request = task.request.clone();
    let mut consumer = Box::pin(stream_runtime.next(&stream_value));
    assert!(
        matches!(first_poll(consumer.as_mut()), Poll::Pending),
        "raw consumer must attach and enter a real Pending poll before the deadline"
    );

    task.execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ))
    .owned();
    let item_error = await_stream_item_publication(
        &task.execution,
        &provider_request,
        std::future::pending::<StreamRuntimeResult<()>>(),
    )
    .await
    .expect_err("item publication must project its request deadline to raw cancellation");
    assert!(matches!(item_error, StreamRuntimeError::Cancelled));

    finish_provider_stream(
        task,
        ProviderTerminal::Provider(Err(RuntimeError::from(item_error))),
    )
    .await;

    let error = consumer
        .await
        .expect_err("attached raw consumer must observe internal cancellation");
    assert!(matches!(error, StreamRuntimeError::Cancelled));
    assert!(
        provider_request.open_stream().is_none(),
        "item deadline must cancel the provider request"
    );
}

#[tokio::test]
async fn f445h_e4r7_stream_deadline_blocked_terminal_preserves_buffered_item_then_cancelled() {
    let (mut task, _, stream_runtime, stream_value, _) = provider_stream_failure_task();
    stream_runtime.cancel(&stream_value);
    let lifetime_drops = Arc::new(AtomicUsize::new(0));
    let (stream_value, sink) = stream_runtime.channel_stream_with_lifetime(
        StreamLifetimeGuard::new(TestLifetimeDrop(Arc::clone(&lifetime_drops))),
    );
    task.stream_value = stream_value.clone();
    task.stream_cancel = sink.cancel_signal();
    task.sink = sink;
    task.sink
        .send(serde_json::json!("buffered-before-terminal"))
        .await
        .expect("test stream buffer must be full before terminal publication");
    task.execution = test_runtime::execution_control_with_deadline(Some(
        Instant::now() - std::time::Duration::from_millis(1),
    ))
    .owned();
    let provider_request = task.request.clone();
    let mut publication = Box::pin(publish_provider_terminal(
        &task,
        ProviderStreamPublication::End,
    ));
    assert!(
        matches!(first_poll(publication.as_mut()), Poll::Pending),
        "the raw cancellation terminal must block behind the full stream buffer"
    );
    assert!(
        provider_request.open_stream().is_none(),
        "publication deadline must cancel the provider request"
    );

    let first = stream_runtime
        .next(&stream_value)
        .await
        .expect("buffered item must remain visible before the deadline terminal");
    assert!(matches!(
        first,
        StreamPoll::Item(value)
            if value == serde_json::json!("buffered-before-terminal")
    ));
    publication.await;

    let error = stream_runtime
        .next(&stream_value)
        .await
        .expect_err("deadline must replace the blocked End with raw cancellation");
    assert!(matches!(error, StreamRuntimeError::Cancelled));
    assert_eq!(
        lifetime_drops.load(Ordering::Acquire),
        1,
        "raw cancellation consumption must release the stream lifetime exactly once"
    );
}

#[tokio::test]
async fn restricted_service_diagnostic_server_stream_failure_submits_once() {
    let (task, generation, stream_runtime, stream_value, _) = provider_stream_failure_task();

    start_restricted_service_diagnostic_probe_for_test(generation);
    run_provider_stream(task).await;

    let terminal = stream_runtime
        .next(&stream_value)
        .await
        .expect_err("provider stream should publish the fixed failure");
    let (fixed, _) = terminal
        .fixed_service_failure_parts()
        .expect("stream terminal retains typed fixed failure");
    let diagnostics = take_restricted_service_diagnostics_for_test(generation);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].correlation.trace_id,
        fixed.envelope().trace_id()
    );
    assert_eq!(
        diagnostics[0].correlation.error_id,
        fixed.envelope().error_id()
    );
}

#[tokio::test]
async fn restricted_service_diagnostic_server_stream_request_cancel_submits_zero() {
    let (task, generation, _, _, cancellation) = provider_stream_failure_task();
    cancellation.cancel();
    start_restricted_service_diagnostic_probe_for_test(generation);

    run_provider_stream(task).await;

    assert!(
        take_restricted_service_diagnostics_for_test(generation).is_empty(),
        "request cancellation must bypass provider failure export"
    );
}

#[test]
fn ordinary_service_call_rebinds_websocket_capability_to_provider_activation() {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let receiver_target = fixture.caller_eval_target();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
        receiver_target.execution_image(),
    ));
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let instruction = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call } => match &call.target {
                LinkedCallTarget::ActivationRelativeService { instruction } => Some(instruction),
                _ => None,
            },
            _ => None,
        })
        .expect("linked ordinary service call");
    let target = receiver_target
        .resolve_service_call(instruction)
        .expect("resolved provider target");
    let receiver_context = fixture.execution_context(&interpreter, receiver_target);
    assert_eq!(
        receiver_context.websocket_context().service_id(),
        "test-service"
    );

    let provider_context =
        provider_execution_context(&receiver_context, &target).expect("provider context");
    assert_eq!(
        provider_context.websocket_context().service_id(),
        target
            .provider_activation()
            .identity()
            .deployment
            .service_id
            .as_str()
    );
    assert_eq!(
        provider_context.websocket_context().websocket_entry_id(),
        target
            .provider_activation()
            .websocket_entry_id()
            .map(|entry| entry.as_str())
    );
    assert_ne!(
        provider_context.websocket_context().service_id(),
        receiver_context.websocket_context().service_id()
    );

    let owned = OwnedProgramExecutionContext::capture(&provider_context);
    let borrowed = owned.borrow();
    assert_eq!(
        borrowed.websocket_context().service_id(),
        provider_context.websocket_context().service_id()
    );
    assert_eq!(
        borrowed.websocket_context().websocket_entry_id(),
        provider_context.websocket_context().websocket_entry_id()
    );
}

pub(super) fn provider_stream_failure_task() -> (
    ProviderStreamTask,
    u64,
    StreamRuntime,
    Value,
    CancellationToken,
) {
    provider_stream_failure_task_with_parent_depth(0)
}

fn provider_stream_failure_task_with_parent_depth(
    parent_depth: usize,
) -> (
    ProviderStreamTask,
    u64,
    StreamRuntime,
    Value,
    CancellationToken,
) {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let receiver_target = fixture.caller_eval_target();
    let generation = receiver_target.request_activation().generation();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
        receiver_target.execution_image(),
    ));
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller executable");
    let call = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call }
                if matches!(
                    call.target,
                    LinkedCallTarget::ActivationRelativeService { .. }
                ) =>
            {
                Some(call.clone())
            }
            _ => None,
        })
        .expect("linked service call");
    let instruction = match &call.target {
        LinkedCallTarget::ActivationRelativeService { instruction } => instruction,
        _ => unreachable!("selected call is activation-relative"),
    };
    let target = receiver_target
        .resolve_service_call(instruction)
        .expect("resolved provider target");
    let receiver_context = fixture
        .execution_context(&interpreter, receiver_target)
        .with_program_call_depth_for_test(parent_depth);
    let mut provider_context =
        provider_execution_context(&receiver_context, &target).expect("provider context");
    let provider_stream_owner = provider_context.take_stream_runtime_owner();
    let stream_runtime = provider_context.stream_runtime();
    let (stream_value, sink) = stream_runtime.channel_stream();
    let stream_cancel = sink.cancel_signal();
    let caller_stack_at_site = receiver_context.exception_stack_for_site(call.site.clone());
    let execution = receiver_context.execution().owned();
    let cancellation = execution.cancellation_token();
    let task = ProviderStreamTask {
        interpreter: interpreter.clone_for_stream_producer(),
        provider_context: Arc::new(OwnedProgramExecutionContext::capture(&provider_context)),
        provider_heap: RequestHeap::default(),
        provider_env: Env::new(),
        caller_addr: fixture.caller_addr().clone(),
        caller_package_build_id: fixture.caller_build().clone(),
        call_site: call.site.clone(),
        caller_stack_at_site,
        provider_addr: target.executable_addr().clone(),
        provider_receiver_const: target.receiver_const().cloned(),
        provider_package_build_id: target
            .provider_activation()
            .implementation_package_build_id()
            .clone(),
        provider_service_id: target
            .provider_activation()
            .identity()
            .deployment
            .service_id
            .clone(),
        operation_id: target.descriptor().operation_id.as_str().to_string(),
        type_args: call.type_args.clone(),
        args: Vec::new(),
        stream_value: stream_value.clone(),
        sink,
        stream_cancel,
        execution,
        request: target.provider_request().clone(),
        _stream_runtime_owner: provider_stream_owner,
        activity_probe: None,
        depth_probe: None,
    };
    (task, generation, stream_runtime, stream_value, cancellation)
}

#[tokio::test]
async fn provider_stream_spawn_resets_only_the_callable_depth() {
    const PARENT_DEPTH: usize = 17;

    let (mut task, _, stream_runtime, stream_value, _) =
        provider_stream_failure_task_with_parent_depth(PARENT_DEPTH);
    assert_eq!(
        task.provider_context.borrow().program_call_depth_for_test(),
        PARENT_DEPTH,
        "provider derivation and capture must retain the active parent depth"
    );

    let activity_probe = Arc::new(ProviderStreamTaskActivityProbe::default());
    let depth_probe = Arc::new(ProviderStreamTaskDepthProbe::default());
    task.activity_probe = Some(Arc::clone(&activity_probe));
    task.depth_probe = Some(Arc::clone(&depth_probe));

    spawn_provider_stream(task);
    stream_runtime
        .next(&stream_value)
        .await
        .expect_err("the fixed provider failure must reach the stream consumer");

    assert_eq!(
        activity_probe.entered(),
        1,
        "the provider callable must run in the spawned task"
    );
    assert_eq!(
        depth_probe.callable_entry(),
        Some(0),
        "the spawned provider callable must enter with fresh task depth"
    );
    assert_eq!(
        depth_probe.error_export(),
        Some(PARENT_DEPTH),
        "error export is an ordinary continuation and must retain captured depth"
    );
    assert_provider_stream_depth_borrow_sites();
}

#[test]
fn in_process_stream_task_probe_returns_to_zero_exactly_once() {
    let activity_probe = Arc::new(ProviderStreamTaskActivityProbe::default());
    let mut task = provider_stream_failure_task().0;
    task.activity_probe = Some(Arc::clone(&activity_probe));
    let guard = ProviderStreamTaskGuard::for_task(&task);
    assert_eq!(activity_probe.entered(), 1);
    assert_eq!(activity_probe.active(), 1);
    drop(guard);
    assert_eq!(activity_probe.active(), 0);
}

#[tokio::test]
async fn activation_context_across_suspend_keeps_explicit_provider_then_restores_receiver() {
    let receiver = activation("receiver", "receiver-build");
    let provider = activation("provider", "provider-build");
    let receiver_request = RequestActivationContext::begin(Arc::clone(&receiver)).unwrap();
    let provider_request = receiver_request.switch_to(Arc::clone(&provider)).unwrap();
    let generation = receiver_request.generation();

    let resumed = tokio::spawn(async move {
        assert!(Arc::ptr_eq(provider_request.current(), &provider));
        tokio::task::yield_now().await;
        assert!(Arc::ptr_eq(provider_request.current(), &provider));
        provider_request.restore_receiver()
    })
    .await
    .expect("owned provider continuation should not panic");

    assert!(Arc::ptr_eq(resumed.current(), &receiver));
    assert_eq!(resumed.generation(), generation);
}

#[test]
fn in_process_stream_items_use_canonical_detached_plan() {
    let ty = ContractTypeRef::Record {
        fields: BTreeMap::from([
            (
                "first".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::builtin("string")],
                },
            ),
            (
                "second".to_string(),
                ContractTypeRef::Builtin {
                    name: "Array".to_string(),
                    arguments: vec![ContractTypeRef::builtin("string")],
                },
            ),
        ]),
    };
    let plan = BoundaryValuePlan::Linkable {
        carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
        encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Stream,
    };
    let mut provider_heap = RequestHeap::default();
    let shared = provider_heap
        .alloc_array(vec![RuntimeValue::String("provider".to_string())])
        .unwrap();
    let root = provider_heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("first".to_string(), RuntimeValue::Heap(shared)),
            ("second".to_string(), RuntimeValue::Heap(shared)),
        ])))
        .unwrap();
    let mut receiver_heap = RequestHeap::default();

    let item = materialize_value(
        "stream item",
        &ty,
        &BTreeMap::new(),
        &plan,
        &RuntimeValue::Heap(root),
        &provider_heap,
        &mut receiver_heap,
        canonical_scope(
            &plan,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
        )
        .unwrap(),
        &FailClosedServiceLinkableCapabilityHooks,
    )
    .unwrap();
    let RuntimeValue::Heap(root) = item else {
        panic!("materialized stream item should remain a record")
    };
    let HeapNode::Object(object) = receiver_heap.get(root).unwrap() else {
        panic!("materialized stream item should remain an object")
    };
    let RuntimeValue::Heap(first) = object.fields()["first"] else {
        panic!("first stream field should be an array")
    };
    let RuntimeValue::Heap(second) = object.fields()["second"] else {
        panic!("second stream field should be an array")
    };
    assert_ne!(first, second, "stream item aliases must be detached");
    receiver_heap
        .set_array_item(first, 0, RuntimeValue::String("receiver".to_string()))
        .unwrap();
    let HeapNode::Array(provider_items) = provider_heap.get(shared).unwrap() else {
        panic!("provider array should remain allocated")
    };
    assert_eq!(
        provider_items,
        &[RuntimeValue::String("provider".to_string())],
        "consumer mutation must not reach the provider heap"
    );
}

#[test]
fn in_process_stream_typed_emit_decodes_before_canonical_plan() {
    let execution_plan =
        RuntimeTypePlan::synthetic_named_builtin("Date", RuntimeTypeNode::Date, Vec::new());
    let mut provider_heap = RequestHeap::default();
    let wire = runtime_to_wire_required_plan(
        &RuntimeValue::Date(1_234),
        Some(&execution_plan),
        "provider stream emit item",
        &mut provider_heap,
    )
    .unwrap();
    let mut decoded_heap = RequestHeap::default();
    let decoded = runtime_from_wire_required_plan(
        &wire,
        Some(&execution_plan),
        "provider stream emit item",
        &mut decoded_heap,
    )
    .unwrap();
    let plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Stream,
    };
    let mut receiver_heap = RequestHeap::default();

    let materialized = materialize_value(
        "stream item",
        &ContractTypeRef::builtin("Date"),
        &BTreeMap::new(),
        &plan,
        &decoded,
        &decoded_heap,
        &mut receiver_heap,
        canonical_scope(
            &plan,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
        )
        .unwrap(),
        &FailClosedServiceLinkableCapabilityHooks,
    )
    .unwrap();

    assert_eq!(materialized, RuntimeValue::Date(1_234));
}

#[test]
fn in_process_stream_named_item_uses_admitted_package_record() {
    let type_id = PackageSchemaTypeId::new("schema:stream-item");
    let ty = ContractTypeRef::package_schema("example.stream", "api.StreamItem", type_id.clone());
    let schema = BTreeMap::from([(
        type_id.clone(),
        Arc::new(PackageSchemaTypeRecord {
            package_id: "example.stream".to_string(),
            stable_schema_key: "api.StreamItem".to_string(),
            package_schema_type_id: type_id,
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            },
        }),
    )]);
    let plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Provider,
        lifetime: BoundaryValueLifetime::Stream,
    };
    let mut provider_heap = RequestHeap::default();
    let source = provider_heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))
        .unwrap();
    let mut receiver_heap = RequestHeap::default();

    materialize_value(
        "stream item",
        &ty,
        &schema,
        &plan,
        &RuntimeValue::Heap(source),
        &provider_heap,
        &mut receiver_heap,
        canonical_scope(
            &plan,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
        )
        .unwrap(),
        &FailClosedServiceLinkableCapabilityHooks,
    )
    .expect("admitted Package stream item should materialize");

    assert!(materialize_value(
        "stream item",
        &ty,
        &BTreeMap::new(),
        &plan,
        &RuntimeValue::Heap(source),
        &provider_heap,
        &mut receiver_heap,
        canonical_scope(
            &plan,
            BoundaryValueOwner::Provider,
            BoundaryValueLifetime::Stream,
        )
        .unwrap(),
        &FailClosedServiceLinkableCapabilityHooks,
    )
    .is_err());
}

pub(super) fn activation(service: &str, package_build: &str) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity: AssemblyIdentity::new("assembly:async-stream"),
            assembly_generation: 7,
            runtime_replica_id: "replica:async-stream".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: service.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new(format!("{service}-r1")),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                    "deployment:{service}"
                )),
            },
        },
        PackageBuildId::new(package_build),
        Vec::new(),
    )
    .unwrap()
}
