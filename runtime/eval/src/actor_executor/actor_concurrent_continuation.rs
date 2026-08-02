use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Poll,
};

use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec,
    plan::BoundaryUse,
    request_heap::{deep_clone_runtime_values_between_heaps, RequestHeap},
    runtime_value::{ActorRef, HeapNode, InterfaceCarrier, RuntimeValue},
};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypeNode, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};

use super::store_error;
use crate::{
    actor_instance::{
        ActorExecutionToken, ActorExecutorAuthority, ActorFieldValue, ActorInstanceExecutionLease,
        ActorInstanceExecutionSnapshot, ActorInstanceHandle, ActorInstanceStore,
        ActorInstanceStoreError,
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
            }),
        }
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

    /// The logical Actor reference of the currently executing instance. `self`
    /// in actor methods is not materialized as an ordinary value, so spawned
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

    /// Materializes exactly the persistent Actor field roots into a fresh heap.
    ///
    /// The evaluator heap also owns invocation arguments, local bindings, and
    /// every temporary allocated while the method runs. None of those values
    /// belong to the Actor instance unless an assigned field reaches them.
    /// Keeping the whole evaluator heap would retain dead invocation state
    /// across methods and make every suspension clone it again.
    fn snapshot_persistent_fields(
        &self,
        heap: &RequestHeap,
    ) -> Result<ActorInstanceExecutionSnapshot, RuntimeError> {
        let (token, field_state) = self.current_access()?;
        token.ensure_active().map_err(store_error)?;
        let mut fields = field_state
            .lock()
            .expect("actor execution fields lock poisoned")
            .clone();
        for field in fields.iter_mut().filter(|field| !field.assigned) {
            // Partial `create` frames may suspend before every field is
            // assigned. Keep those roots canonical and heap-independent.
            field.value = RuntimeValue::Null;
        }
        let assigned_indices = fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| field.assigned.then_some(index))
            .collect::<Vec<_>>();
        let roots = assigned_indices
            .iter()
            .map(|index| {
                let field = &fields[*index];
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
                if type_plan_contains_stream(plan) {
                    return Err(RuntimeError::Decode(format!(
                        "Actor persistent field {} cannot contain a request-scoped Stream",
                        field.name
                    )));
                }
                Ok(field.value.clone())
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        reject_request_scoped_actor_field_values(&roots, heap)?;
        let mut compact_heap = RequestHeap::new(heap.limits().clone());
        let compact_roots =
            deep_clone_runtime_values_between_heaps(heap, &mut compact_heap, &roots)?;
        for (index, value) in assigned_indices.into_iter().zip(compact_roots) {
            fields[index].value = value;
        }
        Ok(ActorInstanceExecutionSnapshot::new(fields, compact_heap))
    }

    /// Commits the current synchronous segment. A child remains open but no
    /// longer owns the instance scheduler while its async operation is pending.
    pub(crate) fn suspend(&self, heap: &RequestHeap) -> Result<(), RuntimeError> {
        let snapshot = self.snapshot_persistent_fields(heap)?;
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
            .commit_execution(&self.suspension.shared.handle, lease, snapshot)
            .map_err(store_error);
        result
    }

    /// Re-enters the exact incarnation scheduler, then imports all Actor field
    /// roots as one graph into this continuation's existing local heap.
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
        execution
            .poll_execution_budget()
            .map_err(RuntimeError::from)?;
        let cancel = execution.cancellation_token();
        let authority = ActorExecutorAuthority::new();
        let mut acquire: Pin<
            Box<
                dyn Future<Output = Result<ActorInstanceExecutionLease, ActorInstanceStoreError>>
                    + Send,
            >,
        > = if self.suspension.shared.activation {
            Box::pin(
                self.suspension
                    .shared
                    .store
                    .acquire_execution_for_activation(&authority, &self.suspension.shared.handle),
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
            let assigned_indices = fields
                .iter()
                .enumerate()
                .filter_map(|(index, field)| field.assigned.then_some(index))
                .collect::<Vec<_>>();
            let roots = assigned_indices
                .iter()
                .map(|index| fields[*index].value.clone())
                .collect::<Vec<_>>();
            let imported = deep_clone_runtime_values_between_heaps(source_heap, heap, &roots)?;
            for (index, value) in assigned_indices.into_iter().zip(imported) {
                fields[index].value = value;
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
        Ok(())
    }

    /// Polls once before introducing a cut point. Buffered operations stay in
    /// the current segment; only an observed `Pending` commits it, releases the
    /// shared arena guard, and reacquires after wake (Exclusive no-ops).
    pub(crate) async fn await_if_pending<F>(
        &self,
        access: &mut HeapAccess<'_>,
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
        self.suspend(access.heap_mut())?;
        access.release();
        let output = future.await;
        access.reacquire().await;
        self.resume(access.heap_mut(), execution).await?;
        Ok(output)
    }

    pub(crate) fn finish(&self, heap: RequestHeap) -> Result<(), RuntimeError> {
        self.finish_borrowed(&heap)
    }

    pub(crate) fn finish_borrowed(&self, heap: &RequestHeap) -> Result<(), RuntimeError> {
        let snapshot = match self.snapshot_persistent_fields(heap) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // A terminal method cannot leave an exclusive scheduler lease
                // installed after its persistent snapshot has failed.
                self.abandon_lease();
                return Err(error);
            }
        };
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
            .commit_execution(&self.suspension.shared.handle, lease, snapshot)
            .map_err(store_error);
        result
    }

    fn abandon_lease(&self) {
        if let Some(lease) = self
            .suspension
            .lease
            .lock()
            .expect("actor suspension lease lock poisoned")
            .take()
        {
            drop(lease);
        }
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

fn reject_request_scoped_actor_field_values(
    roots: &[RuntimeValue],
    heap: &RequestHeap,
) -> Result<(), RuntimeError> {
    let mut visited = HashSet::new();
    let mut pending = roots.iter().map(|root| (root, 0_usize)).collect::<Vec<_>>();
    while let Some((value, depth)) = pending.pop() {
        if depth > heap.limits().max_clone_depth {
            return Err(RuntimeError::ResourceLimitExceeded {
                resource: "requestHeap".to_string(),
                reason: "max persistent Actor graph depth".to_string(),
                limit: heap.limits().max_clone_depth,
                current: depth,
                requested_delta: depth.saturating_sub(heap.limits().max_clone_depth),
            });
        }
        let RuntimeValue::Heap(handle) = value else {
            continue;
        };
        if !visited.insert(*handle) {
            continue;
        }
        match heap.get(*handle)? {
            HeapNode::Bytes(_) => {}
            HeapNode::Array(items) => {
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            HeapNode::Object(object) => {
                pending.extend(object.fields().values().map(|item| (item, depth + 1)));
            }
            HeapNode::Map(map) => {
                pending.extend(map.values().map(|item| (item, depth + 1)));
            }
            HeapNode::Interface(interface) => match interface.carrier() {
                InterfaceCarrier::Local { payload, .. } => pending.push((payload, depth + 1)),
                InterfaceCarrier::CallbackCapability(_) => {
                    return Err(RuntimeError::Decode(
                        "request-scoped callback capability cannot enter persistent Actor state"
                            .to_string(),
                    ));
                }
            },
            HeapNode::Exception(_) => {
                return Err(RuntimeError::Decode(
                    "request-local exception cannot enter persistent Actor state".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// The continuation fence deliberately contains only instance identity.
/// Fields, token, scheduler guard, and local heap ownership never cross frames.
struct ActorContinuationFence {
    instance_identity: usize,
}

pub(super) struct ActorSuspensionState {
    shared: Arc<ActorContinuationShared>,
    pub(super) lease: Mutex<Option<ActorInstanceExecutionLease>>,
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
