//! C6 same-Runtime callback child leaf.
//!
//! The leaf consumes exact F6 callback table facts and resolves the opaque
//! callback carrier through the host capability table. Cross-Runtime carrier
//! placement is rejected here before any owner state is touched; Router
//! reverse transport is deliberately not implemented.

use std::sync::Arc;

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    LinkedCallableSignature, LinkedInterfaceTable, LinkedInterfaceTableKind,
    LinkedServiceBoundaryValue, TypeIndex,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver,
    callback_projection::CallbackContractOperationProjection,
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap},
    runtime_value::{CallbackCapabilityCarrier, InterfaceCarrier, RuntimeValue},
    vm_heap::VmHeap,
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeChildHandoff, BytecodeChildStart, BytecodePortFailure, BytecodeSchedulerError,
    ChildFinish, ChildFinishError, RequestResourceTable,
};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, ResumeOutcome, Vm, VmBudget, VmCompletion, VmFiber,
    VmLifecycleSite, VmLimits, VmOwnedException, VmOwnedValues, VmResumeToken,
};
use tokio::sync::Mutex;

use super::{BytecodeChildHeapFactory, BytecodeRequestChildComposition};

/// Request-owned callback child registration. It stays fail-closed until the
/// host installs an exact same-Runtime resolver and the F6 linked callback
/// facts are present in the image.
#[derive(Clone, Default)]
pub struct BytecodeCallbackChildComposition {
    pub runtime_replica_id: String,
    pub resolver: Option<Arc<dyn BytecodeCallbackResolver>>,
}

impl BytecodeCallbackChildComposition {
    pub fn is_available(&self) -> bool {
        self.resolver.is_some() && !self.runtime_replica_id.is_empty()
    }

    pub fn require_resolver(
        &self,
    ) -> Result<&dyn BytecodeCallbackResolver, BytecodeCallbackChildError> {
        if self.runtime_replica_id.is_empty() {
            return Err(BytecodeCallbackChildError::MissingRuntimeIdentity);
        }
        self.resolver
            .as_deref()
            .ok_or(BytecodeCallbackChildError::MissingResolver)
    }
}

/// Exact same-Runtime callback resolver. The host implementation owns the
/// request/stream capability table and the native adapter payload.
pub trait BytecodeCallbackResolver: Send + Sync + 'static {
    fn resolve_callback(
        &self,
        carrier: &CallbackCapabilityCarrier,
        expected_runtime_replica_id: &str,
        table: &LinkedInterfaceTable,
        method_ordinal: u32,
        method_abi_id: &str,
    ) -> Result<Arc<dyn CallbackExecution>, BytecodeCallbackChildError>;
}

/// One resolved callback execution authority.
///
/// The provider entry is the exact F6 image fact. Until F6 links that fact,
/// every production lookup returns [`BytecodeCallbackChildError::MissingFacts`]
/// instead of guessing a function from the legacy executable address.
pub trait CallbackExecution: Send + Sync {
    fn canonical_contract(&self) -> &str;

    fn operation(
        &self,
        slot: u32,
        method_abi_id: &str,
    ) -> Result<&CallbackContractOperationProjection, BytecodeCallbackChildError>;

    fn receiver(&self) -> &RuntimeValue;

    fn owner_heap_arena(&self) -> Arc<Mutex<RequestHeap>>;

    fn provider_entry(&self) -> Result<DeploymentExecutionEntry, BytecodeCallbackChildError>;
}

