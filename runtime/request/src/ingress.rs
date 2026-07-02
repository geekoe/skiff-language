use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};

use skiff_runtime_eval::{
    EvalRequestEffectDouble, EvalRequestExecutionInput, EvalRequestExecutor,
    EvalRequestExecutorInput, EvalRequestInvocation, EvalRuntimeProgram,
};
use skiff_runtime_linked_program::ExecutableAddr;

use crate::{
    http_ingress::BinaryHttpIngressHandler, runner::RequestExecutionHandles,
    runtime_ingress::RuntimeIngressHandler, websocket_ingress::WebSocketIngressHandler,
    BoundaryResponse, ExecutionBudget, RequestEffectDouble, RequestEnvelope, RequestError,
    RequestEvalExecutionInputParts, RequestPayloadContext, RequestResult, RequestServiceMetadata,
    RuntimeOperation,
};

pub(crate) struct IngressDispatchInput<'a> {
    pub(crate) operation: &'a RuntimeOperation,
    pub(crate) addr: &'a ExecutableAddr,
    pub(crate) metadata: &'a RequestServiceMetadata,
    pub(crate) request: &'a RequestEnvelope,
    pub(crate) execution: super::ExecutionControl<'a>,
    pub(crate) cancelled: &'a AtomicBool,
    pub(crate) execution_budget: Arc<ExecutionBudget>,
    pub(crate) handles: &'a RequestExecutionHandles,
    pub(crate) eval_program: Arc<EvalRuntimeProgram>,
}

pub(crate) struct IngressDispatcher<'a> {
    input: IngressDispatchInput<'a>,
    executor: EvalRequestExecutor,
}

impl<'a> IngressDispatcher<'a> {
    pub(crate) fn new(input: IngressDispatchInput<'a>) -> Self {
        let executor = eval_executor_for_request(
            input.eval_program.clone(),
            input.request,
            input.handles.eval_adapter.as_ref(),
        );
        Self { input, executor }
    }

    pub(crate) fn validate_request(request: &RequestEnvelope) -> RequestResult<()> {
        if request.mode != "unary" && request.mode != "serverStream" {
            return Err(RequestError::Unsupported(format!(
                "runtime only supports unary or serverStream request.start, got {}",
                request.mode
            )));
        }
        if request.binary_http.is_some() && request.websocket_adapter.is_some() {
            return Err(RequestError::protocol(
                request.target.clone(),
                "request.start cannot include both binary HTTP and websocket adapter metadata",
            ));
        }
        Ok(())
    }

    pub(crate) async fn dispatch(self) -> RequestResult<BoundaryResponse> {
        let context = RequestIngressContext::new(&self.input);
        if self.input.request.binary_http.is_some() {
            return BinaryHttpIngressHandler::new(&context, &self.executor)
                .dispatch()
                .await;
        }
        if self.input.request.websocket_adapter.is_some() {
            return WebSocketIngressHandler::new(&context, &self.executor)
                .dispatch()
                .await;
        }
        if self.input.request.mode == "serverStream" {
            return RuntimeIngressHandler::new(&context, &self.executor)
                .dispatch_server_stream()
                .await;
        }
        if self.input.request.extra.contains_key("actorCall") {
            return Err(RequestError::Unsupported(
                "actor.call request.start metadata is retired".to_string(),
            ));
        }
        RuntimeIngressHandler::new(&context, &self.executor)
            .dispatch_unary()
            .await
    }
}

pub(super) struct RequestIngressContext<'a> {
    pub(super) operation: &'a RuntimeOperation,
    pub(super) addr: &'a ExecutableAddr,
    pub(super) metadata: &'a RequestServiceMetadata,
    pub(super) request: &'a RequestEnvelope,
    pub(super) execution: super::ExecutionControl<'a>,
    pub(super) cancelled: &'a AtomicBool,
    pub(super) execution_budget: Arc<ExecutionBudget>,
    pub(super) handles: &'a RequestExecutionHandles,
    pub(super) eval_program: Arc<EvalRuntimeProgram>,
}

impl<'a> RequestIngressContext<'a> {
    fn new(input: &IngressDispatchInput<'a>) -> Self {
        Self {
            operation: input.operation,
            addr: input.addr,
            metadata: input.metadata,
            request: input.request,
            execution: input.execution,
            cancelled: input.cancelled,
            execution_budget: input.execution_budget.clone(),
            handles: input.handles,
            eval_program: input.eval_program.clone(),
        }
    }

    pub(super) fn build_eval_invocation(&self) -> RequestResult<EvalRequestInvocation<'_>> {
        crate::invocation_builder::build_eval_invocation(
            self.request,
            self.operation.operation.as_str(),
            self.addr,
            self.eval_program.as_ref(),
        )
    }

    pub(super) fn require_router_sender(&self, message: &str) -> RequestResult<()> {
        if self.handles.streaming_available {
            Ok(())
        } else {
            Err(RequestError::protocol(self.request.target.clone(), message))
        }
    }

    pub(super) fn eval_execution_input(
        &self,
        request_context: RequestPayloadContext<'a>,
    ) -> EvalRequestExecutionInput<'a> {
        self.handles.eval_adapter.execution_input(
            RequestEvalExecutionInputParts {
                operation: self.operation,
                request: self.request,
                execution: self.execution,
                cancelled: self.cancelled,
                execution_budget: self.execution_budget.clone(),
                request_heap_limits: self.handles.request_heap_limits.clone(),
            },
            request_context,
        )
    }
}

fn eval_executor_for_request(
    program: Arc<EvalRuntimeProgram>,
    request: &RequestEnvelope,
    adapter: &dyn super::RequestEvalAdapter,
) -> EvalRequestExecutor {
    EvalRequestExecutor::new(EvalRequestExecutorInput {
        program,
        test_effects_enabled: request.test_effects_enabled,
        test_effect_doubles: test_effect_doubles_for_executor(&request.test_effect_doubles),
        runtime_factory: adapter.runtime_factory(),
    })
}

fn test_effect_doubles_for_executor(
    doubles: &HashMap<String, Vec<RequestEffectDouble>>,
) -> HashMap<String, Vec<EvalRequestEffectDouble>> {
    doubles
        .iter()
        .map(|(target, sequence)| {
            (
                target.clone(),
                sequence
                    .iter()
                    .map(|double| EvalRequestEffectDouble {
                        expect_request: double.expect_request.clone(),
                        response: double.response.clone(),
                    })
                    .collect(),
            )
        })
        .collect()
}
