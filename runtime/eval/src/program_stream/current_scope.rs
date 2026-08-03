use std::{future::Future, time::Instant};

use serde_json::Value;
use skiff_runtime_capability_context::{
    ExecutionScope, StreamCancelSignal, StreamPoll, StreamRuntime, StreamRuntimeResult,
};

use crate::{
    error::{Result, RuntimeError, ScopeTerminalCarrier},
    heap_access::{await_with_release, HeapAccess},
    program_execution::{ExecutionCheckpoint, ExecutionCheckpointKind, ProgramExecutionContext},
};

pub(super) async fn next_with_actor(
    context: &ProgramExecutionContext<'_>,
    heap: &mut HeapAccess,
    runtime: &StreamRuntime,
    stream: &Value,
    stream_signals: &[StreamCancelSignal],
    instruction_units: u64,
) -> Result<StreamRuntimeResult<StreamPoll>> {
    checkpoint(context, instruction_units)?;
    let scope = context.execution_scope()?;
    let next = wait(
        scope,
        runtime.next_with_cancellation(stream, stream_signals, std::iter::empty()),
    );
    let output = match context.actor_execution_frame().cloned() {
        Some(frame) => {
            frame
                .await_if_pending(heap, &context.execution(), next)
                .await??
        }
        None => await_with_release(heap, next).await?,
    };
    checkpoint(context, 0)?;
    Ok(output)
}

pub(super) async fn next(
    context: &ProgramExecutionContext<'_>,
    runtime: &StreamRuntime,
    stream: &Value,
    stream_signals: &[StreamCancelSignal],
    instruction_units: u64,
) -> Result<StreamRuntimeResult<StreamPoll>> {
    checkpoint(context, instruction_units)?;
    let scope = context.execution_scope()?;
    let output = wait(
        scope,
        runtime.next_with_cancellation(stream, stream_signals, std::iter::empty()),
    )
    .await?;
    checkpoint(context, 0)?;
    Ok(output)
}

async fn wait<F, T>(scope: ExecutionScope, future: F) -> Result<T>
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
                "current stream wait woke without an execution scope terminal".to_string(),
            )
        })
}

fn checkpoint(context: &ProgramExecutionContext<'_>, units: u64) -> Result<()> {
    context.checkpoint(ExecutionCheckpoint::new(
        ExecutionCheckpointKind::GeneratedChunk,
        units,
    ))
}
