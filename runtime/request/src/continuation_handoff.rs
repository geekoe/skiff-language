use skiff_runtime_scheduler::{
    BytecodeScheduler, BytecodeSchedulerError, BytecodeSchedulerPorts, BytecodeUnit, PendingWake,
    RootEscrow, SettlementSource, SuspendedTrampoline,
};

/// Request-owned handoff for the winner of one pending settlement race.
///
/// The type and every field stay private to this module. It is constructed
/// only by consuming a claimed [`PendingWake`], and intentionally implements
/// neither `Clone` nor `Copy`: the exact continuation, suspended chain,
/// escrowed roots, and outcome have one route into the scheduler.
#[must_use = "a claimed pending winner must be routed exactly once"]
struct PendingWinnerHandoff<U: BytecodeUnit> {
    source: SettlementSource,
    resume: U::ResumeToken,
    suspended: SuspendedTrampoline<U, U::ResumeToken>,
    roots: RootEscrow,
    outcome: U::ResumeOutcome,
}

impl<U: BytecodeUnit> PendingWinnerHandoff<U> {
    fn from_pending_wake(
        wake: PendingWake<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, U::ResumeOutcome>,
    ) -> Self {
        let (owner, settlement) = wake.into_parts();
        let source = settlement.source();
        let outcome = settlement.into_outcome();
        let (resume, suspended, roots) = owner.into_parts();
        Self {
            source,
            resume,
            suspended,
            roots,
            outcome,
        }
    }

