use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::{
    CancellationSource, CancellationToken, ExecutionBudgetFailure, ExecutionBudgetReason,
    ExecutionControl, ExecutionControlApi, ExecutionControlError, ExecutionControlResult,
    ExecutionScope, ExecutionScopeAccessError, ExecutionScopeDeriveError, ExecutionScopeTerminal,
    FileSourceStreamContext, OwnedExecutionControl, OwnedExecutionControlApi, StreamRuntime,
};
use skiff_runtime_linked_program::ServiceMeta;
use skiff_runtime_model::request_heap::RequestHeapLimits;

use super::{
    execution_scope::{
        deadline_after_duration_ms, EvalMonotonicClock, ExecutionCheckpoint,
        ExecutionCheckpointKind, ExecutionClock,
    },
    OwnedProgramExecutionContext, ProgramExecutionContext, ProgramExecutionInput,
};
use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::{HttpRuntimeOptions, TimeCapabilityContext},
    error::{BudgetReason, RuntimeError},
};

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

#[derive(Clone)]
struct ScopeAwareControl {
    scope: Option<ExecutionScope>,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
    fail_derive: bool,
    budget_error: Option<ExecutionControlError>,
    instruction_units: Arc<AtomicU64>,
}

impl ScopeAwareControl {
    fn available(scope: ExecutionScope, cancellation: CancellationToken) -> Self {
        Self {
            scope: Some(scope),
            cancelled: cancellation.cancel_flag(),
            cancellation,
            fail_derive: false,
            budget_error: None,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    fn unavailable() -> Self {
        let cancellation = CancellationToken::new();
        Self {
            scope: None,
            cancelled: cancellation.cancel_flag(),
            cancellation,
            fail_derive: false,
            budget_error: None,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    fn with_derive_failure(mut self) -> Self {
        self.fail_derive = true;
        self
    }

    fn with_budget_error(mut self, error: ExecutionControlError) -> Self {
        self.budget_error = Some(error);
        self
    }
}

impl ExecutionControlApi for ScopeAwareControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .as_ref()
            .and_then(ExecutionScope::effective_deadline)
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        self.scope
            .clone()
            .ok_or(ExecutionScopeAccessError::Unavailable)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        if self.fail_derive {
            return Err(ExecutionScopeAccessError::Derive(ExecutionScopeDeriveError));
        }
        let scope = ExecutionControlApi::execution_scope(self)?.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(Self {
            scope: Some(scope),
            cancellation: self.cancellation.clone(),
            cancelled: Arc::clone(&self.cancelled),
            fail_derive: false,
            budget_error: self.budget_error,
            instruction_units: Arc::clone(&self.instruction_units),
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::Relaxed);
        Ok(())
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.budget_error.map_or(Ok(()), Err)
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for ScopeAwareControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

fn context(control: ScopeAwareControl) -> ProgramExecutionContext<'static> {
    let execution = ExecutionControl::new(control);
    let runtime_factory = test_runtime::runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let test_effect_doubles =
        runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(stream_runtime.clone()),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            HttpRuntimeOptions::explicit(false),
            stream_runtime,
            test_effect_doubles.clone(),
        ),
        test_effect_doubles,
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: "skiff.run/eval-scope-test".to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: "1.0.0".to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: Default::default(),
        }),
        actor: actor.clone(),
        spawn: actor,
        outbound: test_runtime::outbound_context(),
        request_heap_limits: RequestHeapLimits::default(),
    })
}

fn root_scope(deadline: Option<Instant>) -> (CancellationSource, ExecutionScope) {
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), deadline);
    (cancellation, scope)
}

#[derive(Clone)]
struct ScriptedClock {
    values: Arc<Mutex<VecDeque<Instant>>>,
    last: Instant,
    calls: Arc<AtomicU64>,
}

impl ScriptedClock {
    fn new(values: Vec<Instant>, calls: Arc<AtomicU64>) -> Self {
        let last = *values.last().expect("scripted clock needs one value");
        Self {
            values: Arc::new(Mutex::new(values.into())),
            last,
            calls,
        }
    }
}

