use std::sync::{atomic::AtomicBool, Arc};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, program_execution::ProgramExecutionContext,
    EvalRequestExecutionInput, Interpreter,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;

use crate::{
    ExecutionBudget, ExecutionControl, RequestEnvelope, RequestPayloadContext,
    RuntimeAssemblyRequestTarget, RuntimeOperation,
};

pub trait RequestEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_input<'a>(
        &'a self,
        parts: RequestEvalExecutionInputParts<'a>,
        request_context: RequestPayloadContext<'a>,
    ) -> EvalRequestExecutionInput<'a>;
}

pub trait AssemblyRequestEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_context<'a>(
        &'a self,
        parts: RequestEvalExecutionInputParts<'a>,
        request_context: RequestPayloadContext<'a>,
        interpreter: &Interpreter,
        target: &RuntimeAssemblyRequestTarget,
    ) -> ProgramExecutionContext<'a>;
}

pub struct RequestEvalExecutionInputParts<'a> {
    pub operation: &'a RuntimeOperation,
    pub request: &'a RequestEnvelope,
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}
