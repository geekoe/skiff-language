use std::{
    num::NonZeroU32,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::ValueSlot,
};
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeControl,
    BytecodeHandoff, BytecodeScheduler,
    BytecodeSchedulerError, BytecodeSchedulerOutcome, BytecodeSchedulerPorts,
    BytecodeStreamHandoff, BytecodeStreamSupervisor, BytecodeUnit, RootDisposition, RootEscrow,
    RootEscrowBacking, SuspendedTrampoline,
};
use skiff_runtime_vm::{VmBudget, VmBudgetError, VmSemanticCharge};

struct NoopHeap;

impl VmHeap for NoopHeap {
    fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Ok(*source)
    }

    fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }
}

struct NoopBudget;

impl VmBudget for NoopBudget {
    fn replenish_raw_fuel(&mut self, maximum: NonZeroU32) -> Result<NonZeroU32, VmBudgetError> {
        Ok(maximum)
    }

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetError> {
        Ok(())
    }

    fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetError> {
        Ok(())
    }
}

type ChainControl = BytecodeControl<usize, usize, usize, usize, usize>;

struct ChainUnit {
    id: usize,
    remaining_children: usize,
    spawned: bool,
    resumes: Arc<AtomicUsize>,
}

impl ChainUnit {
    fn new(id: usize, remaining_children: usize, resumes: Arc<AtomicUsize>) -> Self {
        Self {
            id,
            remaining_children,
            spawned: false,
            resumes,
        }
    }
}

impl VmRootSource for ChainUnit {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl BytecodeUnit for ChainUnit {
    type ResumeToken = usize;
    type ResumeOutcome = usize;
    type RootResult = usize;
    type ChildInvocation = usize;
    type AdapterInvocation = usize;
    type StreamItem = usize;
    type PendingOperation = usize;

    fn run_segment(&mut self, _heap: &mut dyn VmHeap, _budget: &mut dyn VmBudget) -> ChainControl {
        if !self.spawned && self.remaining_children > 0 {
            self.spawned = true;
            ChainControl::EnterChild(self.remaining_children - 1)
        } else {
            ChainControl::Complete(self.id)
        }
    }

    fn resume(&mut self, _token: usize, _outcome: usize) -> Result<(), BytecodeSchedulerError> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn child_completion_to_resume_outcome(completed: usize) -> usize {
        completed
    }
}

struct ChainExecutor {
    starts: AtomicUsize,
    resumes: Arc<AtomicUsize>,
}

impl ChainExecutor {
    fn new(resumes: Arc<AtomicUsize>) -> Self {
        Self {
            starts: AtomicUsize::new(0),
            resumes,
        }
    }
}

impl BytecodeChildExecutor<ChainUnit> for ChainExecutor {
    fn execute_child(
        &self,
        invocation: usize,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<ChainUnit>, BytecodeSchedulerError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(BytecodeChildStart {
            unit: ChainUnit::new(invocation, invocation, Arc::clone(&self.resumes)),
            resume: invocation,
        })
    }