/// Host-backed projection from a VM local-interface carrier into an opaque
/// same-Runtime callback capability in the provider child heap.
///
/// The projector is the service-boundary materialization half of C6. It must
/// register the exact caller image/function facts with the callback table so
/// later `InvokeCallback` dispatch can resolve a provider entry without
/// guessing from a method table or executable address.
pub trait BytecodeCallbackProjector: Send + Sync + 'static {
    fn project_callback_argument(
        &self,
        source_heap: &mut dyn VmHeap,
        source: &ValueSlot,
        caller_image: &Arc<DeploymentExecutionImage>,
        destination_heap: &mut dyn VmHeap,
        provider_image: &DeploymentExecutionImage,
        provider_type: TypeIndex,
        plan: &LinkedServiceBoundaryValue,
    ) -> Result<ValueSlot, BytecodeCallbackChildError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeCallbackChildError {
    #[error("callback child resolver is not configured")]
    MissingResolver,
    #[error("callback child runtime replica identity is not configured")]
    MissingRuntimeIdentity,
    #[error("callback child target is not a callback interface table")]
    NotCallbackTarget,
    #[error("callback method {method_abi_id} at slot {slot} is unavailable")]
    WrongOperation { slot: u32, method_abi_id: String },
    #[error("callback carrier owner {actual} does not match the resolving runtime {expected}")]
    CrossRuntimeRejected { expected: String, actual: String },
    #[error("callback capability is unavailable")]
    CapabilityUnavailable,
    #[error("callback capability is expired or cancelled")]
    CapabilityExpired,
    #[error("callback carrier contract does not match the resolved execution")]
    WrongContract,
    #[error("callback method signature and provider plan disagree: {message}")]
    SignatureMismatch { message: String },
    #[error("callback execution requires F6/K6/X6 exact facts: {message}")]
    MissingFacts { message: String },
    #[error("callback owner heap is unavailable")]
    OwnerStateUnavailable,
    #[error("callback owner value materialization failed: {message}")]
    Materialization { message: String },
    #[error("callback child heap creation failed: {message}")]
    ChildHeap { message: String },
    #[error("callback invocation source release failed: {message}")]
    Release { message: String },
}

