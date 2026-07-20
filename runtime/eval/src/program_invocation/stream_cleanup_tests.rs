use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use serde_json::{json, Value};
use skiff_runtime_capability_context::{
    CancellationToken, StreamCancelSignal, StreamInternalItem, StreamLifetimeGuard, StreamPoll,
    StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeResult, StreamSink,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::Interpreter;
use crate::capabilities::StreamConsumerCleanup;

#[derive(Debug)]
struct CancelTrackingRuntime(Arc<AtomicUsize>);

impl StreamRuntimeApi for CancelTrackingRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!()
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!()
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        unreachable!()
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!()
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        unreachable!()
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        unreachable!()
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        unreachable!()
    }

    fn cancel(&self, _value: &Value) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn program_invocation_external_response_rejects_internal_item_and_cancels_stream() {
    let cancellations = Arc::new(AtomicUsize::new(0));
    let runtime = StreamRuntime::new(CancelTrackingRuntime(Arc::clone(&cancellations)));
    let stream_value = json!({"$stream": "external-response"});
    let cleanup = StreamConsumerCleanup::new(runtime, &stream_value);
    let item = StreamPoll::InternalItem(StreamInternalItem::new(
        RuntimeValue::Null,
        RequestHeap::default(),
    ));

    let error = Interpreter::external_wire_stream_item(item, "server-stream response")
        .expect_err("external response must reject an in-process-only item");
    assert!(error
        .to_string()
        .contains("cannot cross a server-stream response boundary"));
    drop(cleanup);
    assert_eq!(cancellations.load(Ordering::SeqCst), 1);
}
