use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, Weak,
};

use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
};
use skiff_runtime_vm::{
    ChildInvocation as VmChildInvocation, ChildTarget, PendingOperation, ResumeOutcome,
    StreamItem as VmStreamItem, VmBudget, VmError, VmFiber, VmInternalTerminal, VmOwnedValues,
    VmResult, VmResumeToken,
};

use crate::{
    owner_inventory::PendingOwnerRegistration, BytecodeAdapterHandoff, BytecodeChildExecutor,
    BytecodeChildStart, BytecodeHandoff, BytecodeSchedulerError, BytecodeStreamHandoff,
    BytecodeStreamSupervisor, PendingWakeQueue, RootDisposition, RootEscrow, RootEscrowBacking,
    StreamConsumer, StreamEmit, StreamError, StreamEvent, StreamPoll, StreamProducer,
    StreamSupervisor, SuspendedTrampoline, VmCompletionHandle, VmPendingRegistry, WakeSignal,
};

type VmSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;

/// Terminal event delivered to a `VmStreamConsumer` after the producer exits.
#[derive(Debug, Clone, PartialEq)]
pub enum VmStreamTerminal {
    End,
    Error(VmError),
    Cancelled,
}

impl VmRootSource for VmStreamTerminal {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

struct EmptyRoots;

impl VmRootSource for EmptyRoots {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl RootEscrowBacking for EmptyRoots {
    fn root_count(&self) -> usize {
        0
    }

    fn restore_roots(self: Box<Self>) {}

    fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
}

struct BackpressureWake {
    completion: VmCompletionHandle<VmSuspended>,
    cancelled: Arc<AtomicBool>,
}

impl WakeSignal for BackpressureWake {
    fn wake(&self) {
        if self.cancelled.load(Ordering::Acquire) {
            let _ = self
                .completion
                .internal_stop(ResumeOutcome::InternalTerminal(
                    VmInternalTerminal::OwnerStopped,
                ));
        } else {
            let _ = self.completion.complete(ResumeOutcome::Empty);
        }
    }
}

#[derive(Default)]
struct ConsumerWakeLink(Mutex<Option<Weak<dyn WakeSignal>>>);

struct ConsumerWake<P> {
    consumer: Arc<Mutex<StreamConsumer<P, VmOwnedValues, VmStreamTerminal>>>,
    completion: VmCompletionHandle<VmSuspended>,
    cancelled: Arc<AtomicBool>,
    next: ConsumerWakeLink,
}

impl<P> WakeSignal for ConsumerWake<P>
where
    P: Send + Sync + 'static,
{
    fn wake(&self) {
        if self.cancelled.load(Ordering::Acquire) {
            let _ = self
                .completion
                .internal_stop(ResumeOutcome::InternalTerminal(
                    VmInternalTerminal::OwnerStopped,
                ));
            return;
        }
        let mut consumer = self
            .consumer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = self
            .next
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .and_then(|wake| wake.upgrade())
            .expect("consumer wake self-link is installed");
        match consumer.poll_next(next) {
            StreamPoll::Ready(StreamEvent::Item(values)) => {
                let _ = self.completion.complete(ResumeOutcome::Values(values));
            }
            StreamPoll::Ready(StreamEvent::End) => {
                let _ = self.completion.complete(ResumeOutcome::StreamEnd);
            }
            StreamPoll::Ready(StreamEvent::Error(error)) => {
                let outcome = match error {
                    VmStreamTerminal::End => {
                        unreachable!("End is delivered through StreamEvent::End")
                    }
                    VmStreamTerminal::Error(error) => ResumeOutcome::Failure(error),
                    VmStreamTerminal::Cancelled => {
                        ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped)
                    }
                };
                let _ = self.completion.complete(outcome);
            }
            StreamPoll::Ready(StreamEvent::Cancelled) => {
                let _ = self.completion.complete(ResumeOutcome::InternalTerminal(
                    VmInternalTerminal::OwnerStopped,
                ));
            }
            StreamPoll::Pending => {}
            StreamPoll::Rejected(_reason) => {
                let _ = self
                    .completion
                    .internal_stop(ResumeOutcome::InternalTerminal(
                        VmInternalTerminal::OwnerStopped,
                    ));
            }
        }
    }
}

struct VmStreamConsumerShared<P> {
    consumer: Arc<Mutex<StreamConsumer<P, VmOwnedValues, VmStreamTerminal>>>,
    registry: VmPendingRegistry<VmSuspended>,
    queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>>,
    cancelled: Arc<AtomicBool>,
}

/// A `BytecodeChildExecutor` that maps one affine `StreamConsumer` into VM
/// `StreamNext` resume outcomes.
pub struct VmStreamConsumerExecutor<P> {
    shared: Arc<VmStreamConsumerShared<P>>,
}

impl<P> VmStreamConsumerExecutor<P>
where
    P: Send + Sync + 'static,
{
    pub(crate) fn open(
        consumer: StreamConsumer<P, VmOwnedValues, VmStreamTerminal>,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>>,
        pending_owners: PendingOwnerRegistration,
    ) -> Self {
        Self {
            shared: Arc::new(VmStreamConsumerShared {
                consumer: Arc::new(Mutex::new(consumer)),
                registry: VmPendingRegistry::new(pending_owners),
                queue,
                cancelled: Arc::new(AtomicBool::new(false)),
            }),
        }
    }
}

impl<P> Drop for VmStreamConsumerExecutor<P> {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

impl<P> BytecodeChildExecutor<VmFiber> for VmStreamConsumerExecutor<P>
where
    P: Send + Sync + 'static,
{
    fn execute_child(
        &self,
        _invocation: VmChildInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<VmFiber>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedChild)
    }

