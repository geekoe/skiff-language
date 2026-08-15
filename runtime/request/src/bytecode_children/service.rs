//! First accepted child lane: same-runtime service operations.

use std::sync::Arc;

use skiff_runtime_boundary::vm_materialize::{
    linked_type_for_contract, materialize_linked_value, release_boundary_source,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver, vm_heap::VmHeap,
};
use skiff_runtime_scheduler::{
    BytecodeChildHandoff, BytecodeChildStart, BytecodePortFailure, BytecodeSchedulerError,
    ChildFinish, ChildFinishError, RequestResourceTable,
};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, ResumeOutcome, Vm, VmBudget, VmCompletion, VmFiber, VmLimits,
    VmResumeToken,
};

use super::{
    service_operation_by_index, BytecodeChildHeapFactory, BytecodeRequestChildComposition,
    BytecodeServiceChildError,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_service_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    composition: &BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Service(index) = invocation.target() else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };
    let caller_image = Arc::clone(invocation.resume().image());
    let Some(target) = service_operation_by_index(&caller_image, index) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port("service operation table row is absent".to_string()),
            invocation,
        ));
    };
    let Some(slot) = caller_image.dependency_slot(target.service_requirement_key()) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "service operation dependency slot is absent from the caller image".to_string(),
            ),
            invocation,
        ));
    };
    if &slot.contract().service_protocol_identity != target.expected_protocol_identity() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "service dependency protocol identity drifts from the linked call target"
                    .to_string(),
            ),
            invocation,
        ));
    }
    let provider_image = match composition.service_resolver.resolve_service(
        slot,
        target.contract_operation_id(),
        target.expected_protocol_identity(),
    ) {
        Ok(image) => image,
        Err(error) => {
            return Err(BytecodePortFailure::input(service_error(error), invocation));
        }
    };
    if provider_image.service_protocol_identity() != target.expected_protocol_identity() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "resolved provider protocol identity drifts from the caller contract".to_string(),
            ),
            invocation,
        ));
    }
    let provider_entry = match provider_image.operation_entry(target.contract_operation_id()) {
        Ok(entry) => entry,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "provider operation entry is absent: {error}"
                )),
                invocation,
            ));
        }
    };
    let provider_signature = provider_entry.signature().clone();
    let boundary_plan = target.boundary_plan().clone();
    if provider_signature.parameter_types().len() != boundary_plan.arguments().len()
        || provider_signature.result_types().len() != boundary_plan.results().len()
    {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "provider operation signature and linked service boundary plan disagree"
                    .to_string(),
            ),
            invocation,
        ));
    }
    if let Err(reason) =
        validate_boundary_types(&provider_image, &provider_signature, &boundary_plan)
    {
        return Err(BytecodePortFailure::input(reason, invocation));
    }

    let mut child_heap = match child_heap_factory.create_child_heap(
        provider_image.owner(),
        composition.heap_limits.clone(),
        resources,
        Arc::clone(&composition.memory_ledger),
    ) {
        Ok(heap) => heap,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!("child heap creation failed: {error}")),
                invocation,
            ));
        }
    };

    let argument_values = invocation.arguments().values().to_vec();
    let mut provider_arguments = Vec::with_capacity(argument_values.len());
    for (index, (source, value)) in argument_values
        .iter()
        .zip(boundary_plan.arguments().iter())
        .enumerate()
    {
        let caller_type = value.caller_type();
        if source
            .compact_type_tag()
            .is_some_and(|tag| tag.type_index() != caller_type.get())
        {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "service argument {index} does not carry the linked caller type tag"
                )),
                invocation,
            ));
        }
        let provider_type = provider_signature.parameter_types()[index];
        let materialized = match materialize_linked_value(
            heap,
            source,
            child_heap.heap_mut(),
            &provider_image,
            provider_type,
            value,
        ) {
            Ok(value) => value,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "service argument materialization failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        if let Err(error) = child_heap.publish_staging_root(materialized) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!("service argument staging failed: {error}")),
                invocation,
            ));
        }
        provider_arguments.push(materialized);
    }

    let (_, arguments, endpoint, resume) = invocation.into_parts();
    if endpoint.is_some() {
        return Err(BytecodePortFailure::continuation(
            BytecodeSchedulerError::Port(
                "service child must not carry a stream endpoint".to_string(),
            ),
            resume,
        ));
    }
    for value in arguments.values() {
        if let Err(error) = release_boundary_source(heap, value) {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Port(format!(
                    "service argument source release failed: {error}"
                )),
                resume,
            ));
        }
    }

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
    let finish = ServiceChildFinish { boundary_plan };
    Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
        unit: fiber,
        resume,
        child_heap,
        finish: Box::new(finish),
    }))
}

