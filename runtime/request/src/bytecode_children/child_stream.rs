//! K6 stream-child lifecycle: a bounded ordered buffer plus a flat child
//! supervisor installed for the exact producer owner.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{
    BoundaryDropPlan, BoundaryTransfer, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef, LiteralIr,
    TypeRefIr, ValueProvenance,
};
use skiff_runtime_linked_bytecode::{LinkedServiceBoundaryValue, TypeIndex};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    error::RuntimeErrorPayload,
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeHandoff, BytecodeParkFailure, BytecodeParkRequest, BytecodePortFailure,
    BytecodeSchedulerError, BytecodeStreamHandoff, BytecodeStreamSupervisor, ChildFinish,
    ChildFinishError, ChildHeapCarrier, RequestByteStreamPullFuture,
    RequestByteStreamPullStartError, RequestByteStreamSource, RequestResourceHandle,
    RequestResourceTable, RequestResourceTermination,
};
use skiff_runtime_vm::{
    ResumeOutcome, StreamItem, VmBudget, VmCompletion, VmError, VmFiber, VmOwnedValues,
    VmResumeToken,
};

use crate::bytecode_server_stream::validate_stream_producer_authority;

pub const CHILD_STREAM_CAPACITY: usize = 64;

pub fn provider_stream_item(
    image: &DeploymentExecutionImage,
    function: skiff_runtime_linked_bytecode::FunctionIndex,
) -> Option<(TypeIndex, LinkedServiceBoundaryValue)> {
    let stream_type = image
        .functions()
        .get(usize::try_from(function.get()).ok()?)
        .filter(|row| row.index() == function)
        .and_then(|function| function.stream_result_type_ref())?;
    let TypeRefIr::Builtin { name, args } = image
        .types()
        .get(usize::try_from(stream_type.get()).ok()?)
        .filter(|row| row.index() == stream_type)
        .map(|row| row.type_ref())?
    else {
        return None;
    };
    if name != "Stream" || args.len() != 1 {
        return None;
    }
    let item_ref = args.first()?;
    let item_type = image
        .types()
        .iter()
        .find(|entry| entry.type_ref() == item_ref)?
        .index();
    let plan = LinkedServiceBoundaryValue::new(
        ContractTypeRef::Builtin {
            name: "Stream".to_string(),
            arguments: Vec::new(),
        },
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        },
        BoundaryTransfer::Move,
        BoundaryDropPlan::SnapshotRelease,
        ValueProvenance::Fresh,
        item_type,
        item_ref.clone(),
    );
    Some((item_type, plan))
}

pub struct ChildStreamCore {
    buffer: VecDeque<ValueSlot>,
    blocked: Option<ValueSlot>,
    terminal: Option<ChildStreamTerminal>,
    capacity: usize,
}

enum ChildStreamTerminal {
    End,
    Error(String),
}

impl ChildStreamCore {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            blocked: None,
            terminal: None,
            capacity,
        }
    }

    pub fn emit(&mut self, item: ValueSlot) -> Result<bool, String> {
        if self.terminal.is_some() {
            return Err("child stream is already terminal".to_string());
        }
        if self.blocked.is_some() {
            return Err("child stream already has a backpressured item".to_string());
        }
        if self.buffer.len() < self.capacity {
            self.buffer.push_back(item);
            Ok(true)
        } else {
            self.blocked = Some(item);
            Ok(false)
        }
    }

    pub fn take(&mut self) -> Option<ValueSlot> {
        if let Some(item) = self.buffer.pop_front() {
            return Some(item);
        }
        self.blocked.take()
    }

    pub fn finish_end(&mut self) {
        if self.terminal.is_none() {
            self.terminal = Some(ChildStreamTerminal::End);
        }
    }

    pub fn finish_error(&mut self, message: String) {
        if self.terminal.is_none() {
            self.terminal = Some(ChildStreamTerminal::Error(message));
        }
    }

    pub fn terminal_end(&self) -> bool {
        matches!(self.terminal, Some(ChildStreamTerminal::End))
    }

    pub fn terminal_error(&self) -> Option<&str> {
        match &self.terminal {
            Some(ChildStreamTerminal::Error(message)) => Some(message),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.terminal.is_none()
    }

    pub fn has_items(&self) -> bool {
        !self.buffer.is_empty() || self.blocked.is_some()
    }
}

