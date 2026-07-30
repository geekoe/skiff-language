use std::{sync::Arc, time::Instant};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, error::RuntimeError,
    program_execution::ProgramExecutionContext, Interpreter, RuntimeAssemblyEvalTarget,
    RuntimeWebSocketJsonRpcCallable, RuntimeWebSocketJsonRpcExecutionOutcome,
    RuntimeWebSocketJsonRpcExecutionTarget, RuntimeWebSocketJsonRpcExecutionTerminal,
    RuntimeWebSocketJsonRpcRequest,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;

use crate::{ExecutionBudget, ExecutionControl, RuntimeAssemblyWebSocketJsonRpcTarget};

pub struct RuntimeWebSocketJsonRpcExecutionInput {
    pub target: RuntimeAssemblyWebSocketJsonRpcTarget,
    pub params: Vec<u8>,
    pub connection_id: String,
    pub business_identity: Option<String>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub test_effects_enabled: bool,
    pub handles: RuntimeWebSocketJsonRpcExecutionHandles,
}

pub struct RuntimeWebSocketJsonRpcExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeWebSocketJsonRpcEvalAdapter>,
}

/// Host capability-context builder for an already-pinned JSON-RPC execution.
///
/// No transport header, peer request id, Router correlation, or trace id is exposed as an
/// adapter source.
pub trait RuntimeWebSocketJsonRpcEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketJsonRpcEvalExecutionInputParts<'a>,
        interpreter: &'a Interpreter,
        eval_target: &'a RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a>;
}

pub struct RuntimeWebSocketJsonRpcEvalExecutionInputParts<'a> {
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}

/// Execute one pinned WebSocket JSON-RPC method without performing Host or wire work.
pub async fn execute_runtime_websocket_jsonrpc(
    input: RuntimeWebSocketJsonRpcExecutionInput,
) -> RuntimeWebSocketJsonRpcExecutionTerminal {
    let RuntimeWebSocketJsonRpcExecutionInput {
        target,
        params,
        connection_id,
        business_identity,
        cancellation,
        execution_budget,
        test_effects_enabled,
        handles,
    } = input;
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    if cancellation.is_cancelled() {
        execution_budget.record_cancelled();
        return RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
    }
    if execution_budget
        .deadline()
        .is_some_and(|deadline| Instant::now() >= deadline)
    {
        cancellation.cancel();
        execution_budget.record_deadline_exceeded();
        return RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded,
        );
    }

    let interpreter = if test_effects_enabled {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            Default::default(),
            handles.eval_adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory())
    };
    let context = handles.eval_adapter.execution_context(
        RuntimeWebSocketJsonRpcEvalExecutionInputParts {
            execution,
            cancellation: cancellation.clone(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        &interpreter,
        target.eval(),
    );
    let view = RuntimeWebSocketJsonRpcPinnedTargetView(&target);
    let terminal = interpreter
        .execute_runtime_websocket_jsonrpc(
            context,
            RuntimeWebSocketJsonRpcRequest {
                params: &params,
                connection_id: &connection_id,
                business_identity: business_identity.as_deref(),
            },
            &view,
            cancellation.clone(),
            execution_budget.deadline(),
        )
        .await;
    let finalization = interpreter.finalize_test_case();
    let terminal = finalize_execution_terminal(
        terminal,
        finalization,
        &cancellation,
        execution_budget.deadline(),
    );
    record_terminal_budget(&terminal, &execution_budget);
    terminal
}

struct RuntimeWebSocketJsonRpcPinnedTargetView<'a>(&'a RuntimeAssemblyWebSocketJsonRpcTarget);

impl RuntimeWebSocketJsonRpcExecutionTarget for RuntimeWebSocketJsonRpcPinnedTargetView<'_> {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        self.0.eval()
    }

    fn gateway_entry_key(&self) -> &skiff_artifact_model::GatewayEntryKey {
        self.0.gateway_entry_key()
    }

    fn protocol_surface(&self) -> &skiff_artifact_model::GatewayEntryProtocolSurface {
        self.0.protocol_surface()
    }

    fn adapter_plan(&self) -> &skiff_artifact_model::GatewayAdapterPlan {
        self.0.adapter_plan()
    }

    fn handler(&self) -> RuntimeWebSocketJsonRpcCallable<'_> {
        RuntimeWebSocketJsonRpcCallable {
            callable_id: self.0.handler_callable_id(),
            signature: self.0.handler_signature(),
            addr: self.0.handler_addr(),
        }
    }
}

fn finalize_execution_terminal(
    terminal: RuntimeWebSocketJsonRpcExecutionTerminal,
    finalization: Result<(), RuntimeError>,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> RuntimeWebSocketJsonRpcExecutionTerminal {
    if matches!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
    ) {
        return terminal;
    }
    if matches!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded
        )
    ) {
        return terminal;
    }
    if cancellation.is_cancelled() {
        return RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        cancellation.cancel();
        return RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded,
        );
    }
    if finalization.is_err() {
        return RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::InternalError,
        );
    }
    terminal
}

fn record_terminal_budget(
    terminal: &RuntimeWebSocketJsonRpcExecutionTerminal,
    execution_budget: &ExecutionBudget,
) {
    match terminal {
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled => execution_budget.record_cancelled(),
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded,
        ) => execution_budget.record_deadline_exceeded(),
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::Success { .. }
            | RuntimeWebSocketJsonRpcExecutionOutcome::InvalidParams
            | RuntimeWebSocketJsonRpcExecutionOutcome::InternalError,
        ) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_jsonrpc_execution_keeps_cancelled_outside_response_outcome() {
        let terminal = RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
        assert!(matches!(
            terminal,
            RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
        ));
        let failure = RuntimeWebSocketJsonRpcExecutionOutcome::InternalError;
        assert_eq!(failure.payload(), None);
    }

    #[test]
    fn websocket_jsonrpc_execution_finalization_prefers_cancel_then_deadline() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let terminal = finalize_execution_terminal(
            RuntimeWebSocketJsonRpcExecutionTerminal::Response(
                RuntimeWebSocketJsonRpcExecutionOutcome::Success {
                    payload: b"\"late\"".to_vec(),
                },
            ),
            Err(RuntimeError::Decode(
                "private-finalization-message".to_string(),
            )),
            &cancellation,
            Some(Instant::now()),
        );
        assert_eq!(
            terminal,
            RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
        );
        assert!(!format!("{terminal:?}").contains("private-finalization-message"));
    }
}