/// Resolves the exact linked callback method and same-Runtime carrier.
///
/// This is the unique admission point for a callback child. It never consults
/// a method table, native address, or router reverse transport.
pub(crate) fn resolve_callback_invocation(
    table: &LinkedInterfaceTable,
    method_ordinal: u32,
    method_abi_id: &str,
    carrier: &CallbackCapabilityCarrier,
    composition: &BytecodeCallbackChildComposition,
) -> Result<Arc<dyn CallbackExecution>, BytecodeCallbackChildError> {
    let LinkedInterfaceTableKind::Callback(requirement) = table.kind() else {
        return Err(BytecodeCallbackChildError::NotCallbackTarget);
    };
    let method = requirement
        .methods()
        .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
        .filter(|method| {
            method.method_slot() == method_ordinal
                && method.method_abi_id().as_str() == method_abi_id
        })
        .ok_or_else(|| BytecodeCallbackChildError::WrongOperation {
            slot: method_ordinal,
            method_abi_id: method_abi_id.to_string(),
        })?;
    if carrier.owner_runtime_replica_id() != composition.runtime_replica_id {
        return Err(BytecodeCallbackChildError::CrossRuntimeRejected {
            expected: composition.runtime_replica_id.clone(),
            actual: carrier.owner_runtime_replica_id().to_string(),
        });
    }
    let resolver = composition.require_resolver()?;
    let execution = resolver.resolve_callback(
        carrier,
        &composition.runtime_replica_id,
        table,
        method_ordinal,
        method_abi_id,
    )?;
    if execution.canonical_contract() != carrier.interface_or_adapter_contract() {
        return Err(BytecodeCallbackChildError::WrongContract);
    }
    let operation = execution.operation(method_ordinal, method_abi_id)?;
    let linked_params = method.signature().parameter_types();
    if operation.parameters().len() + 1 != linked_params.len() {
        return Err(BytecodeCallbackChildError::SignatureMismatch {
            message: format!(
                "provider operation has {} parameters but linked callback signature has {} values including Self",
                operation.parameters().len(),
                linked_params.len()
            ),
        });
    }
    Ok(execution)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_callback_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    callback_composition: &BytecodeCallbackChildComposition,
    request_composition: &BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let image = Arc::clone(invocation.resume().image());
    let (table, method_ordinal) = match invocation.target() {
        ChildTarget::Interface {
            table,
            method_ordinal,
        } => (table, method_ordinal),
        _ => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                invocation,
            ));
        }
    };
    let Some(row) = interface_table_by_index(&image, table) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "callback child target table is absent from the image".to_string(),
            ),
            invocation,
        ));
    };
    let LinkedInterfaceTableKind::Callback(requirement) = row.kind() else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "callback child received a non-callback interface table".to_string(),
            ),
            invocation,
        ));
    };
    let method = match requirement
        .methods()
        .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
        .filter(|method| method.method_slot() == method_ordinal)
    {
        Some(method) => method,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "callback method row is absent from the linked table".to_string(),
                ),
                invocation,
            ));
        }
    };
    let method_abi_id = method.method_abi_id().as_str().to_string();
    let caller_signature = method.signature().clone();

    let argument_values = invocation.arguments().values().to_vec();
    if argument_values.is_empty() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port("callback invocation has no carrier argument".to_string()),
            invocation,
        ));
    }
    let carrier = match callback_carrier_from_vm(heap, &argument_values[0]) {
        Ok(carrier) => carrier,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                callback_error(error),
                invocation,
            ));
        }
    };
    let execution = match resolve_callback_invocation(
        row,
        method_ordinal,
        &method_abi_id,
        &carrier,
        callback_composition,
    ) {
        Ok(execution) => execution,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                callback_error(error),
                invocation,
            ));
        }
    };
    let provider_entry = match execution.provider_entry() {
        Ok(entry) => entry,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                callback_error(error),
                invocation,
            ));
        }
    };
    let provider_signature = provider_entry.signature().clone();
    let provider_image = Arc::clone(provider_entry.image());
    if provider_signature.parameter_types().len() != caller_signature.parameter_types().len()
        || provider_signature.result_types().len() != caller_signature.result_types().len()
        || provider_signature.result_plans() != caller_signature.result_plans()
    {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "callback provider signature drifts from the linked callback table".to_string(),
            ),
            invocation,
        ));
    }

    let mut child_heap = match child_heap_factory.create_child_heap(
        provider_image.owner(),
        request_composition.heap_limits.clone(),
        resources,
        Arc::clone(&request_composition.memory_ledger),
    ) {
        Ok(heap) => heap,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "callback child heap creation failed: {error}"
                )),
                invocation,
            ));
        }
    };

    let owner_heap = execution.owner_heap_arena();
    let owner_guard = match owner_heap.try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    BytecodeCallbackChildError::OwnerStateUnavailable.to_string(),
                ),
                invocation,
            ));
        }
    };
    let receiver_slot = match materialize_owner_runtime_value(
        &owner_guard,
        execution.receiver(),
        child_heap.heap_mut(),
        &provider_image,
        provider_signature.parameter_types()[0],
        &provider_signature.parameter_plans()[0],
    ) {
        Ok(slot) => slot,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                callback_error(error),
                invocation,
            ));
        }
    };
    if let Err(error) = child_heap.publish_staging_root(receiver_slot) {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(format!("callback receiver staging failed: {error}")),
            invocation,
        ));
    }

    let mut materialized_arguments = Vec::with_capacity(argument_values.len().saturating_sub(1));
    for (index, source) in argument_values.iter().skip(1).enumerate() {
        let provider_type = provider_signature.parameter_types()[index + 1];
        let plan = &provider_signature.parameter_plans()[index + 1];
        let materialized = match skiff_runtime_vm::materialize_local_interface_value(
            heap,
            source,
            child_heap.heap_mut(),
            &provider_image,
            provider_type,
            plan,
        ) {
            Ok(value) => value,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "callback argument {index} materialization failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        if let Err(error) = child_heap.publish_staging_root(materialized) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "callback argument {index} staging failed: {error}"
                )),
                invocation,
            ));
        }
        materialized_arguments.push(materialized);
    }
    drop(owner_guard);

    let (_, arguments, endpoint, resume) = invocation.into_parts();
    if endpoint.is_some() {
        return Err(BytecodePortFailure::continuation(
            BytecodeSchedulerError::Port(
                "callback child must not carry a stream endpoint".to_string(),
            ),
            resume,
        ));
    }
    for (value, plan) in arguments
        .values()
        .iter()
        .zip(caller_signature.parameter_plans().iter())
    {
        if let Err(error) = skiff_runtime_vm::release_local_interface_source(heap, value, plan) {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Port(format!(
                    "callback argument source release failed: {error}"
                )),
                resume,
            ));
        }
    }

    let mut provider_arguments = Vec::with_capacity(materialized_arguments.len() + 1);
    provider_arguments.push(receiver_slot);
    provider_arguments.extend(materialized_arguments);
    let fiber = match Vm::start(
        provider_entry,
        provider_arguments.into_boxed_slice(),
        limits,
        observer,
    ) {
        Ok(fiber) => fiber,
        Err(error) => {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Vm(error),
                resume,
            ));
        }
    };
    let finish = CallbackChildFinish {
        signature: caller_signature,
        opcode: Opcode::InvokeCallback,
    };
    Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
        unit: fiber,
        resume,
        child_heap,
        finish: Box::new(finish),
    }))
}

