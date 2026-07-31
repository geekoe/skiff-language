use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_boundary::http::HttpBoundaryResponseStreamEvent;
use skiff_runtime_capability_context::{
    CancellationToken, DbCapabilityContext, ExecutionControl, FileSourceStreamContext,
    RequestPayloadContext, StreamCancelSignal, StreamLifetimeGuard, StreamPoll, StreamPullSource,
    StreamRuntime, StreamRuntimeApi, StreamRuntimeResult, StreamSink, TimeCapabilityContext,
};
use skiff_runtime_linked_program::ServiceMeta;
use skiff_runtime_model::{request_heap::RequestHeapLimits, type_plan::RuntimeTypePlan};

use super::*;
use crate::{
    assembly_execution::ordinary::tests::test_runtime, error::RuntimeError,
    program_execution::ProgramExecutionInput, stream_callback::EvalStreamExecutionError,
};

#[derive(Debug)]
struct InvocationStreamRuntime {
    outcomes: Mutex<VecDeque<StreamRuntimeResult<StreamPoll>>>,
    cancellations: Arc<AtomicUsize>,
    cancel_token_count: Arc<AtomicUsize>,
}

impl InvocationStreamRuntime {
    fn new(
        outcomes: impl IntoIterator<Item = StreamRuntimeResult<StreamPoll>>,
    ) -> (StreamRuntime, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let cancellations = Arc::new(AtomicUsize::new(0));
        let cancel_token_count = Arc::new(AtomicUsize::new(usize::MAX));
        let runtime = StreamRuntime::new(Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            cancellations: Arc::clone(&cancellations),
            cancel_token_count: Arc::clone(&cancel_token_count),
        });
        (runtime, cancellations, cancel_token_count)
    }

    fn next_result<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        match self
            .outcomes
            .lock()
            .expect("invocation stream mutex poisoned")
            .pop_front()
        {
            Some(outcome) => Box::pin(std::future::ready(outcome)),
            None => Box::pin(std::future::pending()),
        }
    }
}

impl StreamRuntimeApi for InvocationStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!("the invocation fixture only consumes an existing stream")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!("the invocation fixture only consumes an existing stream")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        unreachable!("the invocation fixture only consumes an existing stream")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!("the invocation fixture only consumes an existing stream")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.next_result()
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.cancel_token_count
            .store(cancel_tokens.len(), Ordering::Release);
        self.next_result()
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.next_result()
    }

    fn cancel(&self, _value: &Value) {
        self.cancellations.fetch_add(1, Ordering::AcqRel);
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn execution_input<'a>(
    interpreter: &Interpreter,
    runtime: StreamRuntime,
    execution: ExecutionControl<'a>,
) -> ProgramExecutionInput<'a> {
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    let request = test_runtime::request_context();
    ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: FileSourceStreamContext::new(runtime.clone(), execution.clone()),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            runtime,
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    }
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    runtime: StreamRuntime,
    execution: ExecutionControl<'a>,
) -> ProgramExecutionContext<'a> {
    ProgramExecutionContext::new(execution_input(interpreter, runtime, execution))
}

#[tokio::test]
async fn f445h_e4r_stream_invocation_binary_loop_natural_end_disarms_cleanup() {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (runtime, cancellations, cancel_token_count) =
        InvocationStreamRuntime::new([Ok(StreamPoll::End)]);
    let context = execution_context(&interpreter, runtime, test_runtime::execution_control());
    let mut events = Vec::<HttpBoundaryResponseStreamEvent>::new();
    interpreter
        .consume_binary_http_response_stream_with_context(
            &context,
            &json!({"$stream": "binary-natural-end"}),
            &RuntimeTypePlan::json_value_plan(),
            &[],
            false,
            None,
            &mut |event| {
                events.push(event);
                Ok::<_, ()>(())
            },
        )
        .await
        .expect("runtime End closes the binary response loop");
    assert!(events.is_empty());
    assert_eq!(cancellations.load(Ordering::Acquire), 0);
    assert_eq!(cancel_token_count.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn f445h_e4r_stream_invocation_runtime_loop_preserves_local_deadline_owner() {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (runtime, cancellations, _) = InvocationStreamRuntime::new(Vec::new());
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("expired local deadline");
    let root = test_runtime::execution_control();
    let owned = root
        .derive_scope(deadline, site())
        .expect("derive invocation local scope");
    let current_scope = owned.execution_scope().expect("current invocation scope");
    let request = RequestPayloadContext::new("f445h-e4r-runtime-stream", &[], None);
    let context = ProgramInvocationContext::new(ProgramInvocationInput {
        request,
        operation: "stream",
        execution: execution_input(&interpreter, runtime, owned.borrow()),
        http_response_max_bytes: 1024,
        request_heap_limits: RequestHeapLimits::default(),
    });
    let error = interpreter
        .consume_runtime_response_stream(
            &context,
            &json!({"$stream": "runtime-local-deadline"}),
            &RuntimeTypePlan::json_value_plan(),
            &[],
            &mut |_item, _plan| Ok::<_, ()>(()),
        )
        .await
        .expect_err("current local deadline must stop the runtime response loop");
    let EvalStreamExecutionError::Eval(RuntimeError::ScopeTerminal(carrier)) = error else {
        panic!("invocation loop must preserve the internal scope terminal carrier");
    };
    assert_eq!(carrier.effective_deadline().at(), deadline);
    assert!(carrier.is_owned_by(&current_scope));
    assert_eq!(cancellations.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn f445h_e4r_stream_invocation_runtime_loop_delivers_item_then_natural_end() {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (runtime, cancellations, cancel_token_count) = InvocationStreamRuntime::new([
        Ok(StreamPoll::Item(json!({"value": 1}))),
        Ok(StreamPoll::End),
    ]);
    let request = RequestPayloadContext::new("f445h-e4r-runtime-stream", &[], None);
    let context = ProgramInvocationContext::new(ProgramInvocationInput {
        request,
        operation: "stream",
        execution: execution_input(&interpreter, runtime, test_runtime::execution_control()),
        http_response_max_bytes: 1024,
        request_heap_limits: RequestHeapLimits::default(),
    });
    let mut items = Vec::new();
    interpreter
        .consume_runtime_response_stream(
            &context,
            &json!({"$stream": "runtime-item-end"}),
            &RuntimeTypePlan::json_value_plan(),
            &[],
            &mut |item, _plan| {
                items.push(item);
                Ok::<_, ()>(())
            },
        )
        .await
        .expect("runtime response loop reaches natural End");
    assert_eq!(items, vec![json!({"value": 1})]);
    assert_eq!(cancellations.load(Ordering::Acquire), 0);
    assert_eq!(cancel_token_count.load(Ordering::Acquire), 0);
}

pub(super) async fn logical_end_cleanup_count() -> usize {
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let (runtime, cancellations, _) =
        InvocationStreamRuntime::new([Ok(StreamPoll::Item(json!({"tag": "end"})))]);
    let context = execution_context(&interpreter, runtime, test_runtime::execution_control());
    let mut events = Vec::new();
    interpreter
        .consume_binary_http_response_stream_with_context(
            &context,
            &json!({"$stream": "binary-logical-end"}),
            &RuntimeTypePlan::json_value_plan(),
            &[],
            false,
            None,
            &mut |event| {
                events.push(event);
                Ok::<_, ()>(())
            },
        )
        .await
        .expect("logical HTTP End exits the response loop");
    assert_eq!(events, vec![HttpBoundaryResponseStreamEvent::End]);
    cancellations.load(Ordering::Acquire)
}