impl VmRootSource for ChildStreamCore {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for item in self.buffer.iter().chain(self.blocked.iter()) {
            visitor.visit_root(item)?;
        }
        Ok(())
    }
}

pub struct ChildStreamState {
    pub item_type: TypeIndex,
    pub item_type_ref: TypeRefIr,
    pub item_plan: LinkedServiceBoundaryValue,
    pub image: Arc<DeploymentExecutionImage>,
    pub core: Arc<Mutex<ChildStreamCore>>,
    pub relay: Mutex<Option<skiff_runtime_model::vm_value::VmHandle>>,
}

impl VmRootSource for ChildStreamState {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .visit_roots(visitor)
    }
}

pub struct ChildStreamSupervisor {
    core: Arc<Mutex<ChildStreamCore>>,
    item_type: TypeIndex,
    item_plan: LinkedServiceBoundaryValue,
    image: Arc<DeploymentExecutionImage>,
}

impl ChildStreamSupervisor {
    pub fn new(
        core: Arc<Mutex<ChildStreamCore>>,
        item_type: TypeIndex,
        item_plan: LinkedServiceBoundaryValue,
        image: Arc<DeploymentExecutionImage>,
    ) -> Self {
        Self {
            core,
            item_type,
            item_plan,
            image,
        }
    }

    pub fn core(&self) -> Arc<Mutex<ChildStreamCore>> {
        Arc::clone(&self.core)
    }

    fn materialize_item(
        &self,
        item: &StreamItem,
        producer_heap: &mut dyn VmHeap,
        consumer_heap: &mut dyn VmHeap,
    ) -> Result<ValueSlot, String> {
        let [source] = item.item().values() else {
            return Err("child stream EmitStream must carry exactly one item".to_string());
        };
        if source.kind() == Some(ValueKind::ConstRef) {
            let handle = source
                .as_const_ref()
                .ok_or_else(|| "child stream ConstRef has no exact route".to_string())?;
            let index = skiff_runtime_linked_bytecode::FrozenConstantNodeIndex::new(
                u32::try_from(handle.get())
                    .map_err(|_| "child stream ConstRef index overflows".to_string())?,
            );
            let node = item
                .resume()
                .image()
                .frozen_constant_nodes()
                .get(index.get() as usize)
                .filter(|node| node.index() == index)
                .ok_or_else(|| "child stream ConstRef node is absent".to_string())?;
            let tag =
                CompactTypeTag::try_from_type_index(self.item_type.get()).ok_or_else(|| {
                    "child stream item type cannot be represented by a compact tag".to_string()
                })?;
            return match node.value() {
                skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(
                    LiteralIr::String { value },
                ) => consumer_heap
                    .alloc_typed_string(value.clone(), tag, ValueFlags::new(0))
                    .map_err(|error| error.to_string()),
                skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(
                    LiteralIr::Number { value },
                ) => value
                    .as_f64()
                    .map(ValueSlot::number)
                    .ok_or_else(|| "child stream ConstRef number is not finite".to_string()),
                _ => {
                    Err("child stream ConstRef item is not a supported scalar literal".to_string())
                }
            };
        }
        skiff_runtime_boundary::vm_materialize::materialize_linked_value(
            producer_heap,
            source,
            consumer_heap,
            &self.image,
            self.item_type,
            &self.item_plan,
        )
        .map_err(|error| format!("child stream item materialization failed: {error}"))
    }
}

