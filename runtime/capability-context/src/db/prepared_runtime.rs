use std::{future::Future, pin::Pin};

use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::DbCapabilityResult;

/// An owned, one-shot wait for a prepared DB runtime operation.
///
/// The `'static` bound prevents the wait from retaining the caller's request
/// heap or evaluator state.
pub type DbPreparedRuntimeWait<T> =
    Pin<Box<dyn Future<Output = DbCapabilityResult<DbRuntimeFinalizer<T>>> + Send + 'static>>;

/// A prepared DB operation whose provider work can wait without borrowing the
/// caller's request heap.
#[must_use = "a prepared DB runtime operation must be consumed exactly once"]
pub struct PreparedDbRuntimeOperation<T> {
    wait: DbPreparedRuntimeWait<T>,
}

impl<T> PreparedDbRuntimeOperation<T>
where
    T: Send + 'static,
{
    pub fn new<F>(wait: F) -> Self
    where
        F: Future<Output = DbCapabilityResult<DbRuntimeFinalizer<T>>> + Send + 'static,
    {
        Self {
            wait: Box::pin(wait),
        }
    }

    pub fn into_wait(self) -> DbPreparedRuntimeWait<T> {
        self.wait
    }
}

type DbRuntimeFinalize<T> =
    Box<dyn FnOnce(&mut RequestHeap) -> DbCapabilityResult<T> + Send + 'static>;

/// The owned completion of a prepared DB wait.
///
/// Finalization is synchronous and one-shot. New allocations are rolled back
/// when materialization fails; a provider finalizer must not mutate
/// pre-existing heap nodes before it can still return an error.
#[must_use = "a DB runtime finalizer must be consumed exactly once"]
pub struct DbRuntimeFinalizer<T> {
    finalize: DbRuntimeFinalize<T>,
}

impl<T> DbRuntimeFinalizer<T> {
    pub fn new<F>(finalize: F) -> Self
    where
        F: FnOnce(&mut RequestHeap) -> DbCapabilityResult<T> + Send + 'static,
    {
        Self {
            finalize: Box::new(finalize),
        }
    }

    pub fn finalize(self, heap: &mut RequestHeap) -> DbCapabilityResult<T> {
        let checkpoint = heap.checkpoint();
        match (self.finalize)(heap) {
            Ok(value) => Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }
}

pub type PreparedDbOptionalRuntimeOperation = PreparedDbRuntimeOperation<Option<RuntimeValue>>;
pub type PreparedDbManyRuntimeOperation = PreparedDbRuntimeOperation<Vec<RuntimeValue>>;
pub type PreparedDbValueRuntimeOperation = PreparedDbRuntimeOperation<RuntimeValue>;
