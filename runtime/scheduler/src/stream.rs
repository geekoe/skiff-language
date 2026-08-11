use std::{
    collections::VecDeque,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use skiff_runtime_model::vm_heap::VmHeapError;
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};

/// Runtime-neutral readiness notification.
///
/// Implementations enqueue work only; they must not recursively poll a VM,
/// adapter or stream producer from this call.
pub trait WakeSignal: Send + Sync + 'static {
    fn wake(&self);
}

/// Typed consumer result. `Pending` is returned only when no buffered item,
/// blocked producer item or terminal event is currently available.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamPoll<T, E> {
    Ready(StreamEvent<T, E>),
    Pending,
    Rejected(StreamError),
}

#[derive(Debug, PartialEq, Eq)]
pub enum StreamEvent<T, E> {
    Item(T),
    End,
    Error(E),
    Cancelled,
}

/// Typed producer result. `Pending` means the item is now supervisor-owned and
/// the producer must park until its wake signal is enqueued.
#[derive(Debug, PartialEq, Eq)]
pub enum StreamEmit<T> {
    Ready,
    Pending,
    Rejected { item: T, reason: StreamError },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamError {
    ConsumerPollAlreadyPending,
    ProducerEmitAlreadyPending,
    ProducerHasPendingEmit,
    AlreadyTerminal,
}

impl fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConsumerPollAlreadyPending => {
                formatter.write_str("stream consumer already has a pending poll")
            }
            Self::ProducerEmitAlreadyPending => {
                formatter.write_str("stream producer already has a backpressured item")
            }
            Self::ProducerHasPendingEmit => {
                formatter.write_str("stream producer cannot finish while an emit is pending")
            }
            Self::AlreadyTerminal => formatter.write_str("stream is already terminal"),
        }
    }
}

impl std::error::Error for StreamError {}

enum Terminal<E> {
    Open,
    End,
    Error(E),
    Cancelled,
    Exhausted,
}

struct BlockedEmit<T> {
    item: T,
    wake: Arc<dyn WakeSignal>,
}

struct StreamState<T, E> {
    buffer: VecDeque<T>,
    consumer_waiter: Option<Arc<dyn WakeSignal>>,
    blocked_emit: Option<BlockedEmit<T>>,
    terminal: Terminal<E>,
}

struct SharedStream<P, T, E> {
    // Payload state drops before the exact provider pin on final teardown.
    state: Mutex<StreamState<T, E>>,
    owner_pin: P,
}

/// Phase 5's first bounded producer/consumer buffer.
pub const STREAM_BUFFER_CAPACITY: usize = 1;

/// Owner-pinned supervision state for one affine stream endpoint.
///
/// [`Self::open`] returns the consumer endpoint before any producer body is
/// polled. The scheduler must materialize that endpoint into the caller before
/// installing/running the producer unit.
///
/// Root enumeration keeps the stream state mutex locked so affine buffered and
/// terminal payloads cannot move during the walk. `T`, `E` and the visitor must
/// obey the crate-level safepoint contract: enumeration is non-blocking and
/// must not wake, drop payloads or re-enter this stream.
pub struct StreamSupervisor<P, T, E> {
    shared: Arc<SharedStream<P, T, E>>,
}

impl<P, T, E> StreamSupervisor<P, T, E> {
    pub fn open(owner_pin: P) -> (Self, StreamProducer<P, T, E>, StreamConsumer<P, T, E>) {
        let shared = Arc::new(SharedStream {
            state: Mutex::new(StreamState {
                buffer: VecDeque::with_capacity(STREAM_BUFFER_CAPACITY),
                consumer_waiter: None,
                blocked_emit: None,
                terminal: Terminal::Open,
            }),
            owner_pin,
        });
        (
            Self {
                shared: Arc::clone(&shared),
            },
            StreamProducer {
                shared: Arc::clone(&shared),
                terminal_sent: false,
            },
            StreamConsumer {
                shared,
                terminal_seen: false,
            },
        )
    }

