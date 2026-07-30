use std::future::Future;

use skiff_runtime_model::request_heap::RequestHeap;

use crate::{error::Result, program_execution::ProgramExecutionContext};

/// Waits for one caller-heap-free database operation without duplicating the
/// Actor continuation's first-poll state machine.
pub(super) async fn await_operation<F>(
    context: &ProgramExecutionContext<'_>,
    heap: &mut RequestHeap,
    operation: F,
) -> Result<F::Output>
where
    F: Future + Send + 'static,
{
    match context.actor_execution_frame() {
        Some(frame) => {
            frame
                .await_if_pending(heap, &context.execution(), operation)
                .await
        }
        None => Ok(operation.await),
    }
}
