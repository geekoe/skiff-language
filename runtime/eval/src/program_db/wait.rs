use std::future::Future;

use crate::{
    error::Result,
    heap_access::{await_shared_with_release, HeapAccess},
    program_execution::ProgramExecutionContext,
};

/// Waits for one caller-heap-free database operation without duplicating the
/// Actor continuation's first-poll state machine.
pub(super) async fn await_operation<F>(
    context: &ProgramExecutionContext<'_>,
    heap: &mut HeapAccess<'_>,
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
        None if heap.is_shared() => Ok(await_shared_with_release(heap, operation).await),
        None => Ok(operation.await),
    }
}
