use std::{
    collections::HashMap,
    fmt,
    future::Future,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use serde_json::Value;
use skiff_runtime_boundary::stream::{stream_id, stream_value};
use skiff_runtime_capability_context::{
    CancellationSignals, CancellationToken, StreamPoll, StreamPullSource, StreamRuntimeError,
    StreamRuntimeResult,
};
use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};

const STREAM_BUFFER_CAPACITY: usize = 1;

#[derive(Clone, Debug, Default)]
pub struct StreamRuntime {
    next_id: Arc<AtomicU64>,
    streams: Arc<Mutex<HashMap<String, Arc<StreamState>>>>,
}

#[derive(Clone, Debug)]
pub struct StreamSink {
    sender: mpsc::Sender<StreamEvent>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

#[derive(Clone, Debug)]
pub struct StreamCancelSignal {
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

#[derive(Debug)]
enum StreamEvent {
    Item(Value),
    End,
    Error(StreamRuntimeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamTerminalReason {
    End,
    Error,
    Cancelled,
    SourceDropped,
}

struct StreamState {
    source: StreamSource,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
    cancellation: Option<CancellationToken>,
    ended: AtomicBool,
}

enum StreamSource {
    Channel(AsyncMutex<mpsc::Receiver<StreamEvent>>),
    Pull(AsyncMutex<Box<dyn StreamPullSource>>),
}

impl fmt::Debug for StreamState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamState")
            .field("source", &self.source)
            .field("cancelled", &self.cancelled.load(Ordering::SeqCst))
            .field("ended", &self.ended.load(Ordering::SeqCst))
            .finish()
    }
}

impl fmt::Debug for StreamSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel(_) => formatter.write_str("Channel"),
            Self::Pull(_) => formatter.write_str("Pull"),
        }
    }
}

