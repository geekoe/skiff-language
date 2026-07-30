use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory,
    program_execution::ProgramExecutionContext,
    spawn_ops::{execute_runtime_assembly_spawn_target, resolve_runtime_assembly_spawn_target},
    Interpreter, RuntimeAssemblyEvalTarget, TestEffectCaseContext,
};
use skiff_runtime_linked_program::ExecutableAddr;

use crate::{BoundaryResponse, ExecutionBudget, ExecutionControl, RequestError, RequestResult};

#[derive(Debug, Clone)]
pub struct RuntimeSpawnRequest {
    pub request_id: String,
    pub target: String,
    pub payload: Vec<u8>,
    pub test_effects_enabled: bool,
    pub test_case_capability: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAssemblySpawnTarget {
    eval: RuntimeAssemblyEvalTarget,
    addr: ExecutableAddr,
    target: String,
}

impl RuntimeAssemblySpawnTarget {
    pub fn new(eval: RuntimeAssemblyEvalTarget, target: impl Into<String>) -> RequestResult<Self> {
        let target = target.into();
        let addr = resolve_runtime_assembly_spawn_target(&eval, &target)?;
        Ok(Self { eval, addr, target })
    }

    pub fn eval(&self) -> &RuntimeAssemblyEvalTarget {
        &self.eval
    }

    pub fn addr(&self) -> &ExecutableAddr {
        &self.addr
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

pub struct RuntimeSpawnExecutionInput {
    pub target: RuntimeAssemblySpawnTarget,
    pub request: RuntimeSpawnRequest,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RuntimeSpawnExecutionHandles,
    pub test_effect_execution: Option<RuntimeSpawnTestEffectExecution>,
}

pub struct RuntimeSpawnExecutionHandles {
    pub request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeSpawnEvalAdapter>,
}

pub trait RuntimeSpawnEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn begin_test_effect_execution(&self)
        -> RequestResult<Option<RuntimeSpawnTestEffectExecution>>;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeSpawnEvalExecutionInputParts<'a>,
        interpreter: &'a Interpreter,
        target: &'a RuntimeAssemblySpawnTarget,
    ) -> ProgramExecutionContext<'a>;
}

trait RuntimeSpawnTestEffectLease: Send + Sync {}

impl<T> RuntimeSpawnTestEffectLease for T where T: Send + Sync {}

/// Keeps one test-case derived-request lease alive for the complete execution. The concrete
/// registry and lease type remain Host-private.
#[doc(hidden)]
pub struct RuntimeSpawnTestEffectExecution {
    context: TestEffectCaseContext,
    _lease: Box<dyn RuntimeSpawnTestEffectLease>,
}

impl RuntimeSpawnTestEffectExecution {
    #[doc(hidden)]
    pub fn new(context: TestEffectCaseContext, lease: impl Send + Sync + 'static) -> Self {
        Self {
            context,
            _lease: Box::new(lease),
        }
    }
}

pub struct RuntimeSpawnEvalExecutionInputParts<'a> {
    pub request: &'a RuntimeSpawnRequest,
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
}

pub async fn execute_runtime_spawn_request(
    input: RuntimeSpawnExecutionInput,
) -> RequestResult<BoundaryResponse> {
    let RuntimeSpawnExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
        test_effect_execution,
    } = input;
    let lifecycle = RuntimeSpawnRequestLifecycle::new(target, Arc::clone(&cancelled));
    let target = lifecycle.target();
    if request.target != target.target() {
        return Err(RequestError::protocol(
            request.target,
            "spawn request target does not match the exact resolved target",
        ));
    }
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    if request.test_effects_enabled != request.test_case_capability.is_some() {
        return Err(RequestError::Unsupported(
            "spawn request testEffectsEnabled and testCaseCapability authority disagree"
                .to_string(),
        ));
    }
    if request.test_effects_enabled && test_effect_execution.is_none() {
        return Err(RequestError::Unsupported(
            "spawn request testCaseCapability was not admitted by the Host registry".to_string(),
        ));
    }
    if !request.test_effects_enabled && test_effect_execution.is_some() {
        return Err(RequestError::Unsupported(
            "spawn request cannot borrow test effects without testCaseCapability".to_string(),
        ));
    }
    let interpreter = match test_effect_execution.as_ref() {
        Some(execution) => Interpreter::for_runtime_assembly_with_test_effect_case_context(
            execution.context.clone(),
            handles.eval_adapter.runtime_factory(),
        ),
        None => Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory()),
    };
    let context = handles.eval_adapter.execution_context(
        RuntimeSpawnEvalExecutionInputParts {
            request: &request,
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        &interpreter,
        target,
    );
    execute_runtime_assembly_spawn_target(
        &interpreter,
        context,
        target.eval(),
        target.addr(),
        &request.payload,
    )
    .await?;
    Ok(BoundaryResponse::payload(Vec::new()))
}

struct RuntimeSpawnRequestLifecycle {
    target: RuntimeAssemblySpawnTarget,
    cancelled: Arc<AtomicBool>,
}

impl RuntimeSpawnRequestLifecycle {
    fn new(target: RuntimeAssemblySpawnTarget, cancelled: Arc<AtomicBool>) -> Self {
        Self { target, cancelled }
    }

    fn target(&self) -> &RuntimeAssemblySpawnTarget {
        &self.target
    }
}

impl Drop for RuntimeSpawnRequestLifecycle {
    fn drop(&mut self) {
        let request_activation = self.target.eval().request_activation();
        if self.cancelled.load(Ordering::Acquire) {
            request_activation.cancel();
        }
        request_activation.end_request();
    }
}
