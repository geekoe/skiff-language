use std::{future::Future, time::Instant};

use skiff_runtime_capability_context::{ExecutionControl, ExecutionScope, OwnedExecutionControl};

use crate::error::{Result, RuntimeError, ScopeTerminalCarrier};

pub(super) fn from_execution(execution: &ExecutionControl<'_>) -> Result<ExecutionScope> {
    execution.execution_scope().map_err(scope_access_error)
}

pub(super) fn from_owned_execution(execution: &OwnedExecutionControl) -> Result<ExecutionScope> {
    from_execution(&execution.borrow())
}

pub(super) async fn wait<F, T>(scope: ExecutionScope, future: F) -> Result<T>
where
    F: Future<Output = T>,
{
    if let Some(terminal) = scope.terminal_at(Instant::now()) {
        return Err(ScopeTerminalCarrier::runtime_error(terminal));
    }

    let cancellation = scope.cancellation_signals();
    let deadline = scope.effective_deadline().map(|deadline| deadline.at());
    let deadline_wait = async move {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    let mut future = Box::pin(future);
    tokio::pin!(deadline_wait);
    tokio::select! {
        biased;
        _ = cancellation.wait_cancelled() => Err(current_terminal(&scope)),
        _ = &mut deadline_wait => Err(current_terminal(&scope)),
        output = &mut future => Ok(output),
    }
}

fn current_terminal(scope: &ExecutionScope) -> RuntimeError {
    scope
        .terminal_at(Instant::now())
        .map(ScopeTerminalCarrier::runtime_error)
        .unwrap_or_else(|| {
            RuntimeError::InvalidArtifact(
                "provider stream wait woke without an execution scope terminal".to_string(),
            )
        })
}

fn scope_access_error(
    error: skiff_runtime_capability_context::ExecutionScopeAccessError,
) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!("current execution scope is unavailable: {error}"))
}