    fn execute_adapter(
        &self,
        _invocation: skiff_runtime_vm::AdapterInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedAdapter)
    }

    fn execute_stream_next(
        &self,
        invocation: VmChildInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        if invocation.target() != ChildTarget::StreamNext {
            return Err(BytecodeSchedulerError::UnsupportedChild);
        }
        let (_target, _arguments, resume) = invocation.into_parts();
        let end_resume_pc = resume
            .image()
            .resume_sites()
            .get(resume.resume_site())
            .and_then(|site| site.end_resume());
        if end_resume_pc.is_none() {
            return Err(BytecodeSchedulerError::Port(
                "StreamNext resume site has no verified end continuation".to_string(),
            ));
        }
        let completion = self
            .shared
            .registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let wake = Arc::new(ConsumerWake {
            consumer: Arc::clone(&self.shared.consumer),
            completion: completion.clone(),
            cancelled: Arc::clone(&self.shared.cancelled),
            next: ConsumerWakeLink::default(),
        });
        *wake
            .next
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Arc::downgrade(&wake) as Weak<dyn WakeSignal>);

        let mut consumer = self
            .shared
            .consumer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match consumer.poll_next(Arc::clone(&wake) as Arc<dyn WakeSignal>) {
            StreamPoll::Ready(StreamEvent::Item(values)) => {
                drop(consumer);
                self.shared.registry.abandon(completion.ticket());
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::Values(values),
                }))
            }
            StreamPoll::Ready(StreamEvent::End) => {
                drop(consumer);
                self.shared.registry.abandon(completion.ticket());
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::StreamEnd,
                }))
            }
            StreamPoll::Ready(StreamEvent::Error(error)) => {
                drop(consumer);
                self.shared.registry.abandon(completion.ticket());
                let outcome = match error {
                    VmStreamTerminal::End => {
                        unreachable!("End is delivered through StreamEvent::End")
                    }
                    VmStreamTerminal::Error(error) => ResumeOutcome::Failure(error),
                    VmStreamTerminal::Cancelled => {
                        ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped)
                    }
                };
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome,
                }))
            }
            StreamPoll::Ready(StreamEvent::Cancelled) => {
                drop(consumer);
                self.shared.registry.abandon(completion.ticket());
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped),
                }))
            }
            StreamPoll::Pending => {
                let operation = resume.into_pending(completion.ticket());
                Ok(BytecodeStreamHandoff::Pending(operation))
            }
            StreamPoll::Rejected(reason) => {
                drop(consumer);
                self.shared.registry.abandon(completion.ticket());
                Err(BytecodeSchedulerError::Port(format!(
                    "stream next poll rejected: {reason}"
                )))
            }
        }
    }

    fn park_stream_next(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        self.shared
            .registry
            .publish_operation(operation, suspended, Arc::clone(&self.shared.queue))
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    }
}