    pub fn owner_pin(&self) -> &P {
        &self.shared.owner_pin
    }

    pub fn capacity(&self) -> usize {
        STREAM_BUFFER_CAPACITY
    }

    pub fn buffered_len(&self) -> usize {
        lock_unpoisoned(&self.shared.state).buffer.len()
    }

    pub fn is_backpressured(&self) -> bool {
        lock_unpoisoned(&self.shared.state).blocked_emit.is_some()
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(
            &lock_unpoisoned(&self.shared.state).terminal,
            Terminal::Cancelled
        )
    }
}

impl<P, T, E> VmRootSource for StreamSupervisor<P, T, E>
where
    T: VmRootSource,
    E: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        let state = lock_unpoisoned(&self.shared.state);
        for item in &state.buffer {
            item.visit_roots(visitor)?;
        }
        if let Some(blocked) = &state.blocked_emit {
            blocked.item.visit_roots(visitor)?;
        }
        if let Terminal::Error(error) = &state.terminal {
            error.visit_roots(visitor)?;
        }
        Ok(())
    }
}

impl<P, T, E> Drop for StreamSupervisor<P, T, E> {
    fn drop(&mut self) {
        cancel_stream(&self.shared);
    }
}

/// The unique producer-side stream capability.
///
/// It is intentionally not `Clone`. Dropping it without an explicit terminal
/// event cancels the stream rather than forging a normal end.
pub struct StreamProducer<P, T, E> {
    shared: Arc<SharedStream<P, T, E>>,
    terminal_sent: bool,
}

impl<P, T, E> StreamProducer<P, T, E> {
    pub fn owner_pin(&self) -> &P {
        &self.shared.owner_pin
    }

    /// Offers one item without waiting.
    ///
    /// A waiting consumer or free bounded-buffer slot is `Ready`. Only a full
    /// buffer with no waiting consumer stores the item and returns actual
    /// `Pending` backpressure.
    pub fn emit(&mut self, item: T, wake: Arc<dyn WakeSignal>) -> StreamEmit<T> {
        let mut state = lock_unpoisoned(&self.shared.state);
        if !matches!(&state.terminal, Terminal::Open) {
            drop(state);
            return StreamEmit::Rejected {
                item,
                reason: StreamError::AlreadyTerminal,
            };
        }
        if state.blocked_emit.is_some() {
            drop(state);
            return StreamEmit::Rejected {
                item,
                reason: StreamError::ProducerEmitAlreadyPending,
            };
        }

        if let Some(consumer) = state.consumer_waiter.take() {
            state.buffer.push_back(item);
            drop(state);
            consumer.wake();
            return StreamEmit::Ready;
        }
        if state.buffer.len() < STREAM_BUFFER_CAPACITY {
            state.buffer.push_back(item);
            return StreamEmit::Ready;
        }

        state.blocked_emit = Some(BlockedEmit { item, wake });
        StreamEmit::Pending
    }

    pub fn finish_end(&mut self) -> Result<(), StreamError> {
        let consumer = {
            let mut state = lock_unpoisoned(&self.shared.state);
            if state.blocked_emit.is_some() {
                drop(state);
                return Err(StreamError::ProducerHasPendingEmit);
            }
            if !matches!(&state.terminal, Terminal::Open) {
                drop(state);
                return Err(StreamError::AlreadyTerminal);
            }
            state.terminal = Terminal::End;
            state.consumer_waiter.take()
        };
        self.terminal_sent = true;
        wake(consumer);
        Ok(())
    }