    fn into_scheduler(
        self,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError>
    where
        U: 'static,
    {
        let Self {
            source,
            resume,
            suspended,
            roots,
            outcome,
        } = self;
        route_pending_winner(source, suspended, resume, outcome, roots, ports)
    }
}

/// The only request entry for a claimed pending winner.
pub(crate) fn resume_pending_wake<U>(
    wake: PendingWake<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, U::ResumeOutcome>,
    ports: BytecodeSchedulerPorts<U>,
) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError>
where
    U: BytecodeUnit + 'static,
{
    PendingWinnerHandoff::<U>::from_pending_wake(wake).into_scheduler(ports)
}

#[allow(clippy::match_same_arms)]
fn route_pending_winner<U>(
    source: SettlementSource,
    suspended: SuspendedTrampoline<U, U::ResumeToken>,
    resume: U::ResumeToken,
    outcome: U::ResumeOutcome,
    roots: RootEscrow,
    ports: BytecodeSchedulerPorts<U>,
) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError>
where
    U: BytecodeUnit + 'static,
{
    // Keep the exact winner source until this request-owned closed route. The
    // dormant stage intentionally gives every route the existing scheduler
    // behavior; none of these arms promotes or materializes an exception.
    match source {
        SettlementSource::HostCompletion => {
            BytecodeScheduler::resume_from_suspended(suspended, resume, outcome, roots, ports)
        }
        SettlementSource::Cancellation => {
            BytecodeScheduler::resume_from_suspended(suspended, resume, outcome, roots, ports)
        }
        SettlementSource::Deadline => {
            BytecodeScheduler::resume_from_suspended(suspended, resume, outcome, roots, ports)
        }
        SettlementSource::InternalStop => {
            BytecodeScheduler::resume_from_suspended(suspended, resume, outcome, roots, ports)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU32,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    use skiff_runtime_model::{
        vm_heap::{VmHeap, VmHeapError},
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };
    use skiff_runtime_scheduler::{
        BytecodeControl, BytecodeSchedulerOutcome, FlatTrampoline, PendingWake, RootDisposition,
        RootEscrowBacking,
    };
    use skiff_runtime_vm::{VmBudget, VmBudgetError, VmSemanticCharge};

    use super::*;

    #[derive(Debug)]
    struct MoveOnlyResume {
        id: usize,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for MoveOnlyResume {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    struct MoveOnlyOutcome {
        id: usize,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for MoveOnlyOutcome {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct TestUnit {
        suspended_id: usize,
        resumed: Option<(MoveOnlyResume, MoveOnlyOutcome)>,
    }

    impl VmRootSource for TestUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for TestUnit {
        type ResumeToken = MoveOnlyResume;
        type ResumeOutcome = MoveOnlyOutcome;
        type RootResult = (usize, usize, usize);
        type ChildInvocation = ();
        type AdapterInvocation = ();
        type StreamItem = ();
        type PendingOperation = ();

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeControl<
            Self::RootResult,
            Self::ChildInvocation,
            Self::AdapterInvocation,
            Self::StreamItem,
            Self::PendingOperation,
        > {
            let (resume, outcome) = self
                .resumed
                .take()
                .expect("routed scheduler must inject the pending winner");
            BytecodeControl::Complete((self.suspended_id, resume.id, outcome.id))
        }

        fn resume(
            &mut self,
            token: Self::ResumeToken,
            outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(_completed: Self::RootResult) -> Self::ResumeOutcome {
            unreachable!("the test fixture has no child")
        }
    }

    #[derive(Default)]
    struct RootCounts {
        restored: AtomicUsize,
        discarded: AtomicUsize,
    }

    struct RecordingRoots(Arc<RootCounts>);

    impl VmRootSource for RecordingRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for RecordingRoots {
        fn root_count(&self) -> usize {
            1
        }

        fn restore_roots(self: Box<Self>) {
            self.0.restored.fetch_add(1, Ordering::SeqCst);
        }

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {
            self.0.discarded.fetch_add(1, Ordering::SeqCst);
        }
    }

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

    struct RouteFixture {
        suspended_id: usize,
        resume_id: usize,
        outcome_id: usize,
        resume_drops: Arc<AtomicUsize>,
        outcome_drops: Arc<AtomicUsize>,
        roots: Arc<RootCounts>,
    }

    impl RouteFixture {
        fn new(seed: usize) -> Self {
            Self {
                suspended_id: seed,
                resume_id: seed + 1,
                outcome_id: seed + 2,
                resume_drops: Arc::new(AtomicUsize::new(0)),
                outcome_drops: Arc::new(AtomicUsize::new(0)),
                roots: Arc::new(RootCounts::default()),
            }
        }

        fn route(
            &self,
            source: SettlementSource,
        ) -> Result<BytecodeScheduler<TestUnit>, BytecodeSchedulerError> {
            let suspended = FlatTrampoline::new(TestUnit {
                suspended_id: self.suspended_id,
                resumed: None,
            })
            .suspend();
            route_pending_winner(
                source,
                suspended,
                MoveOnlyResume {
                    id: self.resume_id,
                    drops: Arc::clone(&self.resume_drops),
                },
                MoveOnlyOutcome {
                    id: self.outcome_id,
                    drops: Arc::clone(&self.outcome_drops),
                },
                RootEscrow::new(Box::new(RecordingRoots(Arc::clone(&self.roots)))),
                BytecodeSchedulerPorts::default(),
            )
        }

        fn assert_owned_by_scheduler(&self, scheduler: &BytecodeScheduler<TestUnit>) {
            assert_eq!(scheduler.active().suspended_id, self.suspended_id);
            let (resume, outcome) = scheduler
                .active()
                .resumed
                .as_ref()
                .expect("route must inject exact continuation parts");
            assert_eq!(resume.id, self.resume_id);
            assert_eq!(outcome.id, self.outcome_id);
            assert_eq!(self.resume_drops.load(Ordering::SeqCst), 0);
            assert_eq!(self.outcome_drops.load(Ordering::SeqCst), 0);
            assert_eq!(self.roots.restored.load(Ordering::SeqCst), 1);
            assert_eq!(self.roots.discarded.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn all_winner_sources_reach_the_closed_route_with_exact_owned_parts() {
        let sources = [
            SettlementSource::HostCompletion,
            SettlementSource::Cancellation,
            SettlementSource::Deadline,
            SettlementSource::InternalStop,
        ];

        for (index, source) in sources.into_iter().enumerate() {
            let fixture = RouteFixture::new(index * 10);
            let scheduler = fixture.route(source).expect("closed route must resume");
            fixture.assert_owned_by_scheduler(&scheduler);
            drop(scheduler);
            assert_eq!(fixture.resume_drops.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.outcome_drops.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.roots.restored.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.roots.discarded.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn routed_scheduler_outcome_matches_the_existing_resume_behavior() {
        let fixture = RouteFixture::new(40);
        let scheduler = fixture
            .route(SettlementSource::HostCompletion)
            .expect("host winner must resume");
        fixture.assert_owned_by_scheduler(&scheduler);

        let outcome = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap();
        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete((40, 41, 42))
        ));
        assert_eq!(fixture.resume_drops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.outcome_drops.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.roots.restored.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.roots.discarded.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn claimed_pending_wake_is_the_only_handoff_entry_type() {
        type TestWake = PendingWake<
            MoveOnlyResume,
            SuspendedTrampoline<TestUnit, MoveOnlyResume>,
            MoveOnlyOutcome,
        >;
        let entry: fn(
            TestWake,
            BytecodeSchedulerPorts<TestUnit>,
        ) -> Result<BytecodeScheduler<TestUnit>, BytecodeSchedulerError> =
            resume_pending_wake::<TestUnit>;
        let constructor: fn(TestWake) -> PendingWinnerHandoff<TestUnit> =
            PendingWinnerHandoff::<TestUnit>::from_pending_wake;

        let _ = (entry, constructor);
    }

    #[test]
    fn handoff_has_no_external_construction_clone_or_serde_surface() {
        let production = include_str!("continuation_handoff.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production module prefix");
        let crate_root = include_str!("lib.rs");

        assert!(crate_root.contains("mod continuation_handoff;"));
        assert!(!crate_root.contains("pub mod continuation_handoff;"));
        assert!(!crate_root.contains("pub use continuation_handoff"));
        assert!(production.contains("struct PendingWinnerHandoff"));
        assert!(!production.contains("pub struct PendingWinnerHandoff"));
        assert!(!production.contains("derive(Clone"));
        assert!(!production.contains("impl Clone for PendingWinnerHandoff"));
        assert!(!production.contains("impl Copy for PendingWinnerHandoff"));
        assert!(!production.contains("serde"));
    }
}