impl EvalMonotonicClock for ScriptedClock {
    fn now(&self) -> Instant {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.values
            .lock()
            .expect("clock mutex poisoned")
            .pop_front()
            .unwrap_or(self.last)
    }
}

#[test]
fn program_execution_scope_duration_uses_exact_or_safely_clamped_deadline() {
    let now = Instant::now();
    assert_eq!(
        deadline_after_duration_ms(now, 25),
        now.checked_add(Duration::from_millis(25))
            .expect("ordinary duration should be representable")
    );

    let clamped = deadline_after_duration_ms(now, u64::MAX);
    assert!(clamped >= now);
    if now.checked_add(Duration::from_millis(u64::MAX)).is_none() {
        assert!(clamped.checked_add(Duration::from_millis(1)).is_none());
    }
}

#[test]
fn program_execution_scope_child_capture_and_owned_round_trip_preserve_current_scope() {
    let (cancellation, scope) = root_scope(None);
    let parent = context(ScopeAwareControl::available(scope, cancellation.token()));
    assert_eq!(parent.execution_scope().expect("parent scope").nesting(), 0);

    let child = parent
        .derive_timeout_child(1_000, site())
        .expect("child scope should derive");
    assert_eq!(child.execution_scope().expect("child scope").nesting(), 1);
    assert_eq!(parent.execution_scope().expect("parent scope").nesting(), 0);

    let owned = OwnedProgramExecutionContext::capture(&child);
    let borrowed = owned.borrow();
    assert_eq!(
        borrowed
            .execution_scope()
            .expect("round-trip child scope")
            .nesting(),
        1
    );
    drop(child);
    assert_eq!(
        parent.execution_scope().expect("restored parent").nesting(),
        0
    );
}

#[test]
fn program_execution_scope_owned_round_trip_preserves_current_scripted_clock_sequence() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(3), site())
        .expect("local scope");
    let calls = Arc::new(AtomicU64::new(0));
    let context = context(ScopeAwareControl::available(scope, cancellation.token()))
        .with_execution_clock(ExecutionClock::new(ScriptedClock::new(
            vec![base, base + Duration::from_millis(3)],
            Arc::clone(&calls),
        )));
    let checkpoint = ExecutionCheckpoint::new(ExecutionCheckpointKind::GeneratedChunk, 1);

    context
        .checkpoint(checkpoint)
        .expect("first scripted checkpoint remains before the deadline");
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let owned = OwnedProgramExecutionContext::capture(&context);
    let borrowed = owned.borrow();
    let error = borrowed
        .checkpoint(checkpoint)
        .expect_err("owned round-trip must continue the same scripted clock");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn program_execution_scope_unavailable_and_derive_failure_fail_closed() {
    let unavailable = context(ScopeAwareControl::unavailable());
    assert!(matches!(
        unavailable.execution_scope(),
        Err(RuntimeError::InvalidArtifact(message))
            if message.contains("current execution scope is unavailable")
    ));

    let (cancellation, scope) = root_scope(None);
    let derive_failure =
        context(ScopeAwareControl::available(scope, cancellation.token()).with_derive_failure());
    assert!(matches!(
        derive_failure.derive_timeout_child(1, site()),
        Err(RuntimeError::InvalidArtifact(message))
            if message.contains("execution scope nesting exceeds u32")
    ));
}