struct CallbackChildFinish {
    signature: LinkedCallableSignature,
    opcode: Opcode,
}

impl ChildFinish<VmFiber, VmResumeToken> for CallbackChildFinish {
    fn finish(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        if child_result.thrown_diagnostic().is_some() {
            return self.finish_throw(resume, child_result, child_heap, parent_heap);
        }
        let (outcome, mut residual) = match child_result.into_resume() {
            Ok(parts) => parts,
            Err(_) => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "callback child terminal failure cannot materialize to the caller".to_string(),
                )));
            }
        };
        let outcome = match outcome {
            ResumeOutcome::Values(child_values) => {
                if child_values.values().len() != self.signature.result_types().len() {
                    return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                        "callback result arity diverges from the linked method signature"
                            .to_string(),
                    )));
                }
                if child_values.values().is_empty() {
                    let mut child_escrow = child_values.into_terminal_escrow();
                    child_escrow
                        .release_all(child_heap.heap_mut())
                        .map_err(|error| {
                            ChildFinishError::failure(BytecodeSchedulerError::Vm(error))
                        })?;
                    return Ok(ResumeOutcome::Empty);
                }
                let mut caller_values = Vec::with_capacity(child_values.values().len());
                for (index, (source, (destination_type, plan))) in child_values
                    .values()
                    .iter()
                    .zip(
                        self.signature
                            .result_types()
                            .iter()
                            .zip(self.signature.result_plans().iter()),
                    )
                    .enumerate()
                {
                    match skiff_runtime_vm::materialize_local_interface_value(
                        child_heap.heap_mut(),
                        source,
                        parent_heap,
                        resume.image(),
                        *destination_type,
                        plan,
                    ) {
                        Ok(value) => caller_values.push(value),
                        Err(error) => {
                            for root in &caller_values {
                                let _ = parent_heap.release_snapshot(root);
                            }
                            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                                format!("callback result {index} materialization failed: {error}"),
                            )));
                        }
                    }
                }
                let caller_owned = match VmOwnedValues::try_from_resume(
                    resume,
                    caller_values.into_boxed_slice(),
                ) {
                    Ok(values) => values,
                    Err(rejected) => {
                        let message = rejected.error().to_string();
                        for root in rejected.values() {
                            let _ = parent_heap.release_snapshot(root);
                        }
                        return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                            message,
                        )));
                    }
                };
                let mut child_escrow = child_values.into_terminal_escrow();
                if let Err(error) = child_escrow.release_all(child_heap.heap_mut()) {
                    return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
                }
                ResumeOutcome::Values(caller_owned)
            }
            other => other,
        };
        if let Err(error) = residual.release_all(child_heap.heap_mut()) {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
        }
        Ok(outcome)
    }
}