fn validate_boundary_types(
    provider_image: &DeploymentExecutionImage,
    provider_signature: &skiff_runtime_linked_bytecode::LinkedCallableSignature,
    boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
) -> Result<(), BytecodeSchedulerError> {
    for (index, (plan, provider_type)) in boundary_plan
        .arguments()
        .iter()
        .zip(provider_signature.parameter_types())
        .enumerate()
    {
        let Some(linked) = linked_type_for_contract(provider_image, plan.contract_type()) else {
            return Err(BytecodeSchedulerError::Port(format!(
                "provider image lacks the linked service boundary type for argument {index}"
            )));
        };
        if linked != *provider_type {
            return Err(BytecodeSchedulerError::Port(format!(
                "provider parameter {index} type differs from the linked service boundary plan"
            )));
        }
    }
    for (index, (plan, provider_type)) in boundary_plan
        .results()
        .iter()
        .zip(provider_signature.result_types())
        .enumerate()
    {
        let Some(linked) = linked_type_for_contract(provider_image, plan.contract_type()) else {
            return Err(BytecodeSchedulerError::Port(format!(
                "provider image lacks the linked service boundary type for result {index}"
            )));
        };
        if linked != *provider_type {
            return Err(BytecodeSchedulerError::Port(format!(
                "provider result {index} type differs from the linked service boundary plan"
            )));
        }
    }
    Ok(())
}

fn service_error(error: BytecodeServiceChildError) -> BytecodeSchedulerError {
    BytecodeSchedulerError::Port(format!("service child resolution failed: {error}"))
}

struct ServiceChildFinish {
    boundary_plan: skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
}

impl ChildFinish<VmFiber> for ServiceChildFinish {
    fn finish(
        &self,
        child_result: VmCompletion,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        _parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        if child_result.thrown_diagnostic().is_some() {
            let (mut cause, mut escrow) = child_result.into_terminal();
            if let Some(cause) = cause.as_mut() {
                let _ = cause.release_all(child_heap.heap_mut());
            }
            let _ = escrow.release_all(child_heap.heap_mut());
            return Err(ChildFinishError::Failure(BytecodeSchedulerError::Port(
                "service child ordinary throw requires K6 cross-image VmOwnedException API"
                    .to_string(),
            )));
        }
        let (outcome, mut residual) = match child_result.into_resume() {
            Ok(parts) => parts,
            Err(_) => {
                return Err(ChildFinishError::Failure(BytecodeSchedulerError::Port(
                    "service child terminal failure cannot materialize to the caller".to_string(),
                )));
            }
        };
        let outcome = match outcome {
            ResumeOutcome::Values(values) => {
                if !values.is_empty() {
                    let mut escrow = values.into_terminal_escrow();
                    let _ = escrow.release_all(child_heap.heap_mut());
                    return Err(ChildFinishError::Failure(BytecodeSchedulerError::Port(
                        "service result materialization requires K6 ChildFinish resume token API"
                            .to_string(),
                    )));
                }
                if !self.boundary_plan.results().is_empty() {
                    return Err(ChildFinishError::Failure(BytecodeSchedulerError::Port(
                        "provider returned zero results for a non-void linked service boundary"
                            .to_string(),
                    )));
                }
                let _ = values
                    .into_terminal_escrow()
                    .release_all(child_heap.heap_mut());
                ResumeOutcome::Empty
            }
            other => other,
        };
        if let Err(error) = residual.release_all(child_heap.heap_mut()) {
            return Err(ChildFinishError::Failure(BytecodeSchedulerError::Vm(error)));
        }
        Ok(outcome)
    }
}