    pub fn finish_error(&mut self, error: E) -> Result<(), (StreamError, E)> {
        let consumer = {
            let mut state = lock_unpoisoned(&self.shared.state);
            if state.blocked_emit.is_some() {
                drop(state);
                return Err((StreamError::ProducerHasPendingEmit, error));
            }
            if !matches!(&state.terminal, Terminal::Open) {
                drop(state);
                return Err((StreamError::AlreadyTerminal, error));
            }
            state.terminal = Terminal::Error(error);
            state.consumer_waiter.take()
        };
        self.terminal_sent = true;
        wake(consumer);
        Ok(())
    }
}

impl<P, T, E> Drop for StreamProducer<P, T, E> {
    fn drop(&mut self) {
        if !self.terminal_sent {
            cancel_stream(&self.shared);
        }
    }
}

/// The unique affine consumer endpoint.
///
/// It is intentionally not `Clone`. Dropping it cancels the supervisor,
/// discards queued items through their owner types and wakes a blocked
/// producer. A late producer item is returned in `StreamEmit::Rejected`.
pub struct StreamConsumer<P, T, E> {
    shared: Arc<SharedStream<P, T, E>>,
    terminal_seen: bool,
}

impl<P, T, E> StreamConsumer<P, T, E> {
    pub fn owner_pin(&self) -> &P {
        &self.shared.owner_pin
    }

    pub fn poll_next(&mut self, wake_signal: Arc<dyn WakeSignal>) -> StreamPoll<T, E> {
        let (poll, producer_wake) = {
            let mut state = lock_unpoisoned(&self.shared.state);
            if state.consumer_waiter.is_some() {
                drop(state);
                return StreamPoll::Rejected(StreamError::ConsumerPollAlreadyPending);
            }

            if let Some(item) = state.buffer.pop_front() {
                let producer_wake = refill_from_blocked(&mut state);
                (StreamPoll::Ready(StreamEvent::Item(item)), producer_wake)
            } else if let Some(blocked) = state.blocked_emit.take() {
                (
                    StreamPoll::Ready(StreamEvent::Item(blocked.item)),
                    Some(blocked.wake),
                )
            } else {
                match std::mem::replace(&mut state.terminal, Terminal::Exhausted) {
                    Terminal::Open => {
                        state.terminal = Terminal::Open;
                        state.consumer_waiter = Some(wake_signal);
                        (StreamPoll::Pending, None)
                    }
                    Terminal::End | Terminal::Exhausted => {
                        self.terminal_seen = true;
                        (StreamPoll::Ready(StreamEvent::End), None)
                    }
                    Terminal::Error(error) => {
                        self.terminal_seen = true;
                        (StreamPoll::Ready(StreamEvent::Error(error)), None)
                    }
                    Terminal::Cancelled => {
                        self.terminal_seen = true;
                        (StreamPoll::Ready(StreamEvent::Cancelled), None)
                    }
                }
            }
        };
        wake(producer_wake);
        poll
    }

    pub fn cancel(mut self) {
        cancel_stream(&self.shared);
        self.terminal_seen = true;
    }
}

impl<P, T, E> Drop for StreamConsumer<P, T, E> {
    fn drop(&mut self) {
        if !self.terminal_seen {
            cancel_stream(&self.shared);
        }
    }
}

fn refill_from_blocked<T, E>(state: &mut StreamState<T, E>) -> Option<Arc<dyn WakeSignal>> {
    let blocked = state.blocked_emit.take()?;
    state.buffer.push_back(blocked.item);
    Some(blocked.wake)
}

fn cancel_stream<P, T, E>(shared: &SharedStream<P, T, E>) {
    let (consumer, blocked, buffered, previous_terminal) = {
        let mut state = lock_unpoisoned(&shared.state);
        let buffered = std::mem::take(&mut state.buffer);
        let blocked = state.blocked_emit.take();
        let consumer = state.consumer_waiter.take();
        let previous_terminal = std::mem::replace(&mut state.terminal, Terminal::Cancelled);
        (consumer, blocked, buffered, previous_terminal)
    };

    let producer = blocked.as_ref().map(|blocked| Arc::clone(&blocked.wake));
    drop(buffered);
    drop(blocked);
    drop(previous_terminal);
    wake(consumer);
    wake(producer);
}

