use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};

use super::store_error;
use crate::{
    actor_instance::{
        ActorExecutionToken, ActorExecutorAuthority, ActorFieldValue, ActorInstanceExecutionLease,
        ActorInstanceHandle, ActorInstanceStore, ActorInstanceStoreError,
    },
    error::RuntimeError,
};

mod bridge;

use bridge::{ActorChildContinuationState, ActorConcurrentContinuationGate};
pub(crate) use bridge::{ActorConcurrentContinuationBridge, ActorConcurrentContinuationLane};

#[derive(Clone)]
pub(crate) struct ActorExecutionFrame {
    pub(super) suspension: Arc<ActorSuspensionState>,
}

struct ActorContinuationShared {
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
    fence: ActorContinuationFence,
    field_plans: Vec<(String, RuntimeTypePlan)>,
    /// Activation frames execute `create` before the instance is admitted and
    /// must re-acquire the scheduler through the activation path on resume.
    activation: bool,
}

impl ActorExecutionFrame {
    pub(crate) fn new(
        store: ActorInstanceStore,
        handle: ActorInstanceHandle,
        lease: ActorInstanceExecutionLease,
        field_plans: Vec<(String, RuntimeTypePlan)>,
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
                    field_plans,
                    activation,
                }),
                lease: Mutex::new(Some(lease)),
                outer_gate: Mutex::new(None),
                child: None,
            }),
        }
    }

    fn suspended_child(
        parent: &Self,
        child: Arc<ActorChildContinuationState>,
    ) -> ActorExecutionFrame {
        Self {
            suspension: Arc::new(ActorSuspensionState {
                shared: Arc::clone(&parent.suspension.shared),
                lease: Mutex::new(None),
                outer_gate: Mutex::new(None),
                child: Some(child),
            }),
        }
    }

    pub(crate) fn begin_concurrent(
        &self,
        heap: &RequestHeap,
        lane_count: usize,
    ) -> Result<ActorConcurrentContinuationBridge, RuntimeError> {
        ActorConcurrentContinuationBridge::begin(self, heap, lane_count)
    }

    fn current_access(
        &self,
    ) -> Result<(Arc<ActorExecutionToken>, Arc<Mutex<Vec<ActorFieldValue>>>), RuntimeError> {
        let lease = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned");
        let lease = lease.as_ref().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "Actor self field access attempted while its continuation is suspended".to_string(),
            )
        })?;
        Ok((lease.token(), lease.fields()))
    }

    pub(crate) fn read_field(&self, field: &str) -> Result<RuntimeValue, RuntimeError> {
        let (token, fields) = self.current_access()?;
        token.ensure_active().map_err(store_error)?;
        let value = fields
            .lock()
            .expect("actor execution fields lock poisoned")
            .iter()
            .find(|candidate| candidate.name == field)
            .map(|candidate| {
                if !candidate.assigned {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "Actor execution field {field} is not assigned yet"
                    )));
                }
                Ok(candidate.value.clone())
            })
            .transpose()?
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "Actor execution field {field} is absent from the instance frame"
                ))
            });
        value
    }

    pub(crate) fn write_field(
        &self,
        field: &str,
        field_type: &LinkedTypeRef,
        program: ProgramTypeView<'_>,
        current_addr: &ExecutableAddr,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<(), RuntimeError> {
        let (token, fields) = self.current_access()?;
        token.ensure_active().map_err(store_error)?;
        let plan = RuntimeTypePlan::from_linked(
            field_type,
            &PlanContext::from_type_view(program, current_addr),
        )?;
        // The boundary round trip keeps persistent Actor state free of
        // unchecked continuation-local heap handles.
        let codec = RuntimeBoundaryCodec::new(
            &plan,
            BoundaryUse::NativeArg,
            format!("Actor self field {field}"),
        );
        let wire = codec.to_wire_json(value, heap)?;
        let checked = codec.from_wire_json(&wire, heap)?;
        let mut fields = fields.lock().expect("actor execution fields lock poisoned");
        let target = fields
            .iter_mut()
            .find(|candidate| candidate.name == field)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "Actor execution field {field} is absent from the instance frame"
                ))
            })?;
        target.value = checked;
        target.assigned = true;
        Ok(())
    }

    /// Commits the current synchronous segment. A child remains open but no
    /// longer owns the instance scheduler while its async operation is pending.
    pub(crate) fn suspend(&self, heap: &RequestHeap) -> Result<(), RuntimeError> {
        let lease = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .take()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "Actor continuation attempted to suspend without an execution token"
                        .to_string(),
                )
            })?;
        let result = self
            .suspension
            .shared
            .store
            .commit_execution(&self.suspension.shared.handle, lease, heap.clone())
            .map_err(store_error);
        if let Some(child) = &self.suspension.child {
            child.segment_released();
        }
        result
    }

    /// Re-enters the exact incarnation scheduler, then imports Actor fields
    /// through their linked codecs into this continuation's local heap.
    pub(crate) async fn resume(
        &self,
        heap: &mut RequestHeap,
        execution: &crate::capabilities::ExecutionControl<'_>,
    ) -> Result<(), RuntimeError> {
        if self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .is_some()
        {
            return Err(resume_with_installed_lease_error());
        }
        self.ensure_outer_children_released()?;
        let mut child_resume = self
            .suspension
            .child
            .as_ref()
            .map(|child| child.begin_resume())
            .transpose()?;
        execution
            .poll_execution_budget()
            .map_err(RuntimeError::from)?;
        let cancel = execution.cancellation_token();
        let authority = ActorExecutorAuthority::new();
        let mut acquire: Pin<
            Box<
                dyn Future<
                        Output = Result<
                            ActorInstanceExecutionLease,
                            ActorInstanceStoreError,
                        >,
                    > + Send,
            >,
        > = if self.suspension.shared.activation {
            Box::pin(
                self.suspension
                    .shared
                    .store
                    .acquire_execution_for_activation(
                        &authority,
                        &self.suspension.shared.handle,
                    ),
            )
        } else {
            Box::pin(
                self.suspension
                    .shared
                    .store
                    .acquire_execution(&authority, &self.suspension.shared.handle),
            )
        };
        let mut budget_tick = tokio::time::interval(std::time::Duration::from_millis(5));
        let mut lease = loop {
            tokio::select! {
                result = &mut acquire => break result.map_err(store_error)?,
                () = cancel.wait_cancelled() => {
                    execution.poll_execution_budget().map_err(RuntimeError::from)?;
                    return Err(RuntimeError::from(
                        skiff_runtime_capability_context::ExecutionControlError::Cancelled,
                    ));
                }
                _ = budget_tick.tick() => {
                    execution.poll_execution_budget().map_err(RuntimeError::from)?;
                }
            }
        };
        if lease.instance_identity() != self.suspension.shared.fence.instance_identity {
            return Err(store_error(ActorInstanceStoreError::InstanceReplaced));
        }
        {
            let fields = lease.fields();
            let source_heap = lease.heap_mut();
            let mut fields = fields.lock().expect("actor execution fields lock poisoned");
            for field in fields.iter_mut() {
                if !field.assigned {
                    // `create` may suspend before assigning every non-key
                    // field; unassigned fields stay continuation-local until
                    // `create` writes them.
                    continue;
                }
                let (_, plan) = self
                    .suspension
                    .shared
                    .field_plans
                    .iter()
                    .find(|(name, _)| name == &field.name)
                    .ok_or_else(|| {
                        RuntimeError::InvalidArtifact(format!(
                            "Actor continuation field {} has no linked type plan",
                            field.name
                        ))
                    })?;
                let codec = RuntimeBoundaryCodec::new(
                    plan,
                    BoundaryUse::NativeArg,
                    format!("Actor resume field {}", field.name),
                );
                let wire = codec.to_wire_json(&field.value, source_heap)?;
                field.value = codec.from_wire_json(&wire, heap)?;
            }
        }
        let mut current = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned");
        if current.is_some() {
            return Err(resume_with_installed_lease_error());
        }
        *current = Some(lease);
        if let Some(child_resume) = child_resume.as_mut() {
            if let Err(error) = child_resume.segment_acquired() {
                current.take();
                return Err(error);
            }
        }
        Ok(())
    }

    /// Polls once before introducing a cut point. Buffered operations stay in
    /// the current segment; only an observed `Pending` commits and releases it.
    pub(crate) async fn await_if_pending<F>(
        &self,
        heap: &mut RequestHeap,
        execution: &crate::capabilities::ExecutionControl<'_>,
        future: F,
    ) -> Result<F::Output, RuntimeError>
    where
        F: Future,
    {
        tokio::pin!(future);
        if let Some(output) = poll_once_without_yield(future.as_mut()).await {
            return Ok(output);
        }
        self.suspend(heap)?;
        let output = future.await;
        self.resume(heap, execution).await?;
        Ok(output)
    }

    pub(crate) fn finish(&self, heap: RequestHeap) -> Result<(), RuntimeError> {
        self.ensure_outer_children_released()?;
        let lease = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .take()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "Actor method completed while its continuation was suspended".to_string(),
                )
            })?;
        let result = self
            .suspension
            .shared
            .store
            .commit_execution(&self.suspension.shared.handle, lease, heap)
            .map_err(store_error);
        if let Some(child) = &self.suspension.child {
            child.finish();
        }
        result
    }

    fn abandon_child(&self) {
        if let Some(lease) = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .take()
        {
            drop(lease);
        }
        if let Some(child) = &self.suspension.child {
            child.finish();
        }
    }

    fn ensure_outer_children_released(&self) -> Result<(), RuntimeError> {
        let gate = self
            .suspension
            .outer_gate
            .lock()
            .expect("actor concurrent continuation gate lock poisoned")
            .clone();
        if let Some(gate) = gate {
            gate.ensure_released()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn has_execution_lease(&self) -> bool {
        self.suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .is_some()
    }

    #[cfg(test)]
    pub(super) fn shares_execution_slot(&self, other: &Self) -> bool {
        std::ptr::eq(&self.suspension.lease, &other.suspension.lease)
    }
}

/// The continuation fence deliberately contains only instance identity.
/// Fields, token, scheduler guard, and local heap ownership never cross frames.
struct ActorContinuationFence {
    instance_identity: usize,
}

pub(super) struct ActorSuspensionState {
    shared: Arc<ActorContinuationShared>,
    pub(super) lease: Mutex<Option<ActorInstanceExecutionLease>>,
    outer_gate: Mutex<Option<Arc<ActorConcurrentContinuationGate>>>,
    child: Option<Arc<ActorChildContinuationState>>,
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

fn resume_with_installed_lease_error() -> RuntimeError {
    continuation_error(
        "Actor continuation attempted to resume while an execution token is already installed",
    )
}
