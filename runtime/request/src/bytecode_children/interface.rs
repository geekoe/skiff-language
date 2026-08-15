//! Local interface child leaf.
//!
//! Local interface is registered on the same flat child lifecycle as service.
//! Remote interface, callback and requirement tables remain disabled until
//! their capability lanes land.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    InterfaceTableIndex, LinkedCallableSignature, LinkedInterfaceTableKind,
    LinkedLocalInterfaceTable, TypeIndex,
};
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver, vm_heap::VmHeap,
};
use skiff_runtime_scheduler::{
    BytecodeChildHandoff, BytecodeChildStart, BytecodePortFailure, BytecodeSchedulerError,
    ChildFinish, ChildFinishError, RequestResourceTable,
};
use skiff_runtime_vm::{
    materialize_local_interface_value, release_local_interface_source, ChildInvocation,
    ChildTarget, ResumeOutcome, Vm, VmBudget, VmCompletion, VmFiber, VmLifecycleSite, VmLimits,
    VmOwnedException, VmOwnedValues, VmResumeToken,
};

use super::{BytecodeChildHeapFactory, BytecodeRequestChildComposition};

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_interface_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    composition: &BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Interface {
        table,
        method_ordinal,
    } = invocation.target()
    else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };

    let image = Arc::clone(invocation.resume().image());
    let Some(row) = interface_table_by_index(&image, table) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(interface_required_fact(
                "F6 linked LinkedInterfaceTableKind::Local rows",
            )),
            invocation,
        ));
    };
    if matches!(
        row.kind(),
        LinkedInterfaceTableKind::Remote(_) | LinkedInterfaceTableKind::Callback(_)
    ) {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    }

    let argument_values = invocation.arguments().values().to_vec();
    if argument_values.is_empty() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "local interface invocation has no carrier argument".to_string(),
            ),
            invocation,
        ));
    }
    let carrier = argument_values[0];
    let carrier_table = match heap.local_interface_table(&carrier) {
        Ok(table) => table,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "local interface carrier identity read failed: {error}"
                )),
                invocation,
            ));
        }
    };
    let linked_local =
        match Arc::clone(carrier_table.exact()).downcast::<LinkedLocalInterfaceTable>() {
            Ok(table) => table,
            Err(_) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "local interface carrier does not hold the expected linked local table"
                            .to_string(),
                    ),
                    invocation,
                ));
            }
        };
    if carrier_table.concrete_type() != linked_local.concrete_type().get()
        || carrier_table.method_count() != linked_local.methods().len()
    {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "local interface carrier does not match the linked method table".to_string(),
            ),
            invocation,
        ));
    }
    let row_signature = match row.kind() {
        LinkedInterfaceTableKind::Requirement(requirement) => requirement
            .methods()
            .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
            .map(|method| method.signature()),
        LinkedInterfaceTableKind::Local(local) => local
            .methods()
            .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
            .map(|method| method.signature()),
        LinkedInterfaceTableKind::Remote(_) | LinkedInterfaceTableKind::Callback(_) => None,
    };
    let row_signature = match row_signature {
        Some(signature) => signature,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "local interface method row is absent from the linked call table".to_string(),
                ),
                invocation,
            ));
        }
    };
    let method = match linked_local
        .methods()
        .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
        .filter(|method| method.method_slot() == method_ordinal)
        .cloned()
    {
        Some(method) => method,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "local interface method row is absent from the carrier local table".to_string(),
                ),
                invocation,
            ));
        }
    };
    if row_signature != method.signature() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "local interface call signature drifts from the carrier local table".to_string(),
            ),
            invocation,
        ));
    }
    let method_entry = match image.function_entry(method.function()) {
        Ok(entry) => entry,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "local interface method function is absent: {error}"
                )),
                invocation,
            ));
        }
    };
    let signature = method.signature().clone();
    let entry_signature = method_entry.signature().clone();
    if method_entry.signature().parameter_types().len() != signature.parameter_types().len()
        || method_entry.signature().parameter_plans() != signature.parameter_plans()
        || method_entry.signature().result_types().len() != signature.result_types().len()
        || method_entry.signature().result_plans() != signature.result_plans()
    {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "local interface method entry signature plan/arity drifts from the linked method row"
                    .to_string(),
            ),
            invocation,
        ));
    }
    if argument_values.len() != signature.parameter_types().len() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "local interface invocation arity diverges from the linked method signature"
                    .to_string(),
            ),
            invocation,
        ));
    }

    let mut child_heap = match child_heap_factory.create_child_heap(
        image.owner(),
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
    let payload = match heap.local_interface_payload(&carrier) {
        Ok(payload) => payload,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "local interface carrier payload read failed: {error}"
                )),
                invocation,
            ));
        }
    };
    let mut provider_arguments = Vec::with_capacity(argument_values.len());
    for (index, (source, destination_type)) in argument_values
        .iter()
        .zip(entry_signature.parameter_types())
        .enumerate()
    {
        let source_value = if index == 0 { &payload } else { source };
        let plan = &entry_signature.parameter_plans()[index];
        let materialized = match materialize_local_interface_value(
            heap,
            source_value,
            child_heap.heap_mut(),
            &image,
            *destination_type,
            plan,
        ) {
            Ok(value) => value,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "local interface argument {index} materialization failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        if let Err(error) = child_heap.publish_staging_root(materialized) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "local interface argument {index} staging failed: {error}"
                )),
                invocation,
            ));
        }
        provider_arguments.push(materialized);
    }

    let (_, arguments, endpoint, resume) = invocation.into_parts();
    if endpoint.is_some() {
        return Err(BytecodePortFailure::continuation(
            BytecodeSchedulerError::Port(
                "local interface child must not carry a stream endpoint".to_string(),
            ),
            resume,
        ));
    }
    for (value, plan) in arguments
        .values()
        .iter()
        .zip(signature.parameter_plans().iter())
    {
        if let Err(error) = release_local_interface_source(heap, value, plan) {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Port(format!(
                    "local interface argument source release failed: {error}"
                )),
                resume,
            ));
        }
    }

    let fiber = match Vm::start(
        method_entry,
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
    let finish = LocalInterfaceChildFinish {
        signature,
        opcode: Opcode::CallInterface,
        unary_response_start: Arc::clone(&composition.unary_response_start),
    };
    Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
        unit: fiber,
        resume,
        child_heap,
        finish: Box::new(finish),
    }))
}