impl BytecodeStreamSupervisor<VmFiber> for ChildStreamSupervisor {
    fn emit_stream_handoff(
        &self,
        item: StreamItem,
        depth: usize,
        producer_heap: &mut dyn VmHeap,
        consumer_heap: Option<&mut dyn VmHeap>,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodePortFailure<StreamItem, VmResumeToken>>
    {
        let const_item = item
            .item()
            .values()
            .first()
            .is_some_and(|value| value.kind() == Some(ValueKind::ConstRef));
        if let Err(error) = validate_stream_producer_authority(
            item.resume().image(),
            item.resume().function(),
            item.item_type(),
            depth,
        ) {
            return Err(release_item_failure(item, producer_heap, error));
        }
        let Some(consumer_heap) = consumer_heap else {
            return Err(release_item_failure(
                item,
                producer_heap,
                BytecodeSchedulerError::Port(
                    "child stream emission requires an exact consumer heap".to_string(),
                ),
            ));
        };
        let caller_item = match self.materialize_item(&item, producer_heap, consumer_heap) {
            Ok(item) => item,
            Err(error) => return Err(release_item_failure(item, producer_heap, error.into())),
        };
        let accepted = match self
            .core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .emit(caller_item)
        {
            Ok(accepted) => accepted,
            Err(error) => {
                let _ = consumer_heap.release_snapshot(&caller_item);
                return Err(release_item_failure(item, producer_heap, error.into()));
            }
        };
        let resume = if const_item {
            let (_, resume) = item.into_parts();
            resume
        } else {
            match item.release(producer_heap) {
                Ok(resume) => resume,
                Err(failure) => return Err(BytecodePortFailure::terminal_stream_release(failure)),
            }
        };
        if !accepted {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Port(
                    "child stream bounded buffer is full; producer must backpressure".to_string(),
                ),
                resume,
            ));
        }
        Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
            resume,
            outcome: ResumeOutcome::Empty,
        }))
    }

    fn park(
        &self,
        _request: BytecodeParkRequest<VmFiber>,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<VmFiber>> {
        Err(BytecodeParkFailure::unaccepted(
            BytecodeSchedulerError::UnsupportedPark,
            _request,
        ))
    }

    fn finish_stream(
        &self,
        _depth: usize,
        result: &VmCompletion,
    ) -> Result<(), BytecodeSchedulerError> {
        let mut core = self
            .core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if result.thrown_diagnostic().is_some() || result.failure().is_some() {
            core.finish_error("child stream producer failed".to_string());
        } else if result
            .returned_values()
            .is_some_and(|values| values.is_empty())
        {
            core.finish_end();
        }
        Ok(())
    }
}

fn release_item_failure(
    item: StreamItem,
    heap: &mut dyn VmHeap,
    reason: BytecodeSchedulerError,
) -> BytecodePortFailure<StreamItem, VmResumeToken> {
    match item.release(heap) {
        Ok(resume) => BytecodePortFailure::continuation(reason, resume),
        Err(failure) => BytecodePortFailure::terminal_stream_release(failure),
    }
}

pub struct ChildStreamFinish {
    core: Arc<Mutex<ChildStreamCore>>,
    supervisor: Arc<ChildStreamSupervisor>,
    item_type: TypeIndex,
    item_type_ref: TypeRefIr,
    item_plan: LinkedServiceBoundaryValue,
    image: Arc<DeploymentExecutionImage>,
    child_streams: Arc<Mutex<HashMap<RequestResourceHandle, ChildStreamState>>>,
    resources: RequestResourceTable,
    relay: Mutex<Option<skiff_runtime_model::vm_value::VmHandle>>,
}