fn wake(signal: Option<Arc<dyn WakeSignal>>) {
    if let Some(signal) = signal {
        signal.wake();
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl<P, T, E> fmt::Debug for StreamSupervisor<P, T, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamSupervisor")
            .field("capacity", &STREAM_BUFFER_CAPACITY)
            .field("buffered", &self.buffered_len())
            .field("backpressured", &self.is_backpressured())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Weak,
    };

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };

    use super::{
        SharedStream, StreamEmit, StreamError, StreamEvent, StreamPoll, StreamSupervisor,
        WakeSignal,
    };

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl WakeSignal for Counter {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct ReentrantDrop {
        shared: Weak<SharedStream<&'static str, ReentrantDrop, ReentrantDrop>>,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for ReentrantDrop {
        fn drop(&mut self) {
            if let Some(shared) = self.shared.upgrade() {
                assert!(
                    shared.state.try_lock().is_ok(),
                    "stream payload destructor ran while the state mutex was held"
                );
            }
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct NoRoots;

    impl VmRootSource for NoRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingVisitor(usize);

    impl VmRootVisitor for CountingVisitor {
        fn visit_root(&mut self, _root: &ValueSlot) -> Result<(), VmHeapError> {
            self.0 += 1;
            Ok(())
        }
    }

    struct RootWalkProbe {
        root: ValueSlot,
        shared: Weak<SharedStream<&'static str, RootWalkProbe, NoRoots>>,
        visits: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl VmRootSource for RootWalkProbe {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            let shared = self.shared.upgrade().expect("stream remains pinned");
            assert!(
                shared.state.try_lock().is_err(),
                "stream state must stay stable throughout its root walk"
            );
            self.visits.fetch_add(1, Ordering::Relaxed);
            visitor.visit_root(&self.root)
        }
    }

    impl Drop for RootWalkProbe {
        fn drop(&mut self) {
            if let Some(shared) = self.shared.upgrade() {
                assert!(
                    shared.state.try_lock().is_ok(),
                    "stream root payload dropped while the state mutex was held"
                );
            }
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn safepoint_root_walk_neither_wakes_nor_drops_stream_payloads() {
        let (supervisor, mut producer, consumer) =
            StreamSupervisor::<_, RootWalkProbe, NoRoots>::open("build");
        let visits = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let producer_wake = Arc::new(Counter::default());
        let shared = Arc::downgrade(&producer.shared);
        assert!(matches!(
            producer.emit(
                RootWalkProbe {
                    root: ValueSlot::integer(1),
                    shared: shared.clone(),
                    visits: Arc::clone(&visits),
                    drops: Arc::clone(&drops),
                },
                Arc::new(Counter::default()),
            ),
            StreamEmit::Ready
        ));
        assert!(matches!(
            producer.emit(
                RootWalkProbe {
                    root: ValueSlot::integer(2),
                    shared,
                    visits: Arc::clone(&visits),
                    drops: Arc::clone(&drops),
                },
                producer_wake.clone(),
            ),
            StreamEmit::Pending
        ));

        let mut visitor = CountingVisitor::default();
        supervisor.visit_roots(&mut visitor).unwrap();

        assert_eq!(visitor.0, 2);
        assert_eq!(visits.load(Ordering::Relaxed), 2);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        assert_eq!(producer_wake.0.load(Ordering::Relaxed), 0);

        drop(supervisor);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        assert_eq!(producer_wake.0.load(Ordering::Relaxed), 1);
        drop(consumer);
        drop(producer);
    }

    #[test]
    fn empty_first_poll_is_pending_and_ready_item_does_not_park() {
        let (_supervisor, mut producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("exact-provider-build");
        let consumer_wake = Arc::new(Counter::default());
        let producer_wake = Arc::new(Counter::default());

        assert_eq!(
            consumer.poll_next(consumer_wake.clone()),
            StreamPoll::Pending
        );
        assert_eq!(
            producer.emit(7, producer_wake),
            StreamEmit::Ready,
            "a waiting consumer makes the first emit synchronously ready"
        );
        assert_eq!(consumer_wake.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Item(7))
        );
    }

    #[test]
    fn only_a_full_buffer_creates_backpressure_pending() {
        let (supervisor, mut producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");
        let producer_wake = Arc::new(Counter::default());

        assert_eq!(
            producer.emit(1, Arc::new(Counter::default())),
            StreamEmit::Ready
        );
        assert_eq!(producer.emit(2, producer_wake.clone()), StreamEmit::Pending);
        assert!(supervisor.is_backpressured());

        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Item(1))
        );
        assert_eq!(producer_wake.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Item(2))
        );
    }

    #[test]
    fn concurrent_consumer_poll_is_rejected() {
        let (_supervisor, _producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");

        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Pending
        );
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Rejected(StreamError::ConsumerPollAlreadyPending)
        );
    }

    #[test]
    fn terminal_error_is_delivered_once_then_end_is_stable() {
        let (_supervisor, mut producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");
        producer.finish_error("failed").unwrap();

        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Error("failed"))
        );
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::End)
        );
    }

    #[test]
    fn consumer_drop_cancels_and_rejects_late_item() {
        let (_supervisor, mut producer, consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");
        drop(consumer);

        assert_eq!(
            producer.emit(9, Arc::new(Counter::default())),
            StreamEmit::Rejected {
                item: 9,
                reason: StreamError::AlreadyTerminal
            }
        );
    }

    #[test]
    fn supervisor_drop_is_ancestor_stop() {
        let (supervisor, _producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");
        let consumer_wake = Arc::new(Counter::default());
        assert_eq!(
            consumer.poll_next(consumer_wake.clone()),
            StreamPoll::Pending
        );

        drop(supervisor);

        assert_eq!(consumer_wake.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Cancelled)
        );
    }

    #[test]
    fn natural_end_after_backpressure_leaves_no_pending_state() {
        let (supervisor, mut producer, mut consumer) =
            StreamSupervisor::<_, u64, &'static str>::open("build");
        let producer_wake = Arc::new(Counter::default());

        assert_eq!(
            producer.emit(1, Arc::new(Counter::default())),
            StreamEmit::Ready
        );
        assert_eq!(
            producer.emit(2, producer_wake.clone()),
            StreamEmit::Pending
        );
        assert!(supervisor.is_backpressured());

        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Item(1))
        );
        assert_eq!(producer_wake.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::Item(2))
        );

        producer.finish_end().unwrap();
        assert!(!supervisor.is_backpressured());
        assert_eq!(supervisor.buffered_len(), 0);
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::End)
        );
        assert_eq!(
            consumer.poll_next(Arc::new(Counter::default())),
            StreamPoll::Ready(StreamEvent::End)
        );
        assert_eq!(
            producer.finish_end(),
            Err(StreamError::AlreadyTerminal)
        );
    }

    #[test]
    fn cancellation_drops_buffer_and_terminal_payloads_after_unlocking() {
        let (_supervisor, mut producer, consumer) =
            StreamSupervisor::<_, ReentrantDrop, ReentrantDrop>::open("build");
        let shared = Arc::downgrade(&producer.shared);
        let drops = Arc::new(AtomicUsize::new(0));
        assert!(matches!(
            producer.emit(
                ReentrantDrop {
                    shared: shared.clone(),
                    drops: Arc::clone(&drops),
                },
                Arc::new(Counter::default())
            ),
            StreamEmit::Ready
        ));
        assert!(producer
            .finish_error(ReentrantDrop {
                shared,
                drops: Arc::clone(&drops),
            })
            .is_ok());

        drop(consumer);

        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }
}