impl CallbackChildFinish {
    fn finish_throw(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        let diagnostic = child_result
            .thrown_diagnostic()
            .expect("throw branch is selected only when a thrown diagnostic exists")
            .clone();
        let (outcome, mut residual) = match child_result.into_resume() {
            Ok(parts) => parts,
            Err(_) => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "callback throw completion lacks the exact owned exception".to_string(),
                )));
            }
        };
        let ResumeOutcome::Throw(mut child_exception) = outcome else {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "callback thrown completion did not carry an owned exception".to_string(),
            )));
        };
        let source_payload = child_exception.vm_local_payload().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "callback thrown completion has no VM-local payload".to_string(),
            ))
        })?;
        let source_tag = source_payload.compact_type_tag().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "callback thrown payload has no compact type tag".to_string(),
            ))
        })?;
        let leaf = TypeIndex::new(source_tag.type_index());
        let plan = resume.image().type_plan(leaf).cloned().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                "callback thrown payload type {} has no linked plan",
                leaf.get()
            )))
        })?;
        let caller_payload = skiff_runtime_vm::materialize_local_interface_value(
            child_heap.heap_mut(),
            &source_payload,
            parent_heap,
            resume.image(),
            leaf,
            &plan,
        )
        .map_err(|error| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                "callback throw materialization failed: {error}"
            )))
        })?;
        child_exception
            .release_all(child_heap.heap_mut())
            .map_err(|error| ChildFinishError::failure(BytecodeSchedulerError::Vm(error)))?;
        let site = VmLifecycleSite {
            function: resume.function(),
            instruction: resume.instruction(),
            opcode: self.opcode,
        };
        let caller_exception = VmOwnedException::try_from_caller_resume(
            Arc::clone(resume.image()),
            resume,
            parent_heap,
            Some(caller_payload),
            &diagnostic,
            plan,
            site,
        )
        .map_err(|rejected| {
            let (error, payload) = rejected.into_parts();
            if let Some(payload) = payload {
                let _ = parent_heap.release_snapshot(&payload);
            }
            ChildFinishError::failure(BytecodeSchedulerError::Port(error.to_string()))
        })?;
        if let Err(error) = residual.release_all(child_heap.heap_mut()) {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
        }
        Ok(ResumeOutcome::Throw(caller_exception))
    }
}

fn interface_table_by_index(
    image: &skiff_runtime_linker::DeploymentExecutionImage,
    table: skiff_runtime_linked_bytecode::InterfaceTableIndex,
) -> Option<&LinkedInterfaceTable> {
    let position = usize::try_from(table.get()).ok()?;
    image
        .interface_tables()
        .get(position)
        .filter(|row| row.index() == table)
}

fn callback_carrier_from_vm(
    heap: &mut dyn VmHeap,
    carrier: &ValueSlot,
) -> Result<CallbackCapabilityCarrier, BytecodeCallbackChildError> {
    let runtime_value = heap
        .as_any()
        .and_then(|heap| heap.downcast_ref::<crate::vm_heap::RequestVmHeap>())
        .ok_or_else(|| BytecodeCallbackChildError::Materialization {
            message: "callback carrier heap is not a request VM heap".to_string(),
        })?
        .runtime_value_for_slot(carrier)
        .map_err(|error| BytecodeCallbackChildError::Materialization {
            message: error.to_string(),
        })?;
    let RuntimeValue::Heap(handle) = runtime_value else {
        return Err(BytecodeCallbackChildError::Materialization {
            message: "callback carrier is not a heap interface value".to_string(),
        });
    };
    let request_heap = heap
        .as_any()
        .and_then(|heap| heap.downcast_ref::<crate::vm_heap::RequestVmHeap>())
        .expect("callback carrier heap was checked above")
        .request_heap();
    match request_heap.get(handle) {
        Ok(skiff_runtime_model::value::HeapNode::Interface(interface))
            if matches!(interface.carrier(), InterfaceCarrier::CallbackCapability(_)) =>
        {
            let InterfaceCarrier::CallbackCapability(carrier) = interface.carrier() else {
                unreachable!("callback carrier match was checked above")
            };
            Ok(carrier.clone())
        }
        _ => Err(BytecodeCallbackChildError::Materialization {
            message: "callback carrier is not an opaque callback capability".to_string(),
        }),
    }
}

