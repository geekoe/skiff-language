use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use skiff_runtime_model::{
    memory_ledger::{MemoryLease, MemoryLeaseHost, MemoryLeaseToken},
    vm_heap::{HeapDomainId, HeapEpoch, VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::ValueSlot,
};
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildHandoff, BytecodeChildStart,
    BytecodeControl, BytecodeHandoff, BytecodeParkFailure, BytecodeParkRequest,
    BytecodePortFailure, BytecodeResumeFailure, BytecodeSchedulerError, BytecodeSchedulerOutcome,
    BytecodeSchedulerPorts, BytecodeStreamHandoff, BytecodeStreamSupervisor, BytecodeUnit,
    BytecodeUnitControl, ChildFinish, ChildFinishError, PendingOwnerDraft, RequestExecutionContext,
};
use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmSemanticCharge};
use std::num::NonZeroU64;

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
    fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
        Ok(())
    }

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
        Ok(())
    }

    fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
        Ok(())
    }
}

struct TestMemoryHost;

impl MemoryLeaseHost for TestMemoryHost {
    fn release_lease(&self, _token: MemoryLeaseToken, _amount: usize) {}
}

fn test_child_heap() -> skiff_runtime_scheduler::ChildHeapCarrier {
    let context = RequestExecutionContext::<ChainUnit>::create(BytecodeSchedulerPorts::default());
    skiff_runtime_scheduler::ChildHeapCarrier::new(
        Box::new(NoopHeap),
        HeapDomainId::try_new(1).unwrap(),
        HeapEpoch::new(0),
        MemoryLease::new(
            Arc::new(TestMemoryHost),
            MemoryLeaseToken::new(NonZeroU64::new(1).unwrap()),
            1,
        ),
        context.child_heap_registration().mint_lease().unwrap(),
    )
}

struct ChainFinish;