impl ChildStreamFinish {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        core: Arc<Mutex<ChildStreamCore>>,
        supervisor: Arc<ChildStreamSupervisor>,
        item_type: TypeIndex,
        item_type_ref: TypeRefIr,
        item_plan: LinkedServiceBoundaryValue,
        image: Arc<DeploymentExecutionImage>,
        child_streams: Arc<Mutex<HashMap<RequestResourceHandle, ChildStreamState>>>,
        resources: RequestResourceTable,
    ) -> Self {
        Self {
            core,
            supervisor,
            item_type,
            item_type_ref,
            item_plan,
            image,
            child_streams,
            resources,
            relay: Mutex::new(None),
        }
    }

    fn finish_with_result(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        if child_result.thrown_diagnostic().is_some() {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "child stream producer threw before returning an endpoint".to_string(),
            )));
        }
        let (outcome, mut residual) = child_result.into_resume().map_err(|_| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "child stream producer terminal cannot materialize to the caller".to_string(),
            ))
        })?;
        match outcome {
            ResumeOutcome::Values(values) if values.values().len() == 1 => {
                let value = values.values()[0];
                if value.kind() == Some(ValueKind::Null) {
                    // Direct stream producers return `null` after emitting.
                } else if value.kind() == Some(ValueKind::ResourceRef) {
                    let handle = value
                        .as_resource_ref()
                        .expect("ResourceRef route checked above");
                    *self
                        .relay
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
                } else {
                    return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                        "child stream producer returned a non-stream value".to_string(),
                    )));
                }
            }
            ResumeOutcome::Values(values) if values.is_empty() => {
                if !self
                    .core
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .has_items()
                {
                    self.relay_missing_endpoint()?;
                }
            }
            ResumeOutcome::Empty => {
                if !self
                    .core
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .has_items()
                {
                    self.relay_missing_endpoint()?;
                }
            }
            ResumeOutcome::Failure(error) => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)))
            }
            _ => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "child stream producer returned an unsupported outcome".to_string(),
                )));
            }
        }
        residual
            .release_all(child_heap.heap_mut())
            .map_err(|error| ChildFinishError::failure(BytecodeSchedulerError::Vm(error)))?;
        self.supervisor
            .core()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish_end();

        let handle = self
            .resources
            .register_byte_stream(Box::new(ChildStreamResourceSource))
            .map_err(|error| {
                ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                    "child stream resource registration failed: {error}"
                )))
            })?;
        let relay_route = self
            .relay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.child_streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                handle,
                ChildStreamState {
                    item_type: self.item_type,
                    item_type_ref: self.item_type_ref.clone(),
                    item_plan: self.item_plan.clone(),
                    image: Arc::clone(&self.image),
                    core: self.supervisor.core(),
                    relay: Mutex::new(relay_route),
                },
            );
        if resume.expected_result_count() == 0 {
            return Ok(ResumeOutcome::Empty);
        }
        let stream_type = resume_result_type(resume).map_err(ChildFinishError::failure)?;
        let tag = CompactTypeTag::try_from_type_index(stream_type.get()).ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "child stream endpoint type cannot be represented by a VM tag".to_string(),
            ))
        })?;
        let endpoint = parent_heap
            .admit_resource_ref(handle.vm_handle(), tag, ValueFlags::new(0))
            .map_err(|error| {
                ChildFinishError::failure(BytecodeSchedulerError::Vm(VmError::Heap(error)))
            })?;
        VmOwnedValues::try_from_resume(resume, Box::new([endpoint]))
            .map(ResumeOutcome::Values)
            .map_err(|rejected| {
                let _ = parent_heap.release_snapshot(&endpoint);
                ChildFinishError::failure(BytecodeSchedulerError::Vm(rejected.error().clone()))
            })
    }

    fn relay_missing_endpoint(&self) -> Result<(), ChildFinishError<VmFiber>> {
        let streams = self
            .child_streams
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut candidates = streams
            .iter()
            .filter(|(_, state)| {
                state.item_type_ref == self.item_type_ref
                    && state
                        .relay
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_none()
            })
            .map(|(handle, _)| *handle);
        let Some(handle) = candidates.next() else {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "child stream producer returned empty but opened no relay stream".to_string(),
            )));
        };
        if candidates.next().is_some() {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "child stream producer returned empty with multiple relay candidates".to_string(),
            )));
        }
        *self
            .relay
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle.vm_handle());
        Ok(())
    }
}

impl ChildFinish<VmFiber, VmResumeToken> for ChildStreamFinish {
    fn finish(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        self.finish_with_result(resume, child_result, child_heap, parent_heap)
    }
}

