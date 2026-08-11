use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
};
use skiff_runtime_vm::{
    PendingOperation, ResumeOutcome, VmBudget, VmError, VmFiber, VmInternalTerminal,
    StreamItem as VmStreamItem, VmOwnedValues, VmResult, VmResumeToken,
};

use crate::{
    BytecodeHandoff, BytecodeSchedulerError, BytecodeStreamHandoff, BytecodeStreamSupervisor,
    PendingWakeQueue, RootDisposition, RootEscrow, RootEscrowBacking, StreamConsumer, StreamError,
    StreamEmit, StreamProducer, StreamSupervisor, SuspendedTrampoline, VmCompletionHandle,
    VmPendingRegistry, WakeSignal,
};

type VmSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;

/// Terminal event delivered to a `VmStreamConsumer` after the producer exits.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            let _ = self.completion.internal_stop(ResumeOutcome::InternalTerminal(
                VmInternalTerminal::OwnerStopped,
            ));
        } else {
            let _ = self.completion.complete(ResumeOutcome::Empty);
        }
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
    pub fn open(
        owner_pin: P,
        queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>>,
    ) -> (
        Self,
        StreamConsumer<P, VmOwnedValues, VmStreamTerminal>,
    ) {
        let (supervisor, producer, consumer) = StreamSupervisor::open(owner_pin);
        (
            Self {
                shared: Arc::new(VmStreamShared {
                    supervisor,
                    producer: Mutex::new(producer),
                    registry: VmPendingRegistry::default(),
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
            StreamEmit::Rejected { item: _item, reason } => {
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

    fn finish_stream(
        &self,
        depth: usize,
        result: &VmResult,
    ) -> Result<(), BytecodeSchedulerError> {
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
                if let Err((error, _)) = producer.finish_error(VmStreamTerminal::Error(error.clone()))
                {
                    if error == StreamError::AlreadyTerminal && self.shared.supervisor.is_cancelled()
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