impl ChildFinish<ChainUnit, <ChainUnit as BytecodeUnit>::ResumeToken> for ChainFinish {
    fn finish(
        &self,
        _resume: &<ChainUnit as BytecodeUnit>::ResumeToken,
        child_result: usize,
        _child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        _parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<usize, ChildFinishError<ChainUnit>> {
        Ok(child_result)
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

    fn resume(
        &mut self,
        _token: usize,
        _outcome: usize,
    ) -> Result<(), BytecodeResumeFailure<usize, usize>> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
    ) -> Result<BytecodeChildHandoff<ChainUnit>, BytecodePortFailure<usize, usize>> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
            unit: ChainUnit::new(invocation, invocation, Arc::clone(&self.resumes)),
            resume: invocation,
            child_heap: test_child_heap(),
            finish: Box::new(ChainFinish),
        }))
    }

    fn execute_adapter(
        &self,
        invocation: usize,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<ChainUnit>, BytecodePortFailure<usize, usize>> {
        Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedAdapter,
            invocation,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_stream_next_child_enters_and_finishes_through_phase_6_executor() {
        const DEPTH: usize = 64;

        let resumes = Arc::new(AtomicUsize::new(0));
        let executor = Arc::new(ChainExecutor::new(Arc::clone(&resumes)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(Arc::clone(&executor) as Arc<dyn BytecodeChildExecutor<ChainUnit>>),
            stream_supervisor: None,
        };
        let root = ChainUnit::new(0, DEPTH, Arc::clone(&resumes));

        let mut context = RequestExecutionContext::create(ports);
        context.install_root(root);
        let (outcome, snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(outcome, Ok(BytecodeSchedulerOutcome::Complete(0))));
        assert_eq!(executor.starts.load(Ordering::SeqCst), DEPTH);
        assert_eq!(resumes.load(Ordering::SeqCst), DEPTH);
        assert_eq!(snapshot.child.current, 0);
        assert!(snapshot.child.ever_created);
    }

    #[test]
    fn phase_6_sync_child_completes_without_pending_owner() {
        let resumes = Arc::new(AtomicUsize::new(0));
        let executor = Arc::new(ChainExecutor::new(Arc::clone(&resumes)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(Arc::clone(&executor) as Arc<dyn BytecodeChildExecutor<ChainUnit>>),
            stream_supervisor: None,
        };
        let mut context = RequestExecutionContext::create(ports);
        context.install_root(ChainUnit::new(0, 1, Arc::clone(&resumes)));
        let (outcome, snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(outcome, Ok(BytecodeSchedulerOutcome::Complete(0))));
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
        assert_eq!(snapshot.child.current, 0);
        assert!(snapshot.child.ever_created);
    }

    #[test]
    fn phase_6_child_heap_pending_cleanup_runs_with_the_owner_bundle() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let mut carrier = test_child_heap();
        carrier
            .attach_pending_cleanup(Box::new({
                let cleanups = Arc::clone(&cleanups);
                move || {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .expect("first pending cleanup attaches");
        assert!(carrier.attach_pending_cleanup(Box::new(|| {})).is_err());
        assert_eq!(cleanups.load(Ordering::SeqCst), 0);

        drop(carrier);
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);

        let mut detached = test_child_heap();
        detached
            .attach_pending_cleanup(Box::new({
                let cleanups = Arc::clone(&cleanups);
                move || {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .expect("detached cleanup attaches");
        let cleanup = detached
            .take_pending_cleanup()
            .expect("exact cleanup can be detached before terminal");
        drop(detached);
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        cleanup();
        assert_eq!(cleanups.load(Ordering::SeqCst), 2);
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
        ) -> Result<(), BytecodeResumeFailure<usize, StreamResult>> {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            self.mode = StreamMode::Complete(outcome);
            let _ = token;
            Ok(())
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
        fn emit_stream_handoff(
            &self,
            item: usize,
            _depth: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<StreamUnit>, BytecodePortFailure<usize, usize>> {
            self.emitted.lock().unwrap().push(item);
            Ok(BytecodeStreamHandoff::Ready(match self.mode {
                StreamPortMode::Ready => BytecodeHandoff {
                    resume: item,
                    outcome: Ok(99),
                },
                StreamPortMode::Error => BytecodeHandoff {
                    resume: item,
                    outcome: Err("stream failed"),
                },
            }))
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
            request: BytecodeParkRequest<StreamUnit>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeParkFailure<StreamUnit>> {
            Err(BytecodeParkFailure::unaccepted(
                BytecodeSchedulerError::UnsupportedPark,
                request,
            ))
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

        let mut context = RequestExecutionContext::create(ports);
        context.install_root(StreamUnit::new(StreamMode::Emit(7)));
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(
            outcome,
            Ok(BytecodeSchedulerOutcome::Complete(Ok(99)))
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

        let mut context = RequestExecutionContext::create(ports);
        context.install_root(StreamUnit::new(StreamMode::Emit(9)));
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(
            outcome,
            Ok(BytecodeSchedulerOutcome::Complete(Err("stream failed")))
        ));
        assert_eq!(*stream.emitted.lock().unwrap(), [9]);
        assert_eq!(*stream.finished.lock().unwrap(), [Err("stream failed")]);
    }

    #[test]
    fn absent_ports_fail_closed() {
        let mut context =
            RequestExecutionContext::create(BytecodeSchedulerPorts::<ChainUnit>::default());
        context.install_root(ChainUnit::new(0, 1, Arc::new(AtomicUsize::new(0))));
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        let failure = outcome.unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedChild
        ));

        let mut context =
            RequestExecutionContext::create(BytecodeSchedulerPorts::<StreamUnit>::default());
        context.install_root(StreamUnit::new(StreamMode::Emit(1)));
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        let failure = outcome.unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedStream
        ));
    }

    struct DropProbe {
        drops: Arc<AtomicUsize>,
    }

    impl DropProbe {
        fn new(drops: &Arc<AtomicUsize>) -> Self {
            Self {
                drops: Arc::clone(drops),
            }
        }

        fn sibling(&self) -> Self {
            Self {
                drops: Arc::clone(&self.drops),
            }
        }
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Clone, Copy)]
    enum ProbeAction {
        Adapter,
        StreamNext,
        Emit,
        Park,
        Complete,
    }

    struct OwnerProbeUnit {
        action: ProbeAction,
        probe: Option<DropProbe>,
        reject_resume: bool,
    }

    impl OwnerProbeUnit {
        fn new(action: ProbeAction, drops: &Arc<AtomicUsize>) -> Self {
            Self {
                action,
                probe: Some(DropProbe::new(drops)),
                reject_resume: false,
            }
        }
    }

    impl VmRootSource for OwnerProbeUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for OwnerProbeUnit {
        type ResumeToken = DropProbe;
        type ResumeOutcome = DropProbe;
        type RootResult = DropProbe;
        type ChildInvocation = DropProbe;
        type AdapterInvocation = DropProbe;
        type StreamItem = DropProbe;
        type PendingOperation = DropProbe;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            let probe = self
                .probe
                .take()
                .expect("the owner probe emits exactly one control");
            match self.action {
                ProbeAction::Adapter => BytecodeControl::EnterAdapter(probe),
                ProbeAction::StreamNext => BytecodeControl::EnterChild(probe),
                ProbeAction::Emit => BytecodeControl::EmitStream(probe),
                ProbeAction::Park => BytecodeControl::Park(probe),
                ProbeAction::Complete => BytecodeControl::Complete(probe),
            }
        }

        fn resume(
            &mut self,
            resume: DropProbe,
            outcome: DropProbe,
        ) -> Result<(), BytecodeResumeFailure<DropProbe, DropProbe>> {
            if self.reject_resume {
                return Err(BytecodeResumeFailure::Rejected {
                    reason: BytecodeSchedulerError::Port(
                        "owner probe rejected its ready resume".to_string(),
                    ),
                    resume,
                    outcome,
                });
            }
            drop((resume, outcome));
            Ok(())
        }

        fn is_stream_next_child(_invocation: &DropProbe) -> bool {
            true
        }
    }

    #[derive(Clone, Copy)]
    enum ProbePortDisposition {
        Input,
        Continuation,
        Ready,
    }

    struct ProbeExecutor(ProbePortDisposition);

    impl BytecodeChildExecutor<OwnerProbeUnit> for ProbeExecutor {
        fn execute_child(
            &self,
            _invocation: DropProbe,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildHandoff<OwnerProbeUnit>, BytecodePortFailure<DropProbe, DropProbe>>
        {
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                _invocation,
            ))
        }

        fn execute_adapter(
            &self,
            invocation: DropProbe,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<OwnerProbeUnit>, BytecodePortFailure<DropProbe, DropProbe>>
        {
            match self.0 {
                ProbePortDisposition::Input => Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port("adapter rejected input".to_string()),
                    invocation,
                )),
                ProbePortDisposition::Continuation => Err(BytecodePortFailure::continuation(
                    BytecodeSchedulerError::Port("adapter rejected continuation".to_string()),
                    invocation,
                )),
                ProbePortDisposition::Ready => {
                    let outcome = invocation.sibling();
                    Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
                        resume: invocation,
                        outcome,
                    }))
                }
            }
        }

        fn execute_stream_next(
            &self,
            invocation: DropProbe,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<OwnerProbeUnit>, BytecodePortFailure<DropProbe, DropProbe>>
        {
            match self.0 {
                ProbePortDisposition::Input => Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port("stream next rejected input".to_string()),
                    invocation,
                )),
                ProbePortDisposition::Continuation => Err(BytecodePortFailure::continuation(
                    BytecodeSchedulerError::Port("stream next rejected continuation".to_string()),
                    invocation,
                )),
                ProbePortDisposition::Ready => unreachable!(),
            }
        }
    }

    struct ProbeSupervisor {
        disposition: ProbePortDisposition,
        fail_finish: bool,
        return_pending_draft: bool,
    }

    impl BytecodeStreamSupervisor<OwnerProbeUnit> for ProbeSupervisor {
        fn emit_stream_handoff(
            &self,
            item: DropProbe,
            _depth: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<OwnerProbeUnit>, BytecodePortFailure<DropProbe, DropProbe>>
        {
            match self.disposition {
                ProbePortDisposition::Input => Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port("stream emit rejected input".to_string()),
                    item,
                )),
                ProbePortDisposition::Continuation => Err(BytecodePortFailure::continuation(
                    BytecodeSchedulerError::Port("stream emit rejected continuation".to_string()),
                    item,
                )),
                ProbePortDisposition::Ready => unreachable!(),
            }
        }

        fn park(
            &self,
            request: BytecodeParkRequest<OwnerProbeUnit>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeParkFailure<OwnerProbeUnit>> {
            if self.return_pending_draft {
                let (operation, suspended) = request.into_parts();
                return Err(BytecodeParkFailure::pending_draft(
                    BytecodeSchedulerError::Port("pending registry rejected draft".to_string()),
                    PendingOwnerDraft::new(operation, suspended),
                ));
            }
            Err(BytecodeParkFailure::unaccepted(
                BytecodeSchedulerError::UnsupportedPark,
                request,
            ))
        }

        fn finish_stream(
            &self,
            _depth: usize,
            _result: &DropProbe,
        ) -> Result<(), BytecodeSchedulerError> {
            if self.fail_finish {
                Err(BytecodeSchedulerError::Port(
                    "finish stream rejected result".to_string(),
                ))
            } else {
                Ok(())
            }
        }
    }

    fn probe_ports(
        action: ProbeAction,
        disposition: ProbePortDisposition,
    ) -> BytecodeSchedulerPorts<OwnerProbeUnit> {
        match action {
            ProbeAction::Adapter | ProbeAction::StreamNext => BytecodeSchedulerPorts {
                child_executor: Some(Arc::new(ProbeExecutor(disposition))),
                stream_supervisor: None,
            },
            ProbeAction::Emit => BytecodeSchedulerPorts {
                child_executor: None,
                stream_supervisor: Some(Arc::new(ProbeSupervisor {
                    disposition,
                    fail_finish: false,
                    return_pending_draft: false,
                })),
            },
            ProbeAction::Park | ProbeAction::Complete => unreachable!(),
        }
    }

    #[test]
    fn every_port_failure_retains_input_and_continuation_owners() {
        for action in [
            ProbeAction::Adapter,
            ProbeAction::StreamNext,
            ProbeAction::Emit,
        ] {
            for disposition in [
                ProbePortDisposition::Input,
                ProbePortDisposition::Continuation,
            ] {
                let drops = Arc::new(AtomicUsize::new(0));
                let mut context = RequestExecutionContext::create(probe_ports(action, disposition));
                context.install_root(OwnerProbeUnit::new(action, &drops));
                let (result, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
                let failure = result.unwrap_err();
                assert_eq!(drops.load(Ordering::SeqCst), 0);
                let (_reason, owner) = failure.into_parts();
                assert_eq!(drops.load(Ordering::SeqCst), 0);
                drop(owner);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[test]
    fn ready_resume_rejection_retains_resume_and_outcome() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut unit = OwnerProbeUnit::new(ProbeAction::Adapter, &drops);
        unit.reject_resume = true;
        let mut context = RequestExecutionContext::create(probe_ports(
            ProbeAction::Adapter,
            ProbePortDisposition::Ready,
        ));
        context.install_root(unit);
        let (result, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        let failure = result.unwrap_err();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::Port(message)
                if message == "owner probe rejected its ready resume"
        ));
        let (_reason, owner) = failure.into_parts();
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn finish_stream_failure_retains_complete_result() {
        let drops = Arc::new(AtomicUsize::new(0));
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(Arc::new(ProbeSupervisor {
                disposition: ProbePortDisposition::Input,
                fail_finish: true,
                return_pending_draft: false,
            })),
        };
        let mut context = RequestExecutionContext::create(ports);
        context.install_root(OwnerProbeUnit::new(ProbeAction::Complete, &drops));
        let (result, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        let failure = result.unwrap_err();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let (_reason, owner) = failure.into_parts();
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ordinary_root_success_moves_the_completion_owner_once() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut context = RequestExecutionContext::create(BytecodeSchedulerPorts::default());
        context.install_root(OwnerProbeUnit::new(ProbeAction::Complete, &drops));
        let (result, snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        let BytecodeSchedulerOutcome::Complete(completion) = result.unwrap() else {
            panic!("ordinary root success must return the exact completion owner")
        };
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(completion);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn park_failure_retains_registry_transformed_pending_draft() {
        let drops = Arc::new(AtomicUsize::new(0));
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(Arc::new(ProbeSupervisor {
                disposition: ProbePortDisposition::Input,
                fail_finish: false,
                return_pending_draft: true,
            })),
        };
        let mut context = RequestExecutionContext::create(ports);
        context.install_root(OwnerProbeUnit::new(ProbeAction::Park, &drops));
        let (result, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        let failure = result.unwrap_err();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let (_reason, owner) = failure.into_parts();
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NextResume {
        Item,
        End,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum NextOutcome {
        Values(usize),
        End,
        Failure(&'static str),
    }

    impl VmRootSource for NextOutcome {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    enum NextInvocation {
        StreamNext {
            item_resume: NextResume,
            end_resume: NextResume,
        },
    }

    type NextControl = BytecodeControl<NextOutcome, NextInvocation, usize, usize, NextResume>;
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
        ) -> Result<(), BytecodeResumeFailure<NextResume, NextOutcome>> {
            self.resumed = Some((token, outcome));
            Ok(())
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
    }

    struct NextExecutor {
        mode: NextExecutorMode,
    }

    impl BytecodeChildExecutor<NextUnit> for NextExecutor {
        fn execute_child(
            &self,
            _invocation: NextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildHandoff<NextUnit>, BytecodePortFailure<NextInvocation, NextResume>>
        {
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                _invocation,
            ))
        }

        fn execute_adapter(
            &self,
            invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<NextUnit>, BytecodePortFailure<usize, NextResume>>
        {
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedAdapter,
                invocation,
            ))
        }

        fn execute_stream_next(
            &self,
            invocation: NextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<NextUnit>, BytecodePortFailure<NextInvocation, NextResume>>
        {
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
            }
        }
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
        });

        let mut context = RequestExecutionContext::create(next_ports(executor));
        context.install_root(NextUnit::stream_next());
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(
            outcome,
            Ok(BytecodeSchedulerOutcome::Complete(NextOutcome::Values(7)))
        ));
    }

    #[test]
    fn stream_next_end_resumes_with_stream_end_and_end_continuation() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::End,
        });

        let mut context = RequestExecutionContext::create(next_ports(executor));
        context.install_root(NextUnit::stream_next());
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(
            outcome,
            Ok(BytecodeSchedulerOutcome::Complete(NextOutcome::End))
        ));
    }

    #[test]
    fn stream_next_error_resumes_with_existing_failure_path() {
        let executor = Arc::new(NextExecutor {
            mode: NextExecutorMode::Error,
        });

        let mut context = RequestExecutionContext::create(next_ports(executor));
        context.install_root(NextUnit::stream_next());
        let (outcome, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);

        assert!(matches!(
            outcome,
            Ok(BytecodeSchedulerOutcome::Complete(NextOutcome::Failure(
                "stream failed"
            )))
        ));
    }
}
