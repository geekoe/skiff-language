use std::{any::Any, future::Future, pin::Pin};

use skiff_runtime_capability_context::{
    DbCapabilityError, DbRuntimeFinalizer, PreparedDbValueRuntimeOperation,
};

use crate::{
    error::{Result, RuntimeError},
    runtime_value_facade::{RequestHeap, RuntimeValue},
};

/// A heap-free external wait prepared during the caller's current synchronous segment.
///
/// The outcome is intentionally opaque. It can only be consumed by the matching
/// [`NativeExternalFinalize`], after the evaluator has observed Ready/Pending and
/// restored any Actor execution segment.
pub type NativeExternalWait<'a> =
    Pin<Box<dyn Future<Output = Result<NativeExternalOutcome>> + Send + 'a>>;

pub struct NativeExternalOutcome {
    value: Box<dyn Any + Send>,
}

type NativeFinalizeFn =
    Box<dyn FnOnce(Box<dyn Any + Send>, &mut RequestHeap) -> Result<RuntimeValue> + Send>;

/// The caller-heap materialization half of a prepared external native operation.
pub struct NativeExternalFinalize {
    finalize: NativeFinalizeFn,
}

impl NativeExternalFinalize {
    pub fn finalize(
        self,
        outcome: NativeExternalOutcome,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let checkpoint = heap.checkpoint();
        match (self.finalize)(outcome.value, heap) {
            Ok(value) => Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }
}

/// A native operation whose external wait no longer borrows the caller heap.
pub struct PreparedExternalNativeOperation<'a> {
    wait: NativeExternalWait<'a>,
    finalize: NativeExternalFinalize,
}

impl<'a> PreparedExternalNativeOperation<'a> {
    pub(super) fn new<Output, Wait, Finalize>(wait: Wait, finalize: Finalize) -> Self
    where
        Output: Send + 'static,
        Wait: Future<Output = Result<Output>> + Send + 'a,
        Finalize: FnOnce(Output, &mut RequestHeap) -> Result<RuntimeValue> + Send + 'static,
    {
        let wait = Box::pin(async move {
            wait.await.map(|value| NativeExternalOutcome {
                value: Box::new(value),
            })
        });
        let finalize = NativeExternalFinalize {
            finalize: Box::new(move |value, heap| {
                let value = value.downcast::<Output>().map_err(|_| {
                    RuntimeError::InvalidArtifact(
                        "prepared native outcome did not match its finalize owner".to_string(),
                    )
                })?;
                finalize(*value, heap)
            }),
        };
        Self { wait, finalize }
    }

    pub fn into_parts(self) -> (NativeExternalWait<'a>, NativeExternalFinalize) {
        (self.wait, self.finalize)
    }
}

/// Preparation never calls a variant `Pending`: only the evaluator's first poll
/// can establish whether an external wait is actually pending.
pub enum PreparedNativeCall<'a> {
    Ready(RuntimeValue),
    ExternalWait(PreparedExternalNativeOperation<'a>),
}

pub fn prepared_native_call_from_db_value_operation(
    operation: PreparedDbValueRuntimeOperation,
) -> PreparedNativeCall<'static> {
    let wait = operation.into_wait();
    PreparedNativeCall::ExternalWait(PreparedExternalNativeOperation::new(
        async move {
            wait.await.map_err(db_capability_error_to_native)
        },
        |finalizer: DbRuntimeFinalizer<RuntimeValue>, heap: &mut RequestHeap| {
            finalizer
                .finalize(heap)
                .map_err(db_capability_error_to_native)
        },
    ))
}

fn db_capability_error_to_native(error: DbCapabilityError) -> RuntimeError {
    RuntimeError::Opaque(Box::new(error))
}

pub(super) async fn run_prepared_native_call(
    prepared: PreparedNativeCall<'_>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    match prepared {
        PreparedNativeCall::Ready(value) => Ok(value),
        PreparedNativeCall::ExternalWait(operation) => {
            let (wait, finalize) = operation.into_parts();
            let outcome = wait.await?;
            finalize.finalize(outcome, heap)
        }
    }
}