fn materialize_owner_runtime_value(
    owner_heap: &RequestHeap,
    value: &RuntimeValue,
    destination: &mut dyn VmHeap,
    image: &skiff_runtime_linker::DeploymentExecutionImage,
    ty: TypeIndex,
    _plan: &skiff_runtime_linked_bytecode::LinkedValueTransferPlan,
) -> Result<ValueSlot, BytecodeCallbackChildError> {
    let destination_vm = destination
        .as_any_mut()
        .and_then(|heap| heap.downcast_mut::<crate::vm_heap::RequestVmHeap>())
        .ok_or_else(|| BytecodeCallbackChildError::Materialization {
            message: "callback destination heap is not a request VM heap".to_string(),
        })?;
    let cloned = deep_clone_runtime_value_between_heaps(
        owner_heap,
        destination_vm.request_heap_mut(),
        value,
    )
    .map_err(|error| BytecodeCallbackChildError::Materialization {
        message: error.to_string(),
    })?;
    runtime_value_to_slot(destination_vm, &cloned, image, ty)
}

fn runtime_value_to_slot(
    destination: &mut crate::vm_heap::RequestVmHeap,
    value: &RuntimeValue,
    _image: &skiff_runtime_linker::DeploymentExecutionImage,
    ty: TypeIndex,
) -> Result<ValueSlot, BytecodeCallbackChildError> {
    let tag = CompactTypeTag::try_from_type_index(ty.get()).ok_or_else(|| {
        BytecodeCallbackChildError::Materialization {
            message: format!("callback type {} does not fit compact tag", ty.get()),
        }
    })?;
    let flags = ValueFlags::new(0);
    match value {
        RuntimeValue::Null => Ok(ValueSlot::null()),
        RuntimeValue::Bool(value) => Ok(ValueSlot::bool(*value)),
        RuntimeValue::Number(value) => Ok(ValueSlot::number(*value)),
        RuntimeValue::Date(value) => Ok(ValueSlot::date(*value)),
        RuntimeValue::String(value) => {
            let handle = destination
                .request_heap_mut()
                .alloc_local_carrier_cell(
                    skiff_runtime_model::value::RuntimeValueCarrier::unidentified(
                        RuntimeValue::String(value.clone()),
                    ),
                )
                .map_err(|error| BytecodeCallbackChildError::Materialization {
                    message: error.to_string(),
                })?;
            destination.heap_ref(handle, tag, flags).map_err(|error| {
                BytecodeCallbackChildError::Materialization {
                    message: error.to_string(),
                }
            })
        }
        RuntimeValue::Heap(handle) => destination.heap_ref(*handle, tag, flags).map_err(|error| {
            BytecodeCallbackChildError::Materialization {
                message: error.to_string(),
            }
        }),
        RuntimeValue::ActorRef(_) => Err(BytecodeCallbackChildError::Materialization {
            message: "callback owner receiver cannot be an ActorRef".to_string(),
        }),
    }
}

fn callback_error(error: BytecodeCallbackChildError) -> BytecodeSchedulerError {
    BytecodeSchedulerError::Port(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_child_defaults_fail_closed_without_resolver() {
        let composition = BytecodeCallbackChildComposition::default();
        assert!(!composition.is_available());
        assert!(matches!(
            composition.require_resolver(),
            Err(BytecodeCallbackChildError::MissingRuntimeIdentity)
        ));
    }

    #[test]
    fn callback_required_fact_names_exact_f6_k6_x6_seams() {
        let required = callback_required_fact();
        assert!(required.contains("LinkedInterfaceTableKind::Callback"));
        assert!(required.contains("provider function"));
        assert!(required.contains("X6"));
        assert!(required.contains("Cross-Runtime"));
    }
}

pub(crate) fn callback_required_fact() -> String {
    "C6 callback child requires F6 to link LinkedInterfaceTableKind::Callback with exact \
     method_abi_id, canonical callback contract and provider function/DeploymentExecutionEntry; \
     K6 to expose callback carrier read/owner heap materialization; X6 to register \
     BytecodeCallbackChildComposition and route callback lanes. Cross-Runtime reverse transport \
     is intentionally disabled and must fail closed."
        .to_string()
}
