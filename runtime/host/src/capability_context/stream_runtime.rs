use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use serde_json::Value;
use skiff_runtime_boundary::stream::{stream_id, stream_value};
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, StreamInternalItem, StreamLifetimeGuard, StreamPoll,
    StreamPullSource, StreamRuntimeError, StreamRuntimeResult,
};
use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};

mod state;

use state::{
    ChannelTerminal, ChannelTerminalState, StreamEvent, StreamRegistry, StreamSource, StreamState,
    StreamTerminalReason,
};

const STREAM_BUFFER_CAPACITY: usize = 1;
static STREAM_RUNTIME_STREAMS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Default)]
pub struct StreamRuntime {
    next_id: Arc<AtomicU64>,
    registry: Arc<Mutex<StreamRegistry>>,
}

#[derive(Clone, Debug)]
pub struct StreamSink {
    sender: mpsc::Sender<StreamEvent>,
    terminal: Arc<ChannelTerminalState>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

#[derive(Clone, Debug)]
pub struct StreamCancelSignal {
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

impl StreamRuntime {
    pub fn channel_stream(&self) -> (Value, StreamSink) {
        self.channel_stream_inner(None, None)
    }

    pub fn channel_stream_with_lifetime(
        &self,
        lifetime: StreamLifetimeGuard,
    ) -> (Value, StreamSink) {
        self.channel_stream_inner(None, Some(lifetime))
    }

    pub fn channel_stream_in_scope(&self, scope: u64) -> (Value, StreamSink) {
        self.channel_stream_inner(Some(scope), None)
    }

    pub fn channel_stream_with_lifetime_in_scope(
        &self,
        scope: u64,
        lifetime: StreamLifetimeGuard,
    ) -> (Value, StreamSink) {
        self.channel_stream_inner(Some(scope), Some(lifetime))
    }