pub fn child_stream_next(
    state: &ChildStreamState,
    resume: VmResumeToken,
    item_type: TypeIndex,
    _parent_heap: &mut dyn VmHeap,
) -> Result<(ResumeOutcome, VmResumeToken), (BytecodeSchedulerError, VmResumeToken)> {
    let expected_type_ref = resume
        .image()
        .types()
        .get(usize::try_from(item_type.get()).unwrap_or(usize::MAX))
        .filter(|row| row.index() == item_type)
        .map(|row| row.type_ref());
    let Some(expected_type_ref) = expected_type_ref else {
        return Err((
            BytecodeSchedulerError::Port(
                "child stream consumer item type is absent from the caller image".to_string(),
            ),
            resume,
        ));
    };
    if state.item_type_ref != *expected_type_ref {
        return Err((
            BytecodeSchedulerError::Port(
                "child stream item type drifts from the StreamNext resume site".to_string(),
            ),
            resume,
        ));
    }
    if let Some(_relay) = *state
        .relay
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        // The relay state is stored by the same finish path under its exact
        // resource handle. It is looked up by the ingress after this function
        // returns, so this method intentionally does not traverse it directly.
        return Err((
            BytecodeSchedulerError::Port(
                "child stream relay requires exact ingress handle delegation".to_string(),
            ),
            resume,
        ));
    }
    let mut core = state
        .core
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(item) = core.take() {
        return match VmOwnedValues::try_from_resume(&resume, Box::new([item])) {
            Ok(values) => Ok((ResumeOutcome::Values(values), resume)),
            Err(rejected) => Err((BytecodeSchedulerError::Vm(rejected.error().clone()), resume)),
        };
    }
    if core.terminal_end() {
        return Ok((ResumeOutcome::StreamEnd, resume));
    }
    if let Some(message) = core.terminal_error() {
        return Ok((
            ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "StreamError".to_string(),
                message: message.to_string(),
                status: None,
                details: None,
            })),
            resume,
        ));
    }
    Err((
        BytecodeSchedulerError::Port("child stream has no item or terminal".to_string()),
        resume,
    ))
}

fn resume_result_type(resume: &VmResumeToken) -> Result<TypeIndex, BytecodeSchedulerError> {
    let site = resume
        .image()
        .resume_sites()
        .get(resume.resume_site())
        .filter(|site| site.function() == resume.function() && site.site() == resume.instruction())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "child stream resume token has no matching linked resume site".to_string(),
            )
        })?;
    site.result_types().get(0).copied().ok_or_else(|| {
        BytecodeSchedulerError::Port(
            "child stream resume token has no stream endpoint result".to_string(),
        )
    })
}

struct ChildStreamResourceSource;

impl RequestByteStreamSource for ChildStreamResourceSource {
    fn start_pull(&self) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        Err(RequestByteStreamPullStartError::WrongResourceKind)
    }

    fn terminate(self: Box<Self>, _termination: RequestResourceTermination) {}
}

impl VmRootSource for ChildStreamResourceSource {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_stream_core_orders_buffered_and_backpressured_items_then_ends() {
        let mut core = ChildStreamCore::new(2);
        assert!(core.emit(ValueSlot::number(1.0)).unwrap());
        assert!(core.emit(ValueSlot::number(2.0)).unwrap());
        assert!(!core.emit(ValueSlot::number(3.0)).unwrap());

        let item = core.take().expect("first item");
        assert_eq!(item.as_number(), Some(1.0));
        let item = core.take().expect("second item");
        assert_eq!(item.as_number(), Some(2.0));
        let item = core.take().expect("third item");
        assert_eq!(item.as_number(), Some(3.0));
        assert!(core.take().is_none());

        core.finish_end();
        assert!(core.terminal_end());
        assert!(!core.is_open());
        assert!(core.emit(ValueSlot::number(4.0)).is_err());
    }

    #[test]
    fn child_stream_core_terminal_error_is_sticky_and_fail_closed() {
        let mut core = ChildStreamCore::new(1);
        core.finish_error("producer failed".to_string());
        assert_eq!(core.terminal_error(), Some("producer failed"));
        assert!(!core.terminal_end());
        assert!(core.emit(ValueSlot::number(1.0)).is_err());
        core.finish_end();
        assert!(!core.terminal_end());
        assert_eq!(core.terminal_error(), Some("producer failed"));
    }
}
