use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    task::Poll,
};

use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec,
    plan::BoundaryUse,
    request_heap::RequestHeap,
    runtime_value::{ActorRef, RuntimeValue},
};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypeNode, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};

use super::store_error;
use crate::{
    actor_instance::{
        reject_request_scoped_actor_field_values, ActorInstanceHandle, ActorInstanceStore,
        SegmentLease,
    },
    error::RuntimeError,
    heap_access::HeapAccess,
};

#[derive(Clone)]
pub(crate) struct ActorExecutionFrame {
    pub(super) suspension: Arc<ActorSuspensionState>,
}

struct ActorContinuationShared {
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
    fence: ActorContinuationFence,
    /// Activation frames execute `create` before the instance is admitted.
    activation: bool,
}

impl ActorExecutionFrame {
    pub(crate) fn new(
        store: ActorInstanceStore,
        handle: ActorInstanceHandle,
        lease: SegmentLease,
        activation: bool,
    ) -> Self {
        Self {
            suspension: Arc::new(ActorSuspensionState {
                shared: Arc::new(ActorContinuationShared {
                    store,
                    handle,
                    fence: ActorContinuationFence {
                        instance_identity: lease.instance_identity(),
                    },
                    activation,
                }),
                segment: Mutex::new(Some(lease)),
            }),
        }
    }

    fn current_segment(&self) -> Result<MutexGuard<'_, Option<SegmentLease>>, RuntimeError> {
        Ok(self
            .suspension
            .segment
            .lock()
            .expect("actor suspension segment lock poisoned"))
    }

    fn ensure_active(&self) -> Result<MutexGuard<'_, Option<SegmentLease>>, RuntimeError> {
        let segment = self.current_segment()?;
        let lease = segment.as_ref().ok_or_else(|| {
            continuation_error(
                "Actor self field access attempted while its continuation is suspended",
            )
        })?;
        if lease.is_suspended() {
            return Err(continuation_error(
                "Actor self field access attempted while its continuation is suspended",
            ));
        }
        if lease.is_finished() {
            return Err(continuation_error(
                "Actor self field access attempted after its method completed",
            ));
        }
        lease.token().ensure_active().map_err(store_error)?;
        Ok(segment)
    }

    /// Reads the live field root directly from the shared arena state. The
    /// value is the current arena handle/scalar; the evaluator dereferences it
    /// against the segment's `HeapAccess`.
    pub(crate) fn read_field(&self, field: &str) -> Result<RuntimeValue, RuntimeError> {
        let _segment = self.ensure_active()?;
        let root = self
            .suspension
            .shared
            .store
            .field_root(&self.suspension.shared.handle, field)
            .map_err(store_error)?
            .ok_or_else(|| {
                continuation_error(format!(
                    "Actor execution field {field} is absent from the instance frame"
                ))
            })?;
        if !root.assigned {
            return Err(continuation_error(format!(
                "Actor execution field {field} is not assigned yet"
            )));
        }
        Ok(root.value)
    }

    /// The logical Actor reference of the currently executing instance. `self`
    /// in actor methods is not materialized as an ordinary value, so task
    /// self-messages derive the receiver from the active execution frame.
    pub(crate) fn current_actor_ref(&self) -> Result<ActorRef, RuntimeError> {
        let fence = self.suspension.shared.handle.fence();
        let key = &fence.incarnation.logical_key;
        Ok(ActorRef::new(
            key.service_id.clone(),
            key.actor_type_identity.clone(),
            key.actor_id_type_identity.clone(),
            key.actor_id_encoding_version.clone(),
            key.canonical_actor_id_key_bytes.clone(),
            key.actor_id_hash.clone(),
            Some(fence.incarnation.epoch),
        ))
    }

    /// Writes a field root directly into the shared arena.
    ///
    /// Type validation and request-scoped validation run on this write path
    /// (design §3.5); there is no wire roundtrip and no persistent snapshot.
    pub(crate) fn write_field(
        &self,
        field: &str,
        field_type: &LinkedTypeRef,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<(), RuntimeError> {
        let _segment = self.ensure_active()?;
        let plan = RuntimeTypePlan::from_linked(
            field_type,
            &PlanContext::from_type_view(program, current_addr),
        )?;
        if type_plan_contains_stream(&plan) {
            return Err(RuntimeError::Decode(format!(
                "Actor self field {field} cannot contain a request-scoped Stream"
            )));
        }
        let codec = RuntimeBoundaryCodec::new(
            &plan,
            BoundaryUse::NativeArg,
            format!("Actor self field {field}"),
        );
        let checked = codec.coerce_runtime_value(value, heap)?;
        reject_request_scoped_actor_field_values(std::slice::from_ref(&checked), heap)?;
        let present = self
            .suspension
            .shared
            .store
            .set_field_root(&self.suspension.shared.handle, field, checked)
            .map_err(store_error)?;
        if !present {
            return Err(continuation_error(format!(
                "Actor execution field {field} is absent from the instance frame"
            )));
        }
        Ok(())
    }

    /// Moves the segment to the suspended counter. The arena guard itself is
    /// released by the caller through `HeapAccess::release` before the await.
    pub(crate) fn suspend(&self) -> Result<(), RuntimeError> {
        let mut segment = self.current_segment()?;
        let lease = segment.as_mut().ok_or_else(|| {
            continuation_error("Actor continuation attempted to suspend without an execution token")
        })?;
        if lease.is_finished() {
            return Err(continuation_error(
                "Actor continuation attempted to suspend after its method completed",
            ));
        }
        self.suspension
            .shared
            .store
            .suspend_segment(&self.suspension.shared.handle, lease)
            .map_err(store_error)
    }

    /// Revalidates the instance fence and arena epoch after the guard was
    /// reacquired, then moves the segment back to the active counter.
    pub(crate) fn resume(
        &self,
        execution: &crate::capabilities::ExecutionControl<'_>,
    ) -> Result<(), RuntimeError> {
        execution
            .poll_execution_budget()
            .map_err(RuntimeError::from)?;
        execution.check_cancelled().map_err(RuntimeError::from)?;
        let mut segment = self.current_segment()?;
        let lease = segment.as_mut().ok_or_else(|| {
            continuation_error("Actor continuation attempted to resume without a suspended segment")
        })?;
        if !lease.is_suspended() {
            return Err(continuation_error(
                "Actor continuation attempted to resume without a suspended segment",
            ));
        }
        self.suspension
            .shared
            .store
            .resume_segment(&self.suspension.shared.handle, lease)
            .map_err(store_error)
    }

    /// Polls once before introducing a cut point. Buffered operations stay in
    /// the current segment; only an observed `Pending` suspends the segment,
    /// releases the shared arena guard, and reacquires after wake (Exclusive
    /// release/reacquire are no-ops).
    pub(crate) async fn await_if_pending<F>(
        &self,
        access: &mut HeapAccess,
        execution: &crate::capabilities::ExecutionControl<'_>,
        future: F,
    ) -> Result<F::Output, RuntimeError>
    where
        F: Future,
    {
        let mut future = Box::pin(future);
        if let Some(output) = poll_once_without_yield(future.as_mut()).await {
            return Ok(output);
        }
        self.suspend()?;
        access.release();
        let output = future.await;
        access.reacquire().await;
        self.resume(execution)?;
        Ok(output)
    }

    /// Ends the segment. Field state is already live in the shared arena;
    /// finish only revalidates and releases the continuation counters.
    pub(crate) fn finish(&self) -> Result<(), RuntimeError> {
        let mut segment = self.current_segment()?;
        if segment.as_ref().is_some_and(SegmentLease::is_suspended) {
            return Err(continuation_error(
                "Actor method completed while its continuation was suspended",
            ));
        }
        let mut lease = segment.take().ok_or_else(|| {
            continuation_error("Actor method completed while its continuation was suspended")
        })?;
        self.suspension
            .shared
            .store
            .commit_segment(&self.suspension.shared.handle, &mut lease)
            .map_err(store_error)
    }

    #[cfg(test)]
    pub(super) fn has_execution_lease(&self) -> bool {
        self.suspension
            .segment
            .lock()
            .expect("actor suspension segment lock poisoned")
            .is_some()
    }

    #[cfg(test)]
    pub(crate) fn is_suspended(&self) -> bool {
        self.suspension
            .segment
            .lock()
            .expect("actor suspension segment lock poisoned")
            .as_ref()
            .is_some_and(SegmentLease::is_suspended)
    }

    #[cfg(test)]
    pub(super) fn shares_execution_slot(&self, other: &Self) -> bool {
        std::ptr::eq(&self.suspension.segment, &other.suspension.segment)
    }
}