    fn execute_adapter(
        &self,
        _invocation: usize,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<ChainUnit>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedAdapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_child_chain_stays_flat_and_each_parent_resumes_exactly_once() {
        const DEPTH: usize = 20_000;

        let resumes = Arc::new(AtomicUsize::new(0));
        let executor = Arc::new(ChainExecutor::new(Arc::clone(&resumes)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(Arc::clone(&executor) as Arc<dyn BytecodeChildExecutor<ChainUnit>>),
            stream_supervisor: None,
        };
        let root = ChainUnit::new(0, DEPTH, Arc::clone(&resumes));

        let outcome = BytecodeScheduler::new(root, ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(outcome, BytecodeSchedulerOutcome::Complete(0)));
        assert_eq!(executor.starts.load(Ordering::SeqCst), DEPTH);
        assert_eq!(resumes.load(Ordering::SeqCst), DEPTH);
    }

    #[test]
    fn parent_resumes_exactly_once_for_a_synchronous_child() {
        let resumes = Arc::new(AtomicUsize::new(0));
        let executor = Arc::new(ChainExecutor::new(Arc::clone(&resumes)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(Arc::clone(&executor) as Arc<dyn BytecodeChildExecutor<ChainUnit>>),
            stream_supervisor: None,
        };

        let outcome = BytecodeScheduler::new(ChainUnit::new(0, 1, resumes.clone()), ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(outcome, BytecodeSchedulerOutcome::Complete(0)));
        assert_eq!(executor.starts.load(Ordering::SeqCst), 1);
        assert_eq!(resumes.load(Ordering::SeqCst), 1);
    }

    type StreamResult = Result<usize, &'static str>;
    type StreamControl = BytecodeControl<StreamResult, usize, usize, usize, usize>;

    enum StreamMode {
        Emit(usize),
        Complete(StreamResult),
    }

    struct StreamUnit {
        mode: StreamMode,
        resumes: Arc<AtomicUsize>,
    }

    impl StreamUnit {
        fn new(mode: StreamMode) -> Self {
            Self {
                mode,
                resumes: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl VmRootSource for StreamUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for StreamUnit {
        type ResumeToken = usize;
        type ResumeOutcome = StreamResult;
        type RootResult = StreamResult;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> StreamControl {
            match std::mem::replace(&mut self.mode, StreamMode::Complete(Ok(0))) {
                StreamMode::Emit(item) => StreamControl::EmitStream(item),
                StreamMode::Complete(result) => StreamControl::Complete(result),
            }
        }

        fn resume(
            &mut self,
            token: usize,
            outcome: StreamResult,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            self.mode = StreamMode::Complete(outcome);
            let _ = token;
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: StreamResult) -> StreamResult {
            completed
        }
    }

    enum StreamPortMode {
        Ready,
        Error,
    }

    struct RecordingStream {
        emitted: Mutex<Vec<usize>>,
        finished: Mutex<Vec<StreamResult>>,
        mode: StreamPortMode,
    }

    impl BytecodeStreamSupervisor<StreamUnit> for RecordingStream {
        fn emit_stream(
            &self,
            item: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeHandoff<StreamUnit>, BytecodeSchedulerError> {
            self.emitted.lock().unwrap().push(item);
            match self.mode {
                StreamPortMode::Ready => Ok(BytecodeHandoff {
                    resume: item,
                    outcome: Ok(99),
                }),
                StreamPortMode::Error => Ok(BytecodeHandoff {
                    resume: item,
                    outcome: Err("stream failed"),
                }),
            }
        }

        fn finish_stream(
            &self,
            _depth: usize,
            result: &StreamResult,
        ) -> Result<(), BytecodeSchedulerError> {
            self.finished.lock().unwrap().push(*result);
            Ok(())
        }

        fn park(
            &self,
            _operation: usize,
            _suspended: skiff_runtime_scheduler::SuspendedTrampoline<StreamUnit, usize>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedPark)
        }
    }

    #[test]
    fn stream_item_is_handed_to_supervisor_and_resumes_producer() {
        let stream = Arc::new(RecordingStream {
            emitted: Mutex::new(Vec::new()),
            finished: Mutex::new(Vec::new()),
            mode: StreamPortMode::Ready,
        });
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(stream.clone() as Arc<dyn BytecodeStreamSupervisor<StreamUnit>>),
        };

        let outcome = BytecodeScheduler::new(StreamUnit::new(StreamMode::Emit(7)), ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(Ok(99))
        ));
        assert_eq!(*stream.emitted.lock().unwrap(), [7]);
        assert_eq!(*stream.finished.lock().unwrap(), [Ok(99)]);
    }

    #[test]
    fn stream_error_handoff_resumes_with_error() {
        let stream = Arc::new(RecordingStream {
            emitted: Mutex::new(Vec::new()),
            finished: Mutex::new(Vec::new()),
            mode: StreamPortMode::Error,
        });
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(stream.clone() as Arc<dyn BytecodeStreamSupervisor<StreamUnit>>),
        };

        let outcome = BytecodeScheduler::new(StreamUnit::new(StreamMode::Emit(9)), ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(Err("stream failed"))
        ));
        assert_eq!(*stream.emitted.lock().unwrap(), [9]);
        assert_eq!(*stream.finished.lock().unwrap(), [Err("stream failed")]);
    }

    #[test]
    fn absent_ports_fail_closed() {
        let ports = BytecodeSchedulerPorts::<ChainUnit>::default();
        let child =
            BytecodeScheduler::new(ChainUnit::new(0, 1, Arc::new(AtomicUsize::new(0))), ports)
                .run(&mut NoopHeap, &mut NoopBudget)
                .unwrap_err();
        assert!(matches!(child, BytecodeSchedulerError::UnsupportedChild));

        let ports = BytecodeSchedulerPorts::<StreamUnit>::default();
        let stream = BytecodeScheduler::new(StreamUnit::new(StreamMode::Emit(1)), ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap_err();
        assert!(matches!(stream, BytecodeSchedulerError::UnsupportedStream));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NextResume {
        Item,
        End,
        Pending,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NextOutcome {
        Values(usize),
        End,
        Failure(&'static str),
    }

    #[derive(Debug)]
    enum NextInvocation {
        StreamNext {
            item_resume: NextResume,
            end_resume: NextResume,
        },
    }

    type NextControl = BytecodeControl<NextOutcome, NextInvocation, usize, usize, NextResume>;
    type NextSuspended = SuspendedTrampoline<NextUnit, NextResume>;
    struct NextUnit {
        invocation: Option<NextInvocation>,
        resumed: Option<(NextResume, NextOutcome)>,
    }

    impl NextUnit {
        fn stream_next() -> Self {
            Self {
                invocation: Some(NextInvocation::StreamNext {
                    item_resume: NextResume::Item,
                    end_resume: NextResume::End,
                }),
                resumed: None,
            }
        }
    }

    impl VmRootSource for NextUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for NextUnit {
        type ResumeToken = NextResume;
        type ResumeOutcome = NextOutcome;
        type RootResult = NextOutcome;
        type ChildInvocation = NextInvocation;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = NextResume;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> NextControl {
            if let Some((_, outcome)) = self.resumed.take() {
                NextControl::Complete(outcome)
            } else if let Some(invocation) = self.invocation.take() {
                NextControl::EnterChild(invocation)
            } else {
                NextControl::Complete(NextOutcome::Values(0))
            }
        }

        fn resume(
            &mut self,
            token: NextResume,
            outcome: NextOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: NextOutcome) -> NextOutcome {
            completed
        }

        fn is_stream_next_child(invocation: &NextInvocation) -> bool {
            matches!(invocation, NextInvocation::StreamNext { .. })
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NextExecutorMode {
        Item,
        End,
        Error,
        Pending,
    }

    struct NextExecutor {
        mode: NextExecutorMode,
        pending: Mutex<Option<(NextResume, NextSuspended)>>,
    }

    impl BytecodeChildExecutor<NextUnit> for NextExecutor {
        fn execute_child(
            &self,
            _invocation: NextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildStart<NextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedChild)
        }

        fn execute_adapter(
            &self,
            _invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<NextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        }

        fn execute_stream_next(
            &self,
            invocation: NextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<NextUnit>, BytecodeSchedulerError> {
            let NextInvocation::StreamNext {
                item_resume,
                end_resume,
            } = invocation;
            match self.mode {
                NextExecutorMode::Item => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: item_resume,
                    outcome: NextOutcome::Values(7),
                })),
                NextExecutorMode::End => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: end_resume,
                    outcome: NextOutcome::End,
                })),
                NextExecutorMode::Error => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: item_resume,
                    outcome: NextOutcome::Failure("stream failed"),
                })),
                NextExecutorMode::Pending => {
                    Ok(BytecodeStreamHandoff::Pending(NextResume::Pending))
                }
            }
        }