#[test]
fn program_execution_scope_nested_deadlines_keep_precise_owner() {
    let base = Instant::now();
    let (_, root) = root_scope(None);

    let outer = root
        .derive(base + Duration::from_millis(20), site())
        .expect("outer scope");
    let inner_earlier = outer
        .derive(base + Duration::from_millis(10), site())
        .expect("inner scope");
    assert!(matches!(
        inner_earlier.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));

    let outer_earlier = root
        .derive(base + Duration::from_millis(10), site())
        .expect("outer scope");
    let inner_later = outer_earlier
        .derive(base + Duration::from_millis(20), site())
        .expect("inner scope");
    assert!(matches!(
        inner_later.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));

    let equal_inner = outer_earlier
        .derive(base + Duration::from_millis(10), site())
        .expect("equal inner scope");
    assert!(matches!(
        equal_inner.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
}

#[test]
fn program_execution_scope_checkpoint_kinds_account_explicit_units() {
    let (cancellation, scope) = root_scope(None);
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let units = Arc::clone(&control.instruction_units);
    let context = context(control);
    let kinds = [
        ExecutionCheckpointKind::FunctionEntry,
        ExecutionCheckpointKind::LoopCondition,
        ExecutionCheckpointKind::LoopBackedge,
        ExecutionCheckpointKind::LaneStart,
        ExecutionCheckpointKind::LaneEnd,
        ExecutionCheckpointKind::TailStart,
        ExecutionCheckpointKind::GeneratedChunk,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let checkpoint = ExecutionCheckpoint::new(kind, index as u64 + 1);
        assert_eq!(checkpoint.kind(), kind);
        context.checkpoint(checkpoint).expect("checkpoint");
    }
    assert_eq!(units.load(Ordering::Relaxed), 28);
}

#[test]
fn program_execution_scope_scripted_clock_crosses_on_bounded_checkpoint() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(3), site())
        .expect("local scope");
    let calls = Arc::new(AtomicU64::new(0));
    let clock = ScriptedClock::new(
        vec![
            base,
            base + Duration::from_millis(2),
            base + Duration::from_millis(3),
        ],
        Arc::clone(&calls),
    );
    let context = context(ScopeAwareControl::available(
        scope.clone(),
        cancellation.token(),
    ))
    .with_execution_clock(ExecutionClock::new(clock));

    let checkpoint = ExecutionCheckpoint::new(ExecutionCheckpointKind::GeneratedChunk, 1);
    context.checkpoint(checkpoint).expect("first checkpoint");
    context.checkpoint(checkpoint).expect("second checkpoint");
    let error = context
        .checkpoint(checkpoint)
        .expect_err("third checkpoint crosses deadline");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[test]
fn program_execution_scope_checkpoint_normalizes_cancel_and_keeps_instruction_limit() {
    let (cancellation, scope) = root_scope(None);
    let cancelled_context = context(ScopeAwareControl::available(scope, cancellation.token()));
    cancellation.cancel();
    assert!(matches!(
        cancelled_context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::FunctionEntry,
            1,
        )),
        Err(RuntimeError::Cancelled)
    ));

    let (active_cancellation, active_scope) = root_scope(None);
    let limit = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
        reason: ExecutionBudgetReason::InstructionLimitExceeded,
        instruction_count: 11,
        limit: Some(10),
        elapsed_ms: 1.5,
    });
    let limit_context = context(
        ScopeAwareControl::available(active_scope, active_cancellation.token())
            .with_budget_error(limit),
    );
    assert!(matches!(
        limit_context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopBackedge,
            1,
        )),
        Err(RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::InstructionLimitExceeded,
            instruction_count: 11,
            limit: Some(10),
            ..
        })
    ));
}

#[test]
fn program_execution_scope_generic_deadline_race_recovers_current_owner() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(2), site())
        .expect("local scope");
    let deadline_error = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
        reason: ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 2,
        limit: None,
        elapsed_ms: 2.0,
    });
    let calls = Arc::new(AtomicU64::new(0));
    let context = context(
        ScopeAwareControl::available(scope.clone(), cancellation.token())
            .with_budget_error(deadline_error),
    )
    .with_execution_clock(ExecutionClock::new(ScriptedClock::new(
        vec![base, base + Duration::from_millis(2)],
        Arc::clone(&calls),
    )));

    let error = context
        .checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopCondition,
            1,
        ))
        .expect_err("generic deadline must recover scope owner");
    let carrier = error.scope_terminal().expect("internal scope terminal");
    assert!(matches!(
        carrier.terminal(),
        ExecutionScopeTerminal::LocalDeadlineExceeded(_)
    ));
    assert!(carrier.is_owned_by(&scope));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}