fn type_plan_contains_stream(plan: &RuntimeTypePlan) -> bool {
    match plan.node() {
        RuntimeTypeNode::Alias(target)
        | RuntimeTypeNode::Nullable(target)
        | RuntimeTypeNode::Representation {
            payload: target, ..
        } => type_plan_contains_stream(target),
        RuntimeTypeNode::Union(types) => types.iter().any(type_plan_contains_stream),
        RuntimeTypeNode::Stream(_) => true,
        RuntimeTypeNode::Array(item) => type_plan_contains_stream(item),
        RuntimeTypeNode::Map { key, value } => {
            type_plan_contains_stream(key) || type_plan_contains_stream(value)
        }
        RuntimeTypeNode::Record { fields, .. } => fields
            .iter()
            .any(|field| type_plan_contains_stream(&field.ty)),
        RuntimeTypeNode::LiteralString(_)
        | RuntimeTypeNode::Json
        | RuntimeTypeNode::JsonObject
        | RuntimeTypeNode::Bytes
        | RuntimeTypeNode::Date
        | RuntimeTypeNode::String
        | RuntimeTypeNode::Bool
        | RuntimeTypeNode::Number
        | RuntimeTypeNode::Integer
        | RuntimeTypeNode::Null
        | RuntimeTypeNode::Unknown => false,
    }
}

/// The continuation fence deliberately contains only instance identity. Field
/// roots and the arena epoch live with the instance; the segment lease carries
/// the epoch snapshot used for resume/commit validation.
struct ActorContinuationFence {
    instance_identity: usize,
}

pub(super) struct ActorSuspensionState {
    shared: Arc<ActorContinuationShared>,
    pub(super) segment: Mutex<Option<SegmentLease>>,
}

async fn poll_once_without_yield<F>(mut future: Pin<&mut F>) -> Option<F::Output>
where
    F: Future,
{
    std::future::poll_fn(|context| {
        Poll::Ready(match future.as_mut().poll(context) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

fn continuation_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(message.into())
}
