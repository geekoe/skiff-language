use super::imports::*;

#[derive(Debug, Default)]
pub(super) struct ProbeStreamState {
    next_calls: AtomicUsize,
    pub(super) last_cancel_token_count: AtomicUsize,
    pub(super) cleanup_cancels: AtomicUsize,
    cancelled: AtomicBool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ProbeStreamRuntime {
    pub(super) state: Arc<ProbeStreamState>,
}

impl ProbeStreamRuntime {
    fn wait_with_tokens<'a>(
        &'a self,
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.state.next_calls.fetch_add(1, Ordering::AcqRel);
        self.state
            .last_cancel_token_count
            .store(cancel_tokens.len(), Ordering::Release);
        Box::pin(async move {
            loop {
                if self.state.cancelled.load(Ordering::Acquire)
                    || cancel_tokens.iter().any(CancellationToken::is_cancelled)
                {
                    return Err(StreamRuntimeError::cancelled());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
    }
}

impl StreamRuntimeApi for ProbeStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        (
            stream_value("f445h-e4r-combined-channel"),
            StreamSink::new(NoopStreamSink::default()),
        )
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        self.channel_stream()
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        stream_value("f445h-e4r-combined-pull")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        stream_value("f445h-e4r-combined-buffered")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(
            cancel_flags
                .iter()
                .cloned()
                .map(CancellationToken::from_flag)
                .collect(),
        )
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(cancel_tokens)
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(Vec::new())
    }

    fn cancel(&self, _value: &Value) {
        self.state.cancelled.store(true, Ordering::Release);
        self.state.cleanup_cancels.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Debug)]
struct NoopStreamSignal {
    token: CancellationToken,
}

impl StreamCancelSignalApi for NoopStreamSignal {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.token.wait_cancelled())
    }
}

#[derive(Clone, Debug)]
struct NoopStreamSink {
    cancellation: CancellationToken,
}

impl Default for NoopStreamSink {
    fn default() -> Self {
        Self {
            cancellation: CancellationToken::new(),
        }
    }
}

impl StreamSinkApi for NoopStreamSink {
    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if cancel_flags.iter().any(|flag| flag.load(Ordering::Acquire)) {
                Err(StreamRuntimeError::cancelled())
            } else {
                Ok(())
            }
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if cancel_tokens.iter().any(CancellationToken::is_cancelled) {
                Err(StreamRuntimeError::cancelled())
            } else {
                Ok(())
            }
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
        self.cancellation.is_cancelled()
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other.downcast_ref::<Self>().is_some()
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(NoopStreamSignal {
            token: self.cancellation.clone(),
        })
    }
}