impl StreamRuntime {
    pub fn channel_stream(&self) -> (Value, StreamSink) {
        let id = format!("stream-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = mpsc::channel(STREAM_BUFFER_CAPACITY);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_notify = Arc::new(Notify::new());
        let state = StreamState {
            source: StreamSource::Channel(AsyncMutex::new(receiver)),
            cancelled: cancelled.clone(),
            cancel_notify: cancel_notify.clone(),
            cancellation: None,
            ended: AtomicBool::new(false),
        };
        self.streams
            .lock()
            .expect("stream registry mutex poisoned")
            .insert(id.clone(), Arc::new(state));
        (
            stream_value(&id),
            StreamSink {
                sender,
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
        let id = format!("stream-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let state = StreamState {
            source: StreamSource::Pull(AsyncMutex::new(Box::new(source))),
            cancelled: cancellation.cancel_flag(),
            cancel_notify: Arc::new(Notify::new()),
            cancellation: Some(cancellation),
            ended: AtomicBool::new(false),
        };
        self.streams
            .lock()
            .expect("stream registry mutex poisoned")
            .insert(id.clone(), Arc::new(state));
        stream_value(&id)
    }

    fn finish_stream(&self, id: &str, terminal: StreamTerminalReason) {
        let state = self
            .streams
            .lock()
            .expect("stream registry mutex poisoned")
            .remove(id);
        if let Some(state) = state {
            state.finish(terminal);
        }
    }

    fn finish_all_streams(&self, terminal: StreamTerminalReason) {
        let states = self
            .streams
            .lock()
            .expect("stream registry mutex poisoned")
            .drain()
            .map(|(_, state)| state)
            .collect::<Vec<_>>();
        for state in states {
            state.finish(terminal);
        }
    }

    pub fn active_stream_count(&self) -> usize {
        self.streams
            .lock()
            .expect("stream registry mutex poisoned")
            .len()
    }

    #[allow(dead_code)]
    pub fn buffered_stream(&self, items: impl IntoIterator<Item = Value>) -> Value {
        let (value, sink) = self.channel_stream();
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
            .streams
            .lock()
            .expect("stream registry mutex poisoned")
            .get(id)
            .cloned()
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
            StreamSource::Channel(receiver) => {
                let event =
                    next_channel_event(self, id, &state, receiver, signals, cancellation).await?;
                match event {
                    Some(StreamEvent::Item(value)) => Ok(StreamPoll::Item(value)),
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
                let event =
                    next_pull_event(self, id, &state, source, signals, cancellation).await?;
                match event {
                    Some(value) => Ok(StreamPoll::Item(value)),
                    None => {
                        self.finish_stream(id, StreamTerminalReason::End);
                        Ok(StreamPoll::End)
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

impl Drop for StreamRuntime {
    fn drop(&mut self) {
        if Arc::strong_count(&self.streams) == 1 {
            self.finish_all_streams(StreamTerminalReason::SourceDropped);
        }
    }
}

impl StreamState {
    fn finish(&self, _terminal: StreamTerminalReason) -> bool {
        if self
            .ended
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.cancelled.store(true, Ordering::SeqCst);
            self.cancel_notify.notify_waiters();
            true
        } else {
            false
        }
    }
}

async fn next_channel_event(
    runtime: &StreamRuntime,
    id: &str,
    state: &StreamState,
    receiver: &AsyncMutex<mpsc::Receiver<StreamEvent>>,
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
    let cancel_notified = wait_for_stream_cancel(state);
    tokio::pin!(cancel_notified);
    let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
    tokio::pin!(external_cancel_notified);
    if state.cancelled.load(Ordering::SeqCst) {
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }
    if external_cancelled(signals, cancellation) {
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }

    tokio::select! {
        event = receiver.recv() => Ok(event),
        _ = &mut cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            Err(StreamRuntimeError::cancelled())
        }
        _ = &mut external_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            Err(StreamRuntimeError::cancelled())
        }
    }
}

async fn next_pull_event(
    runtime: &StreamRuntime,
    id: &str,
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
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }
    let mut source = tokio::select! {
        source = source.lock() => source,
        _ = &mut lock_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
        _ = &mut external_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            return Err(StreamRuntimeError::cancelled());
        }
    };
    let cancel_notified = wait_for_stream_cancel(state);
    tokio::pin!(cancel_notified);
    let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
    tokio::pin!(external_cancel_notified);
    if state.cancelled.load(Ordering::SeqCst) {
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }
    if external_cancelled(signals, cancellation) {
        runtime.finish_stream(id, StreamTerminalReason::Cancelled);
        return Err(StreamRuntimeError::cancelled());
    }

    tokio::select! {
        event = source.next() => event,
        _ = &mut cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            Err(StreamRuntimeError::cancelled())
        }
        _ = &mut external_cancel_notified => {
            runtime.finish_stream(id, StreamTerminalReason::Cancelled);
            Err(StreamRuntimeError::cancelled())
        }
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
        if self.is_cancelled() {
            return Err(StreamRuntimeError::cancelled());
        }
        if external_cancelled(signals, cancellation) {
            return Err(StreamRuntimeError::cancelled());
        }
        if cancellation.is_cancelled() {
            return Err(StreamRuntimeError::cancelled());
        }
        let cancel_notified = self.cancel_notify.notified();
        tokio::pin!(cancel_notified);
        cancel_notified.as_mut().enable();
        let external_cancel_notified = wait_for_external_cancel(signals, cancellation);
        tokio::pin!(external_cancel_notified);
        if self.is_cancelled() {
            return Err(StreamRuntimeError::cancelled());
        }
        if external_cancelled(signals, cancellation) {
            return Err(StreamRuntimeError::cancelled());
        }
        tokio::select! {
            permit = self.sender.reserve() => {
                permit
                    .map_err(|_| StreamRuntimeError::cancelled())?
                    .send(StreamEvent::Item(item));
                Ok(())
            }
            _ = &mut cancel_notified => Err(StreamRuntimeError::cancelled()),
            _ = &mut external_cancel_notified => Err(StreamRuntimeError::cancelled()),
        }
    }

    pub async fn end(&self) {
        if !self.is_cancelled() {
            let _ = self.send_event(StreamEvent::End).await;
        }
    }

    pub async fn fail(&self, error: StreamRuntimeError) {
        if !self.is_cancelled() {
            let _ = self.send_event(StreamEvent::Error(error)).await;
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

    async fn send_event(&self, event: StreamEvent) -> StreamRuntimeResult<()> {
        if self.is_cancelled() {
            return Err(StreamRuntimeError::cancelled());
        }
        let cancel_notified = self.cancel_notify.notified();
        tokio::pin!(cancel_notified);
        cancel_notified.as_mut().enable();
        if self.is_cancelled() {
            return Err(StreamRuntimeError::cancelled());
        }
        tokio::select! {
            result = self.sender.send(event) => result.map_err(|_| StreamRuntimeError::cancelled()),
            _ = &mut cancel_notified => Err(StreamRuntimeError::cancelled()),
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
    loop {
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
                _ = &mut cancel_notified => return,
                _ = cancellation.wait_cancelled() => return,
            }
        } else {
            cancel_notified.await;
            return;
        }
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