struct VmStreamShared<P> {
    supervisor: StreamSupervisor<P, VmOwnedValues, VmStreamTerminal>,
    producer: Mutex<StreamProducer<P, VmOwnedValues, VmStreamTerminal>>,
    registry: VmPendingRegistry<VmSuspended>,
    queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>>,
    active_depth: Mutex<Option<usize>>,
    cancelled: Arc<AtomicBool>,
}

impl<P> Drop for VmStreamShared<P> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// A `BytecodeStreamSupervisor` backed by the crate's affine stream state and
/// pending registry.
///
/// `open` returns the consumer endpoint to the external response sink before
/// the scheduler installs/runs the producer. The producer side is retained by
/// this supervisor so `EmitStream` can use real backpressure and `finish_stream`
/// can emit exactly one natural terminal.
pub struct VmStreamSupervisor<P> {
    shared: Arc<VmStreamShared<P>>,
}

impl<P> VmStreamSupervisor<P>
where
    P: Clone + Send + Sync + 'static,
{
    pub(crate) fn open(
        owner_pin: P,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>>,
        pending_owners: PendingOwnerRegistration,
    ) -> (Self, StreamConsumer<P, VmOwnedValues, VmStreamTerminal>) {
        let (supervisor, producer, consumer) = StreamSupervisor::open(owner_pin);
        (
            Self {
                shared: Arc::new(VmStreamShared {
                    supervisor,
                    producer: Mutex::new(producer),
                    registry: VmPendingRegistry::new(pending_owners),
                    queue,
                    active_depth: Mutex::new(None),
                    cancelled: Arc::new(AtomicBool::new(false)),
                }),
            },
            consumer,
        )
    }

    pub fn owner_pin(&self) -> &P {
        self.shared.supervisor.owner_pin()
    }
}

impl<P> Drop for VmStreamSupervisor<P> {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

impl<P> BytecodeStreamSupervisor<VmFiber> for VmStreamSupervisor<P>
where
    P: Clone + Send + Sync + 'static,
{
    fn emit_stream_handoff(
        &self,
        item: VmStreamItem,
        depth: usize,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        let (item_value, resume) = item.into_parts();
        let completion = self
            .shared
            .registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let wake = Arc::new(BackpressureWake {
            completion: completion.clone(),
            cancelled: Arc::clone(&self.shared.cancelled),
        });

        let mut producer = self
            .shared
            .producer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match producer.emit(item_value, wake) {
            StreamEmit::Ready => {
                drop(producer);
                self.shared.registry.abandon(completion.ticket());
                *self
                    .shared
                    .active_depth
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(depth);
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::Empty,
                }))
            }
            StreamEmit::Pending => {
                let operation = resume.into_pending(completion.ticket());
                *self
                    .shared
                    .active_depth
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(depth);
                Ok(BytecodeStreamHandoff::Pending(operation))
            }
            StreamEmit::Rejected {
                item: _item,
                reason,
            } => {
                drop(producer);
                self.shared.registry.abandon(completion.ticket());
                Err(BytecodeSchedulerError::Port(format!(
                    "stream emit rejected: {reason}"
                )))
            }
        }
    }

    fn park(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        self.shared
            .registry
            .publish_operation(operation, suspended, Arc::clone(&self.shared.queue))
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    }

    fn finish_stream(&self, depth: usize, result: &VmResult) -> Result<(), BytecodeSchedulerError> {
        let mut active_depth = self
            .shared
            .active_depth
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *active_depth != Some(depth) {
            return Ok(());
        }
        *active_depth = None;

        let mut producer = self
            .shared
            .producer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(_) => producer
                .finish_end()
                .map_err(|error| BytecodeSchedulerError::Port(error.to_string())),
            Err(error) => {
                if let Err((error, _)) =
                    producer.finish_error(VmStreamTerminal::Error(error.clone()))
                {
                    if error == StreamError::AlreadyTerminal
                        && self.shared.supervisor.is_cancelled()
                    {
                        Ok(())
                    } else {
                        Err(BytecodeSchedulerError::Port(error.to_string()))
                    }
                } else {
                    Ok(())
                }
            }
        }
    }
}
