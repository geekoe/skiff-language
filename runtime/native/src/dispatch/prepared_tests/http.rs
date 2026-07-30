use super::*;

#[derive(Clone)]
pub(super) struct TestHttpClient {
    response: Value,
    first_pending: bool,
    calls: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl NativeHttpClientCapability for TestHttpClient {
    fn dispatch_test_http_effect_invocation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        None
    }

    fn dispatch_http_request<'a>(&'a self, _input: &'a Value) -> NativeCapabilityFuture<'a, Value> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(PendingOnce::new(
            self.response.clone(),
            self.first_pending,
            Arc::clone(&self.polls),
            Arc::clone(&self.drops),
        ))
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        _input: &'a Value,
        _expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("HTTP stream is not under test")
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        _input: &'a Value,
        _expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        panic!("HTTP SSE is not under test")
    }
}

pub(super) struct NoHttpResponseStream;

impl NativeHttpResponseStreamCapability for NoHttpResponseStream {
    fn response_item_type(&self, _target: &str) -> Result<RuntimeTypePlan> {
        panic!("response stream is not under test")
    }

    fn send_response_event<'a>(
        &'a self,
        _target: &'a str,
        _event: Value,
    ) -> NativeCapabilityFuture<'a, ()> {
        panic!("response stream is not under test")
    }
}

#[derive(Clone)]
struct RecordingHttpResponseStream {
    calls: Arc<AtomicUsize>,
}

impl NativeHttpResponseStreamCapability for RecordingHttpResponseStream {
    fn response_item_type(&self, _target: &str) -> Result<RuntimeTypePlan> {
        Ok(scalar_plan("number", RuntimeTypeNode::Number))
    }

    fn send_response_event<'a>(
        &'a self,
        _target: &'a str,
        _event: Value,
    ) -> NativeCapabilityFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async { Ok(()) })
    }
}

fn unpolled_http_client(calls: Arc<AtomicUsize>) -> TestHttpClient {
    TestHttpClient {
        response: Value::Null,
        first_pending: false,
        calls,
        polls: Arc::new(AtomicUsize::new(0)),
        drops: Arc::new(AtomicUsize::new(0)),
    }
}

fn http_request_invocation(return_plan: RuntimeTypePlan) -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        TARGET_STD_HTTP_REQUEST.to_string(),
        TARGET_STD_HTTP_REQUEST,
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static(TARGET_STD_HTTP_REQUEST),
            vec![scalar_plan("number", RuntimeTypeNode::Number)],
            return_plan,
            NativeRequiredContext::HttpClient,
        )),
        None,
        None,
    )
}

#[test]
fn prepared_http_wait_is_heap_free_starts_once_and_writes_only_at_finalize() {
    let calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let client = TestHttpClient {
        response: json!(7),
        first_pending: true,
        calls: Arc::clone(&calls),
        polls: Arc::clone(&polls),
        drops: Arc::clone(&drops),
    };
    let mut heap = RequestHeap::default();
    let prepared =
        HttpNativeDispatch::new()
            .prepare(
                NativeCapabilityContexts::<
                    (),
                    (),
                    (),
                    TestHttpClient,
                    NoHttpResponseStream,
                    (),
                    (),
                    (),
                >::HttpClient(client),
                http_request_invocation(scalar_plan("number", RuntimeTypeNode::Number)),
                TARGET_STD_HTTP_REQUEST.to_string(),
                vec![RuntimeValue::Number(1.0)],
                &mut heap,
            )
            .expect("HTTP request should prepare");

    assert_eq!(calls.load(Ordering::Acquire), 0);
    heap.alloc_bytes(b"independent caller mutation".to_vec())
        .expect("wait does not borrow caller heap");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("real HTTP request must expose an external wait");
    };
    let (mut wait, finalize) = operation.into_parts();
    let before_wait = heap.checkpoint();
    assert!(matches!(poll_external_wait(&mut wait), Poll::Pending));
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_eq!(heap.checkpoint(), before_wait);

    let Poll::Ready(outcome) = poll_external_wait(&mut wait) else {
        panic!("second poll should complete the owned HTTP wait");
    };
    let outcome = outcome.expect("HTTP wait succeeds");
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert_eq!(polls.load(Ordering::Acquire), 2);
    assert_eq!(heap.checkpoint(), before_wait);
    assert_eq!(
        finalize
            .finalize(outcome, &mut heap)
            .expect("HTTP response finalizes"),
        RuntimeValue::Number(7.0)
    );
    drop(wait);
    assert_eq!(drops.load(Ordering::Acquire), 1);
}