    fn channel_stream_inner(
        &self,
        scope: Option<u64>,
        lifetime: Option<StreamLifetimeGuard>,
    ) -> (Value, StreamSink) {
        let id = format!("stream-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = mpsc::channel(STREAM_BUFFER_CAPACITY);
        let terminal = Arc::new(ChannelTerminalState::default());
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_notify = Arc::new(Notify::new());
        let state = StreamState {
            scope,
            source: StreamSource::Channel {
                receiver: AsyncMutex::new(receiver),
                terminal: terminal.clone(),
            },
            cancelled: cancelled.clone(),
            cancel_notify: cancel_notify.clone(),
            cancellation: None,
            lifetime: Mutex::new(lifetime),
            ended: AtomicBool::new(false),
        };
        let state = Arc::new(state);
        let registered = {
            let mut registry = self
                .registry
                .lock()
                .expect("stream registry mutex poisoned");
            let registered = registry.register(id.clone(), state.clone());
            if registered {
                STREAM_RUNTIME_STREAMS_ACTIVE.fetch_add(1, Ordering::AcqRel);
            }
            registered
        };
        if !registered {
            state.finish(StreamTerminalReason::SourceDropped);
        }
        (
            stream_value(&id),
            StreamSink {
                sender,
                terminal,
                cancelled,
                cancel_notify,
            },
        )
    }

    pub fn pull_stream(
        &self,
        source: impl StreamPullSource + 'static,
        cancelled: Arc<AtomicBool>,
    ) -> Value {
        self.pull_stream_with_cancellation(source, CancellationToken::from_flag(cancelled))
    }

    pub fn pull_stream_with_cancellation(
        &self,
        source: impl StreamPullSource + 'static,
        cancellation: CancellationToken,
    ) -> Value {
        self.pull_stream_with_cancellation_inner(source, cancellation, None)
    }

    pub fn pull_stream_with_cancellation_in_scope(
        &self,
        source: impl StreamPullSource + 'static,
        cancellation: CancellationToken,
        scope: u64,
    ) -> Value {
        self.pull_stream_with_cancellation_inner(source, cancellation, Some(scope))
    }

    fn pull_stream_with_cancellation_inner(
        &self,
        source: impl StreamPullSource + 'static,
        cancellation: CancellationToken,
        scope: Option<u64>,
    ) -> Value {
        let id = format!("stream-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let cancelled = Arc::new(AtomicBool::new(false));
        let state = StreamState {
            scope,
            source: StreamSource::Pull(AsyncMutex::new(Box::new(source))),
            cancelled,
            cancel_notify: Arc::new(Notify::new()),
            cancellation: Some(cancellation),
            lifetime: Mutex::new(None),
            ended: AtomicBool::new(false),
        };
        let state = Arc::new(state);
        let registered = {
            let mut registry = self
                .registry
                .lock()
                .expect("stream registry mutex poisoned");
            let registered = registry.register(id.clone(), state.clone());
            if registered {
                STREAM_RUNTIME_STREAMS_ACTIVE.fetch_add(1, Ordering::AcqRel);
            }
            registered
        };
        if !registered {
            state.finish(StreamTerminalReason::SourceDropped);
        }
        stream_value(&id)
    }

    fn finish_stream(&self, id: &str, terminal: StreamTerminalReason) {
        let state = self
            .registry
            .lock()
            .expect("stream registry mutex poisoned")
            .remove(id);
        if let Some(state) = state {
            finish_stream_state(state.as_ref(), terminal);
        }
    }

    fn finish_all_streams(&self, terminal: StreamTerminalReason) {
        let states = {
            let mut registry = self
                .registry
                .lock()
                .expect("stream registry mutex poisoned");
            registry.drain_all()
        };
        for state in states {
            finish_stream_state(state.as_ref(), terminal);
        }
    }

    pub fn active_stream_count(&self) -> usize {
        self.registry
            .lock()
            .expect("stream registry mutex poisoned")
            .active_count()
    }

    pub fn active_stream_count_in_scope(&self, scope: u64) -> usize {
        self.registry
            .lock()
            .expect("stream registry mutex poisoned")
            .active_count_in_scope(scope)
    }

    pub fn open_scope(&self, scope: u64) {
        let mut registry = self
            .registry
            .lock()
            .expect("stream registry mutex poisoned");
        registry.open_scope(scope);
    }

    pub fn close_scope(&self, scope: u64) {
        let states = {
            let mut registry = self
                .registry
                .lock()
                .expect("stream registry mutex poisoned");
            registry.close_scope(scope)
        };
        for state in states {
            finish_stream_state(state.as_ref(), StreamTerminalReason::SourceDropped);
        }
    }

    pub fn close_owner(&self) {
        {
            let mut registry = self
                .registry
                .lock()
                .expect("stream registry mutex poisoned");
            registry.close_owner();
        }
        self.finish_all_streams(StreamTerminalReason::SourceDropped);
    }

    #[allow(dead_code)]
    pub fn buffered_stream(&self, items: impl IntoIterator<Item = Value>) -> Value {
        self.buffered_stream_inner(items, None)
    }

    pub fn buffered_stream_in_scope(
        &self,
        items: impl IntoIterator<Item = Value>,
        scope: u64,
    ) -> Value {
        self.buffered_stream_inner(items, Some(scope))
    }

    fn buffered_stream_inner(
        &self,
        items: impl IntoIterator<Item = Value>,
        scope: Option<u64>,
    ) -> Value {
        let (value, sink) = match scope {
            Some(scope) => self.channel_stream_in_scope(scope),
            None => self.channel_stream(),
        };
        let items = items.into_iter().collect::<Vec<_>>();
        tokio::spawn(async move {
            for item in items {
                if sink.send(item).await.is_err() {
                    return;
                }
            }
            sink.end().await;
        });
        value
    }

    #[allow(dead_code)]
    pub async fn next(&self, value: &Value) -> StreamRuntimeResult<StreamPoll> {
        let cancellation = CancellationSignals::none();
        self.next_with_cancellation(value, &[], &cancellation).await
    }

    pub async fn next_with_cancel(
        &self,
        value: &Value,
        signals: &[StreamCancelSignal],
        cancel_flags: &[Arc<AtomicBool>],
    ) -> StreamRuntimeResult<StreamPoll> {
        let cancellation = CancellationSignals::from_flags(cancel_flags.iter().cloned());
        self.next_with_cancellation(value, signals, &cancellation)
            .await
    }

    pub async fn next_with_cancellation(
        &self,
        value: &Value,
        signals: &[StreamCancelSignal],
        cancellation: &CancellationSignals<'_>,
    ) -> StreamRuntimeResult<StreamPoll> {
        let id = stream_id(value)
            .ok_or_else(|| StreamRuntimeError::decode("for stream source is not a Stream value"))?;
        let state = self
            .registry
            .lock()
            .expect("stream registry mutex poisoned")
            .get(id)
            .ok_or_else(|| StreamRuntimeError::decode("unknown Stream value"))?;
        if state.ended.load(Ordering::SeqCst) {
            self.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::decode(
                "Stream value has already been consumed",
            ));
        }
        if state.cancelled.load(Ordering::SeqCst) {
            self.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
        if external_cancelled(signals, cancellation) {
            self.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }

        match &state.source {
            StreamSource::Channel { receiver, terminal } => {
                let event =
                    next_channel_event(self, id, &state, receiver, terminal, signals, cancellation)
                        .await?;
                match event {
                    Some(StreamEvent::Item(value)) => Ok(StreamPoll::Item(value)),
                    Some(StreamEvent::InternalItem(item)) => Ok(StreamPoll::InternalItem(item)),
                    Some(StreamEvent::End) => {
                        self.finish_stream(id, StreamTerminalReason::End);
                        Ok(StreamPoll::End)
                    }
                    None => {
                        self.finish_stream(id, StreamTerminalReason::SourceDropped);
                        Ok(StreamPoll::End)
                    }
                    Some(StreamEvent::Error(error)) => {
                        self.finish_stream(id, StreamTerminalReason::Error);
                        Err(error)
                    }
                }
            }
            StreamSource::Pull(source) => {
                match next_pull_event(&state, source, signals, cancellation).await {
                    Ok(Some(value)) => Ok(StreamPoll::Item(value)),
                    Ok(None) => {
                        self.finish_stream(id, StreamTerminalReason::End);
                        Ok(StreamPoll::End)
                    }
                    Err(error) => {
                        let terminal = match error {
                            StreamRuntimeError::Cancelled => StreamTerminalReason::Cancelled,
                            StreamRuntimeError::Decode(_) | StreamRuntimeError::Producer(_) => {
                                StreamTerminalReason::Error
                            }
                        };
                        self.finish_stream(id, terminal);
                        Err(error)
                    }
                }
            }
        }
    }

    pub fn cancel(&self, value: &Value) {
        let Some(id) = stream_id(value) else {
            return;
        };
        self.finish_stream(id, StreamTerminalReason::Cancelled);
    }
}

pub(crate) fn stream_runtime_streams_active() -> usize {
    STREAM_RUNTIME_STREAMS_ACTIVE.load(Ordering::Acquire)
}

fn finish_stream_state(state: &StreamState, terminal: StreamTerminalReason) {
    if state.finish(terminal) {
        STREAM_RUNTIME_STREAMS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for StreamRuntime {
    fn drop(&mut self) {
        if Arc::strong_count(&self.registry) == 1 {
            self.finish_all_streams(StreamTerminalReason::SourceDropped);
        }
    }
}

async fn next_channel_event(
    runtime: &StreamRuntime,
    id: &str,
    state: &StreamState,
    receiver: &AsyncMutex<mpsc::Receiver<StreamEvent>>,
    terminal: &ChannelTerminalState,
    signals: &[StreamCancelSignal],
    cancellation: &CancellationSignals<'_>,
) -> StreamRuntimeResult<Option<StreamEvent>> {
    let lock_cancel_notified = wait_for_stream_cancel(state);
    tokio::pin!(lock_cancel_notified);
    let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
    tokio::pin!(external_cancel_notified);
    if state.cancelled.load(Ordering::SeqCst) {
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }
    let mut receiver = tokio::select! {
        receiver = receiver.lock() => receiver,
        _ = &mut lock_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
        _ = &mut external_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
    };
    loop {
        let terminal_notified = terminal.notify.notified();
        tokio::pin!(terminal_notified);
        terminal_notified.as_mut().enable();
        if state.cancelled.load(Ordering::SeqCst) {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
        if external_cancelled(signals, cancellation) {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
        match receiver.try_recv() {
            Ok(event) => return Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                return Ok(terminal.take_event());
            }
        }
        if let Some(event) = terminal.take_event() {
            return Ok(Some(event));
        }
        let cancel_notified = wait_for_stream_cancel(state);
        tokio::pin!(cancel_notified);
        let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
        tokio::pin!(external_cancel_notified);
        tokio::select! {
            biased;
            event = receiver.recv() => {
                if let Some(event) = event {
                    return Ok(Some(event));
                }
            }
            _ = &mut terminal_notified => {}
            _ = &mut cancel_notified => {
                runtime.finish_stream(id, StreamTerminalReason::Cancelled);
                return Err(StreamRuntimeError::cancelled());
            }
            _ = &mut external_cancel_notified => {
                runtime.finish_stream(id, StreamTerminalReason::Cancelled);
                return Err(StreamRuntimeError::cancelled());
            }
        }
    }
}

async fn next_pull_event(
    state: &StreamState,
    source: &AsyncMutex<Box<dyn StreamPullSource>>,
    signals: &[StreamCancelSignal],
    cancellation: &CancellationSignals<'_>,
) -> StreamRuntimeResult<Option<Value>> {
    let lock_cancel_notified = wait_for_stream_cancel(state);
    tokio::pin!(lock_cancel_notified);
    let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
    tokio::pin!(external_cancel_notified);
    if state.cancelled.load(Ordering::SeqCst) {
        return Err(StreamRuntimeError::cancelled());
    }
    let mut source = tokio::select! {
        source = source.lock() => source,
        _ = &mut lock_cancel_notified => {
            return Err(StreamRuntimeError::cancelled());
        }
        _ = &mut external_cancel_notified => {
            return Err(StreamRuntimeError::cancelled());
        }
    };
    let cancel_notified = wait_for_stream_cancel(state);
    tokio::pin!(cancel_notified);
    let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
    tokio::pin!(external_cancel_notified);
    if state.cancelled.load(Ordering::SeqCst) {
        return Err(StreamRuntimeError::cancelled());
    }
    if external_cancelled(signals, cancellation) {
        return Err(StreamRuntimeError::cancelled());
    }

    tokio::select! {
        event = source.next() => event,
        _ = &mut cancel_notified => {
            Err(StreamRuntimeError::cancelled())
        }
        _ = &mut external_cancel_notified => {
            Err(StreamRuntimeError::cancelled())
        }
    }
}

impl StreamCancelSignal {
    pub async fn wait_cancelled(&self) {
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        let notified = self.cancel_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.cancelled.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }
}

impl StreamSink {
    pub async fn send(&self, item: Value) -> StreamRuntimeResult<()> {
        let cancellation = CancellationSignals::none();
        self.send_with_cancellation(item, &cancellation).await
    }

    pub async fn send_with_cancel(
        &self,
        item: Value,
        cancel_flags: &[Arc<AtomicBool>],
    ) -> StreamRuntimeResult<()> {
        let cancellation = CancellationSignals::from_flags(cancel_flags.iter().cloned());
        self.send_with_cancellation(item, &cancellation).await
    }

    pub async fn send_with_cancellation(
        &self,
        item: Value,
        cancellation: &CancellationSignals<'_>,
    ) -> StreamRuntimeResult<()> {
        self.send_with_stream_cancellation(item, &[], cancellation)
            .await
    }

    pub async fn send_with_stream_cancellation(
        &self,
        item: Value,
        signals: &[StreamCancelSignal],
        cancellation: &CancellationSignals<'_>,
    ) -> StreamRuntimeResult<()> {
        self.send_event_with_stream_cancellation(StreamEvent::Item(item), signals, cancellation)
            .await
    }

    pub async fn send_internal_with_stream_cancellation(
        &self,
        item: StreamInternalItem,
        signals: &[StreamCancelSignal],
        cancellation: &CancellationSignals<'_>,
    ) -> StreamRuntimeResult<()> {
        self.send_event_with_stream_cancellation(
            StreamEvent::InternalItem(item),
            signals,
            cancellation,
        )
        .await
    }

    async fn send_event_with_stream_cancellation(
        &self,
        event: StreamEvent,
        signals: &[StreamCancelSignal],
        cancellation: &CancellationSignals<'_>,
    ) -> StreamRuntimeResult<()> {
        if self.is_cancelled() || external_cancelled(signals, cancellation) {
            return Err(StreamRuntimeError::cancelled());
        }
        let cancel_notified = self.cancel_notify.notified();
        tokio::pin!(cancel_notified);
        cancel_notified.as_mut().enable();
        let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
        tokio::pin!(external_cancel_notified);
        if self.is_cancelled() || external_cancelled(signals, cancellation) {
            return Err(StreamRuntimeError::cancelled());
        }
        tokio::select! {
            permit = self.sender.reserve() => {
                let permit = permit.map_err(|_| StreamRuntimeError::cancelled())?;
                if !self.terminal.send_if_open(permit, event) {
                    return Err(StreamRuntimeError::cancelled());
                }
                Ok(())
            }
            _ = &mut cancel_notified => Err(StreamRuntimeError::cancelled()),
            _ = &mut external_cancel_notified => Err(StreamRuntimeError::cancelled()),
        }
    }

    pub async fn end(&self) {
        if !self.is_cancelled() {
            self.terminal.publish(ChannelTerminal::End);
        }
    }

    pub async fn fail(&self, error: StreamRuntimeError) {
        if !self.is_cancelled() {
            self.terminal.publish(ChannelTerminal::Error(error));
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn is_same_stream(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
            && Arc::ptr_eq(&self.cancel_notify, &other.cancel_notify)
            && self.sender.same_channel(&other.sender)
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    pub fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal {
            cancelled: self.cancelled.clone(),
            cancel_notify: self.cancel_notify.clone(),
        }
    }
}

fn external_cancelled(
    signals: &[StreamCancelSignal],
    cancellation: &CancellationSignals<'_>,
) -> bool {
    signals
        .iter()
        .any(|signal| signal.cancelled.load(Ordering::SeqCst))
        || cancellation.is_cancelled()
}

async fn wait_for_external_cancel(
    signals: &[StreamCancelSignal],
    cancellation: &CancellationSignals<'_>,
) {
    if signals.is_empty() && cancellation.is_empty() {
        std::future::pending::<()>().await;
        return;
    }
    while !external_cancelled(signals, cancellation) {
        tokio::select! {
            _ = wait_for_any_signal(signals), if !signals.is_empty() => {},
            _ = cancellation.wait_cancelled(), if !cancellation.is_empty() => {},
        }
    }
}

async fn wait_for_stream_cancel(state: &StreamState) {
    if state.cancelled.load(Ordering::SeqCst) {
        return;
    }
    let cancel_notified = state.cancel_notify.notified();
    tokio::pin!(cancel_notified);
    cancel_notified.as_mut().enable();
    if state.cancelled.load(Ordering::SeqCst) {
        return;
    }
    if let Some(cancellation) = &state.cancellation {
        tokio::select! {
            _ = &mut cancel_notified => {},
            _ = cancellation.wait_cancelled() => {},
        }
    } else {
        cancel_notified.await;
    }
}

async fn wait_for_any_signal(signals: &[StreamCancelSignal]) {
    if signals.is_empty() {
        std::future::pending::<()>().await;
        return;
    }
    loop {
        if signals
            .iter()
            .any(|signal| signal.cancelled.load(Ordering::SeqCst))
        {
            return;
        }
        let mut futures = signals
            .iter()
            .map(|signal| Box::pin(signal.cancel_notify.notified()))
            .collect::<Vec<_>>();
        for future in &mut futures {
            future.as_mut().enable();
        }
        if signals
            .iter()
            .any(|signal| signal.cancelled.load(Ordering::SeqCst))
        {
            return;
        }
        std::future::poll_fn(|context| {
            if signals
                .iter()
                .any(|signal| signal.cancelled.load(Ordering::SeqCst))
            {
                return std::task::Poll::Ready(());
            }
            for future in &mut futures {
                if future.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(());
                }
            }
            std::task::Poll::Pending
        })
        .await;
    }
}

#[cfg(test)]
mod tests;