struct LocalInterfaceChildFinish {
    signature: LinkedCallableSignature,
    opcode: Opcode,
    unary_response_start: Arc<AtomicBool>,
}

impl ChildFinish<VmFiber, VmResumeToken> for LocalInterfaceChildFinish {
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
                    "local interface child terminal failure cannot materialize to the caller"
                        .to_string(),
                )));
            }
        };
        let outcome = match outcome {
            ResumeOutcome::Values(child_values) => {
                if child_values.values().len() != self.signature.result_types().len() {
                    return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                        "local interface result arity diverges from the linked method signature"
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
                    match materialize_local_interface_value(
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
                                format!(
                                    "local interface result {index} materialization failed: {error}"
                                ),
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
                self.unary_response_start.store(true, Ordering::Release);
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

impl LocalInterfaceChildFinish {
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
                    "local interface throw completion lacks the exact owned exception".to_string(),
                )));
            }
        };
        let ResumeOutcome::Throw(mut child_exception) = outcome else {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "local interface thrown completion did not carry an owned exception".to_string(),
            )));
        };
        let source_payload = child_exception.vm_local_payload().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "local interface thrown completion has no VM-local payload".to_string(),
            ))
        })?;
        let source_tag = source_payload.compact_type_tag().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "local interface thrown payload has no compact type tag".to_string(),
            ))
        })?;
        let leaf = TypeIndex::new(source_tag.type_index());
        let plan = resume.image().type_plan(leaf).cloned().ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                "local interface thrown payload type {} has no linked plan",
                leaf.get()
            )))
        })?;
        let caller_payload = materialize_local_interface_value(
            child_heap.heap_mut(),
            &source_payload,
            parent_heap,
            resume.image(),
            leaf,
            &plan,
        )
        .map_err(|error| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                "local interface throw materialization failed: {error}"
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
    table: InterfaceTableIndex,
) -> Option<&skiff_runtime_linked_bytecode::LinkedInterfaceTable> {
    let position = usize::try_from(table.get()).ok()?;
    image
        .interface_tables()
        .get(position)
        .filter(|row| row.index() == table)
}

pub(crate) fn interface_required_fact(seam: &str) -> String {
    format!(
        "local interface child requires {seam}; F6 must link \
         LinkedInterfaceTableKind::Local with concrete_type, method_abi_id, function and \
         receiver_call_abi; K6 exposes the heap-neutral local interface carrier \
         read/materialization API and registers the method on the same flat \
         ChildHeapCarrier/ChildFinish lifecycle as service"
    )
}

#[cfg(test)]
mod tests {
    use super::super::BytecodeChildLane;
    use super::*;

    #[test]
    fn local_interface_registration_names_flat_child_lifecycle() {
        let required = interface_required_fact("K6 local interface carrier read");
        assert!(
            required.contains("ChildHeapCarrier/ChildFinish"),
            "local interface must register on the service flat child lifecycle: {required}"
        );
        assert!(
            required.contains("LinkedInterfaceTableKind::Local"),
            "local interface must require the F6 local table: {required}"
        );
    }

    #[test]
    fn disabled_targets_do_not_route_to_interface() {
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::StreamNext),
            BytecodeChildLane::Disabled
        );
    }
}
