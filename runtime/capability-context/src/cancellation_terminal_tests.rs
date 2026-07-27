use std::{
    future::{pending, Future},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use serde_json::{json, Value};

use crate::{
    CancellationSignals, CancellationSource, CancellationToken, StreamCancelSignal,
    StreamCancelSignalApi, StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi,
    StreamRuntimeError, StreamRuntimeResult, StreamSink, StreamSinkApi,
};

#[derive(Debug)]
struct TokenStreamCancelSignal(CancellationToken);

impl StreamCancelSignalApi for TokenStreamCancelSignal {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.wait_cancelled())
    }
}

async fn wait_for_stream_cancellation(
    signals: &[StreamCancelSignal],
    cancel_tokens: Vec<CancellationToken>,
) {
    let token_signals = CancellationSignals::from_tokens(cancel_tokens);
    if let Some(signal) = signals.first() {
        tokio::select! {
            biased;
            _ = signal.wait_cancelled() => {}
            _ = token_signals.wait_cancelled() => {}
        }
    } else {
        token_signals.wait_cancelled().await;
    }
}

#[derive(Debug, Default)]
struct BlockingStreamSink {
    local_cancel: CancellationSource,
}

impl StreamSinkApi for BlockingStreamSink {
    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(pending())
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        let cancel_tokens = cancel_flags
            .iter()
            .cloned()
            .map(CancellationToken::from_flag)
            .collect();
        Box::pin(async move {
            wait_for_stream_cancellation(&[], cancel_tokens).await;
            Err(StreamRuntimeError::cancelled())
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            wait_for_stream_cancellation(signals, cancel_tokens).await;
            Err(StreamRuntimeError::cancelled())
        })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn fail<'a>(
        &'a self,
        _error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn is_cancelled(&self) -> bool {
        self.local_cancel.is_cancelled()
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other.downcast_ref::<Self>().is_some()
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.local_cancel.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(TokenStreamCancelSignal(self.local_cancel.token()))
    }
}

#[derive(Debug)]
struct BlockingStreamRuntime;

impl StreamRuntimeApi for BlockingStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        panic!("channel_stream is outside this cancellation probe")
    }

    fn channel_stream_with_lifetime(
        &self,
        _lifetime: crate::StreamLifetimeGuard,
    ) -> (Value, StreamSink) {
        panic!("channel_stream_with_lifetime is outside this cancellation probe")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        panic!("pull_stream_with_cancellation is outside this cancellation probe")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        panic!("buffered_stream is outside this cancellation probe")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        signals: &'a [StreamCancelSignal],
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        let cancel_tokens = cancel_flags
            .iter()
            .cloned()
            .map(CancellationToken::from_flag)
            .collect();
        Box::pin(async move {
            wait_for_stream_cancellation(signals, cancel_tokens).await;
            Err(StreamRuntimeError::cancelled())
        })
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(async move {
            wait_for_stream_cancellation(signals, cancel_tokens).await;
            Err(StreamRuntimeError::cancelled())
        })
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        Box::pin(pending())
    }

    fn cancel(&self, _value: &Value) {}
}

#[tokio::test]
async fn blocked_stream_send_wakes_on_outer_cancellation_as_internal_terminal() {
    let outer = CancellationSource::new();
    let inner = CancellationSource::new();
    let sink = StreamSink::new(BlockingStreamSink::default());
    let inner_signal = StreamCancelSignal::new(TokenStreamCancelSignal(inner.token()));
    let task = tokio::spawn({
        let outer = outer.token();
        async move {
            sink.send_with_cancellation(json!({"item": 1}), &[inner_signal], [outer])
                .await
        }
    });

    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    outer.cancel();

    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("outer cancellation should wake blocked stream send")
        .expect("blocked send task should join")
        .expect_err("cancellation should terminate the blocked send");
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert!(!inner.is_cancelled());
}

#[tokio::test]
async fn blocked_stream_next_wakes_on_inner_cancellation_as_internal_terminal() {
    let outer = CancellationSource::new();
    let inner = CancellationSource::new();
    let runtime = StreamRuntime::new(BlockingStreamRuntime);
    let inner_signal = StreamCancelSignal::new(TokenStreamCancelSignal(inner.token()));
    let task = tokio::spawn({
        let outer = outer.token();
        async move {
            runtime
                .next_with_cancellation(
                    &json!({"$stream": "blocked-next"}),
                    &[inner_signal],
                    [outer],
                )
                .await
        }
    });

    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    inner.cancel();

    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("inner cancellation should wake blocked stream next")
        .expect("blocked next task should join")
        .expect_err("cancellation should terminate the blocked next");
    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert!(!outer.is_cancelled());
}

#[test]
fn cancellation_sources_are_single_terminal() {
    let source = CancellationSource::new();
    source.cancel();
    source.cancel();
    assert!(source.is_cancelled());
    assert!(source.token().is_cancelled());
    assert!(source.cancel_flag().load(Ordering::Acquire));
}