#[test]
fn prepared_http_finalize_failure_rolls_back_partial_heap_materialization() {
    let calls = Arc::new(AtomicUsize::new(0));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let client = TestHttpClient {
        response: Value::Array(vec![bytes_value(b"one")]),
        first_pending: false,
        calls,
        polls,
        drops,
    };
    let limits = RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::new(limits);
    let prepared =
        HttpNativeDispatch::new()
            .prepare(
                NativeCapabilityContexts::<
                    (),
                    (),
                    (),
                    TestHttpClient,
                    NoHttpResponseStream,
                    (),
                    (),
                    (),
                >::HttpClient(client),
                http_request_invocation(RuntimeTypePlan::synthetic_array(scalar_plan(
                    "bytes",
                    RuntimeTypeNode::Bytes,
                ))),
                TARGET_STD_HTTP_REQUEST.to_string(),
                vec![RuntimeValue::Number(1.0)],
                &mut heap,
            )
            .expect("HTTP request should prepare");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("HTTP request must expose a wait");
    };
    let (mut wait, finalize) = operation.into_parts();
    let Poll::Ready(outcome) = poll_external_wait(&mut wait) else {
        panic!("fixture response is immediately ready");
    };
    let outcome = outcome.expect("fixture wait succeeds");
    let before_finalize = heap.checkpoint();
    let error = finalize
        .finalize(outcome, &mut heap)
        .expect_err("outer Array allocation must exceed the one-node limit");
    assert!(matches!(error, RuntimeError::ResourceLimitExceeded { .. }));
    assert_eq!(heap.checkpoint(), before_finalize);
}

#[test]
fn http_stream_sse_and_response_emit_prepare_owned_waits_without_starting_them() {
    let client_calls = Arc::new(AtomicUsize::new(0));
    let response_calls = Arc::new(AtomicUsize::new(0));
    let mut heap = RequestHeap::default();
    let number = scalar_plan("number", RuntimeTypeNode::Number);
    let stream_return =
        RuntimeTypePlan::synthetic_request_record(vec![RuntimeRecordFieldPlan::new(
            "body",
            RuntimeTypePlan::synthetic_stream(number.clone()),
            true,
        )]);
    let sse_return = RuntimeTypePlan::synthetic_stream(number.clone());

    for (target, return_plan) in [
        (TARGET_STD_HTTP_STREAM, stream_return),
        (TARGET_STD_HTTP_SSE, sse_return),
    ] {
        let invocation = RuntimeNativeInvocation::new(
            target.to_string(),
            target,
            Some(NativeCallPlan::new(
                NativeBindingKey::from_static(target),
                vec![number.clone()],
                return_plan,
                NativeRequiredContext::HttpClient,
            )),
            None,
            None,
        );
        let prepared = HttpNativeDispatch::new()
            .prepare(
                NativeCapabilityContexts::<
                    (),
                    (),
                    (),
                    TestHttpClient,
                    NoHttpResponseStream,
                    (),
                    (),
                    (),
                >::HttpClient(unpolled_http_client(Arc::clone(&client_calls))),
                invocation,
                target.to_string(),
                vec![RuntimeValue::Number(1.0)],
                &mut heap,
            )
            .unwrap_or_else(|error| panic!("{target} should prepare: {error}"));
        assert!(
            matches!(prepared, PreparedNativeCall::ExternalWait(_)),
            "{target} must expose an owned wait"
        );
        heap.alloc_bytes(target.as_bytes().to_vec())
            .expect("caller heap remains independently mutable");
        drop(prepared);
    }

    let emit_target = "std.http.stream.emitResponse";
    let emit_invocation = RuntimeNativeInvocation::new(
        emit_target.to_string(),
        emit_target,
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static(emit_target),
            vec![number],
            scalar_plan("null", RuntimeTypeNode::Null),
            NativeRequiredContext::HttpResponseStream,
        )),
        None,
        None,
    );
    let prepared = HttpNativeDispatch::new()
        .prepare(
            NativeCapabilityContexts::<
                (),
                (),
                (),
                TestHttpClient,
                RecordingHttpResponseStream,
                (),
                (),
                (),
            >::HttpResponseStream(RecordingHttpResponseStream {
                calls: Arc::clone(&response_calls),
            }),
            emit_invocation,
            emit_target.to_string(),
            vec![RuntimeValue::Number(1.0)],
            &mut heap,
        )
        .expect("response emit should prepare");
    assert!(matches!(prepared, PreparedNativeCall::ExternalWait(_)));
    heap.alloc_bytes(b"emit caller mutation".to_vec())
        .expect("emit wait does not borrow caller heap");
    drop(prepared);
    assert_eq!(client_calls.load(Ordering::Acquire), 0);
    assert_eq!(response_calls.load(Ordering::Acquire), 0);
}