        fn park_stream_next(
            &self,
            operation: NextResume,
            suspended: NextSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.pending.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct EmptyNextRoots;

    impl VmRootSource for EmptyNextRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for EmptyNextRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {}

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
    }

    fn next_ports(executor: Arc<NextExecutor>) -> BytecodeSchedulerPorts<NextUnit> {
        BytecodeSchedulerPorts {
            child_executor: Some(executor.clone() as Arc<dyn BytecodeChildExecutor<NextUnit>>),
            stream_supervisor: None,
        }
    }

    #[test]
    fn stream_next_item_resumes_with_values_and_item_continuation() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::Item,
            pending: Mutex::new(None),
        });

        let outcome = BytecodeScheduler::new(NextUnit::stream_next(), next_ports(executor))
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(NextOutcome::Values(7))
        ));
    }

    #[test]
    fn stream_next_end_resumes_with_stream_end_and_end_continuation() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::End,
            pending: Mutex::new(None),
        });

        let outcome = BytecodeScheduler::new(NextUnit::stream_next(), next_ports(executor))
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(NextOutcome::End)
        ));
    }

    #[test]
    fn stream_next_error_resumes_with_existing_failure_path() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::Error,
            pending: Mutex::new(None),
        });

        let outcome = BytecodeScheduler::new(NextUnit::stream_next(), next_ports(executor))
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(NextOutcome::Failure("stream failed"))
        ));
    }

    #[test]
    fn stream_next_pending_wake_end_does_not_produce_an_item() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::Pending,
            pending: Mutex::new(None),
        });
        let ports = next_ports(Arc::clone(&executor));

        let outcome = BytecodeScheduler::new(NextUnit::stream_next(), ports)
            .run(&mut NoopHeap, &mut NoopBudget)
            .unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Parked));

        let (operation, suspended) = executor.pending.lock().unwrap().take().unwrap();
        assert_eq!(operation, NextResume::Pending);

        let scheduler = BytecodeScheduler::<NextUnit>::resume_from_suspended(
            suspended,
            operation,
            NextOutcome::End,
            RootEscrow::new(Box::new(EmptyNextRoots)),
            next_ports(Arc::clone(&executor)),
        )
        .unwrap();
        let outcome = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap();

        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(NextOutcome::End)
        ));
    }
}
