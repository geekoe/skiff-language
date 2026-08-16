//! Interface child leaf.
//!
//! Local and remote interface are registered on the same flat child lifecycle
//! as service. Remote method rows are dispatched through the exact linked
//! service operation/build; callback tables remain fail-closed.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::Opcode;
use skiff_runtime_boundary::vm_materialize::{
    boundary_value_matches_linked_type, materialize_linked_value, release_boundary_source,
};
use skiff_runtime_linked_bytecode::{
    InterfaceTableIndex, LinkedCallableSignature, LinkedInterfaceTableKind,
    LinkedLocalInterfaceTable, LinkedRemoteInterfaceTable, TypeIndex,
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
    materialize_local_interface_value, materialize_operation_receiver,
    release_local_interface_source, ChildInvocation, ChildTarget, ResumeOutcome, Vm, VmBudget,
    VmCompletion, VmFiber, VmLifecycleSite, VmLimits, VmOwnedException, VmOwnedValues,
    VmResumeToken,
};

use super::provider_receiver::provider_receiver_plan;
use super::{
    execute_callback_child, BytecodeChildHeapFactory, BytecodeRequestChildComposition,
    BytecodeServiceChildError, ServiceChildThrowMaterializer,
};

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
        method_ordinal: _,
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
            BytecodeSchedulerError::Port(interface_required_fact("F6 linked interface table rows")),
            invocation,
        ));
    };
    let LinkedInterfaceTableKind::Remote(remote) = row.kind() else {
        if interface_table_kind_is_child_executable(row.kind()) {
            return execute_local_interface_child(
                invocation,
                heap,
                _budget,
                composition,
                child_heap_factory,
                resources,
                observer,
                limits,
                row,
            );
        }
        if matches!(row.kind(), LinkedInterfaceTableKind::Callback(_)) {
            return execute_callback_child(
                invocation,
                heap,
                _budget,
                &composition.callback_child,
                composition,
                child_heap_factory,
                resources,
                observer,
                limits,
            );
        }
        unreachable!("callback is the only non-executable interface kind")
    };
    execute_remote_interface_child(
        invocation,
        heap,
        _budget,
        composition,
        child_heap_factory,
        resources,
        observer,
        limits,
        row,
        remote,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_local_interface_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    composition: &BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
    row: &skiff_runtime_linked_bytecode::LinkedInterfaceTable,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let image = Arc::clone(invocation.resume().image());
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
    debug_assert_eq!(
        row.index(),
        table,
        "interface child row was selected from the caller image by its exact index"
    );
    let _ = table;

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

#[allow(clippy::too_many_arguments)]
fn execute_remote_interface_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    composition: &BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
    row: &skiff_runtime_linked_bytecode::LinkedInterfaceTable,
    remote: &LinkedRemoteInterfaceTable,
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
    debug_assert_eq!(
        row.index(),
        table,
        "remote interface child row was selected from the caller image by its exact index"
    );
    let _ = table;
    let caller_image = Arc::clone(invocation.resume().image());
    let method = match remote
        .methods()
        .get(usize::try_from(method_ordinal).unwrap_or(usize::MAX))
        .filter(|method| method.method_slot() == method_ordinal)
        .cloned()
    {
        Some(method) => method,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "remote interface method row is absent from the linked remote table"
                        .to_string(),
                ),
                invocation,
            ));
        }
    };
    let operation_index = match method.service_operation() {
        Some(operation) => operation,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "remote interface method has no exact linked service operation/build"
                        .to_string(),
                ),
                invocation,
            ));
        }
    };
    let operation = match caller_image
        .service_operations()
        .get(operation_index.get() as usize)
        .cloned()
    {
        Some(operation) => operation,
        None => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "remote interface method links an absent service operation row".to_string(),
                ),
                invocation,
            ));
        }
    };
    if !remote_service_operation_matches(
        remote.service_requirement_key(),
        method.contract_operation_id(),
        remote.callee_protocol_identity(),
        operation.service_requirement_key(),
        operation.contract_operation_id(),
        operation.expected_protocol_identity(),
    ) {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote interface method links a drifted service operation row".to_string(),
            ),
            invocation,
        ));
    }
    let Some(slot) = caller_image.dependency_slot(remote.service_requirement_key()) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote interface dependency slot is absent from the caller image".to_string(),
            ),
            invocation,
        ));
    };
    if &slot.contract().service_protocol_identity != remote.callee_protocol_identity()
        || &slot.contract().service_protocol_identity != operation.expected_protocol_identity()
    {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote interface dependency protocol drifts from the linked remote table"
                    .to_string(),
            ),
            invocation,
        ));
    }
    if !remote_method_signature_matches_operation(method.signature(), operation.signature()) {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote interface method signature drifts from its linked service operation"
                    .to_string(),
            ),
            invocation,
        ));
    }
    let provider_image = match composition.service_resolver.resolve_service(
        slot,
        method.contract_operation_id(),
        remote.callee_protocol_identity(),
    ) {
        Ok(image) => image,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                remote_service_error(error),
                invocation,
            ));
        }
    };
    if provider_image.service_protocol_identity() != remote.callee_protocol_identity() {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "resolved remote provider protocol drifts from the remote interface contract"
                    .to_string(),
            ),
            invocation,
        ));
    }
    let provider_entry = match provider_image.operation_entry(method.contract_operation_id()) {
        Ok(entry) => entry,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "remote provider operation entry is absent: {error}"
                )),
                invocation,
            ));
        }
    };
    let provider_signature = provider_entry.signature().clone();
    let boundary_plan = operation.boundary_plan().clone();
    let receiver_plan = match provider_receiver_plan(
        &provider_signature,
        boundary_plan.arguments().len(),
        boundary_plan.results().len(),
        provider_entry.receiver(),
    ) {
        Ok(facts) => facts,
        Err(reason) => return Err(BytecodePortFailure::input(reason, invocation)),
    };
    if receiver_plan.parameter_offset != 1 {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote provider operation has no linked receiver parameter".to_string(),
            ),
            invocation,
        ));
    }
    if let Err(reason) =
        validate_remote_boundary_types(&provider_image, &provider_signature, &boundary_plan)
    {
        return Err(BytecodePortFailure::input(reason, invocation));
    }

    let argument_values = invocation.arguments().values().to_vec();
    if argument_values.len() != boundary_plan.arguments().len() + 1 {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(
                "remote interface invocation arity diverges from its linked boundary plan"
                    .to_string(),
            ),
            invocation,
        ));
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
    let mut provider_arguments = Vec::with_capacity(boundary_plan.arguments().len() + 1);
    let (Some(receiver_constant), Some(receiver_type), Some(receiver_plan)) = (
        receiver_plan.constant,
        receiver_plan.receiver_type,
        receiver_plan.receiver_plan.as_ref(),
    ) else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port("remote provider receiver plan is incomplete".to_string()),
            invocation,
        ));
    };
    let receiver_slot = match materialize_operation_receiver(
        child_heap.heap_mut(),
        &provider_image,
        receiver_constant,
        receiver_type,
        receiver_plan,
    ) {
        Ok(value) => value,
        Err(error) => {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "remote provider receiver materialization failed: {error}"
                )),
                invocation,
            ));
        }
    };
    if let Err(error) = child_heap.publish_staging_root(receiver_slot) {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::Port(format!(
                "remote provider receiver staging failed: {error}"
            )),
            invocation,
        ));
    }
    provider_arguments.push(receiver_slot);
    for (index, (source, value)) in argument_values
        .iter()
        .skip(1)
        .zip(boundary_plan.arguments())
        .enumerate()
    {
        let caller_type = value.caller_type();
        if source
            .compact_type_tag()
            .is_some_and(|tag| tag.type_index() != caller_type.get())
        {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "remote interface argument {index} does not carry the linked caller type tag"
                )),
                invocation,
            ));
        }
        let provider_type = provider_signature.parameter_types()[index + 1];
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
                        "remote interface argument {index} materialization failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        if let Err(error) = child_heap.publish_staging_root(materialized) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "remote interface argument {index} staging failed: {error}"
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
                "remote interface child must not carry a stream endpoint".to_string(),
            ),
            resume,
        ));
    }
    for value in arguments.values() {
        if let Err(error) = release_boundary_source(heap, value) {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Port(format!(
                    "remote interface argument source release failed: {error}"
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
    let finish = RemoteInterfaceChildFinish {
        boundary_plan,
        throw_materializer: Arc::clone(&composition.throw_materializer),
        unary_response_start: Arc::clone(&composition.unary_response_start),
    };
    Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
        unit: fiber,
        resume,
        child_heap,
        finish: Box::new(finish),
    }))
}

fn remote_service_operation_matches(
    remote_key: &skiff_artifact_model::ServiceRequirementKey,
    remote_operation: &skiff_artifact_model::ContractOperationId,
    remote_protocol: &skiff_artifact_model::ServiceProtocolIdentity,
    operation_key: &skiff_artifact_model::ServiceRequirementKey,
    operation_operation: &skiff_artifact_model::ContractOperationId,
    operation_protocol: &skiff_artifact_model::ServiceProtocolIdentity,
) -> bool {
    remote_key == operation_key
        && remote_operation == operation_operation
        && remote_protocol == operation_protocol
}

fn remote_method_signature_matches_operation(
    method: &LinkedCallableSignature,
    operation: &LinkedCallableSignature,
) -> bool {
    method.parameter_types().len() == operation.parameter_types().len().saturating_add(1)
        && method.result_types() == operation.result_types()
        && method.parameter_types().get(1..) == Some(operation.parameter_types())
        && method.parameter_modes().get(1..) == Some(operation.parameter_modes())
        && method.parameter_plans().get(1..) == Some(operation.parameter_plans())
        && method.result_plans() == operation.result_plans()
        && method.effect_summary() == operation.effect_summary()
}

fn interface_table_kind_is_child_executable(kind: &LinkedInterfaceTableKind) -> bool {
    matches!(
        kind,
        LinkedInterfaceTableKind::Requirement(_)
            | LinkedInterfaceTableKind::Local(_)
            | LinkedInterfaceTableKind::Remote(_)
    )
}

fn validate_remote_boundary_types(
    provider_image: &DeploymentExecutionImage,
    provider_signature: &LinkedCallableSignature,
    boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
) -> Result<(), BytecodeSchedulerError> {
    for (index, (plan, provider_type)) in boundary_plan
        .arguments()
        .iter()
        .zip(provider_signature.parameter_types().iter().skip(1))
        .enumerate()
    {
        if !boundary_value_matches_linked_type(provider_image, *provider_type, plan) {
            return Err(BytecodeSchedulerError::Port(format!(
                "remote provider parameter {index} type differs from the linked service boundary plan"
            )));
        }
    }
    for (index, (plan, provider_type)) in boundary_plan
        .results()
        .iter()
        .zip(provider_signature.result_types())
        .enumerate()
    {
        if !boundary_value_matches_linked_type(provider_image, *provider_type, plan) {
            return Err(BytecodeSchedulerError::Port(format!(
                "remote provider result {index} type differs from the linked service boundary plan"
            )));
        }
    }
    Ok(())
}

fn remote_service_error(error: BytecodeServiceChildError) -> BytecodeSchedulerError {
    BytecodeSchedulerError::Port(format!(
        "remote interface service resolution failed: {error}"
    ))
}

struct RemoteInterfaceChildFinish {
    boundary_plan: skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
    throw_materializer: Arc<dyn ServiceChildThrowMaterializer>,
    unary_response_start: Arc<AtomicBool>,
}

impl ChildFinish<VmFiber, VmResumeToken> for RemoteInterfaceChildFinish {
    fn finish(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        if child_result.thrown_diagnostic().is_some() {
            return self.throw_materializer.materialize_throw(
                child_result,
                child_heap,
                parent_heap,
                resume.image(),
                &self.boundary_plan,
            );
        }
        let (outcome, mut residual) = match child_result.into_resume() {
            Ok(parts) => parts,
            Err(_) => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "remote interface child terminal failure cannot materialize to the caller"
                        .to_string(),
                )));
            }
        };
        let outcome = match outcome {
            ResumeOutcome::Values(child_values) => {
                if !child_values.is_empty() {
                    let mut caller_values = Vec::with_capacity(child_values.len());
                    for (index, (source, plan)) in child_values
                        .values()
                        .iter()
                        .zip(self.boundary_plan.results())
                        .enumerate()
                    {
                        match materialize_linked_value(
                            child_heap.heap_mut(),
                            source,
                            parent_heap,
                            resume.image(),
                            plan.caller_type(),
                            plan,
                        ) {
                            Ok(value) => caller_values.push(value),
                            Err(error) => {
                                for root in &caller_values {
                                    let _ = parent_heap.release_snapshot(root);
                                }
                                return Err(ChildFinishError::failure(
                                    BytecodeSchedulerError::Port(format!(
                                        "remote interface result {index} materialization failed: {error}"
                                    )),
                                ));
                            }
                        }
                    }
                    let caller_values_owned = match VmOwnedValues::try_from_resume(
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
                    return Ok(ResumeOutcome::Values(caller_values_owned));
                }
                if !self.boundary_plan.results().is_empty() {
                    return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                        "remote provider returned zero results for a non-void boundary".to_string(),
                    )));
                }
                let _ = child_values
                    .into_terminal_escrow()
                    .release_all(child_heap.heap_mut());
                self.unary_response_start.store(true, Ordering::Release);
                ResumeOutcome::Empty
            }
            other => other,
        };
        if let Err(error) = residual.release_all(child_heap.heap_mut()) {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
        }
        Ok(outcome)
    }
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
        "interface child requires {seam}; F6 must link exact \
         LinkedInterfaceTableKind::Local/Remote rows with method_abi_id, signature, \
         function/service operation and receiver_call_abi; K6 exposes the heap-neutral \
         local interface carrier read/materialization API and registers the method on the \
         same flat ChildHeapCarrier/ChildFinish lifecycle as service"
    )
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        CallableEffectSummary, ContractOperationId, PackageBuildId, ParamModeIr,
        ServiceProtocolIdentity, ServiceRequirementKey,
    };
    use skiff_runtime_linked_bytecode::{LinkedInterfaceRequirementTable, LinkedPublicInstanceKey};
    use skiff_runtime_linked_bytecode::{LinkedValueDropPlan, LinkedValueTransferPlan};

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

    #[test]
    fn remote_interface_required_fact_names_service_operation_path() {
        let required = interface_required_fact("F6 exact remote table rows");
        assert!(
            required.contains("LinkedInterfaceTableKind::Local/Remote"),
            "interface fact seam must include the remote table path: {required}"
        );
        assert!(
            required.contains("service operation"),
            "remote interface must require the exact service operation path: {required}"
        );
        assert!(
            required.contains("ChildHeapCarrier/ChildFinish"),
            "remote interface must register on the service flat child lifecycle: {required}"
        );
    }

    #[test]
    fn remote_service_operation_matching_requires_exact_facts() {
        let key = ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:caller"),
            service_requirement_slot: 0,
        };
        let operation = ContractOperationId::new("operation:reader.read");
        let protocol = ServiceProtocolIdentity::new("protocol:reader-v1");
        let other_key = ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:other"),
            service_requirement_slot: 1,
        };
        let other_operation = ContractOperationId::new("operation:reader.scan");
        let other_protocol = ServiceProtocolIdentity::new("protocol:reader-v2");

        assert!(remote_service_operation_matches(
            &key, &operation, &protocol, &key, &operation, &protocol
        ));
        assert!(!remote_service_operation_matches(
            &key, &operation, &protocol, &other_key, &operation, &protocol
        ));
        assert!(!remote_service_operation_matches(
            &key,
            &operation,
            &protocol,
            &key,
            &other_operation,
            &protocol
        ));
        assert!(!remote_service_operation_matches(
            &key,
            &operation,
            &protocol,
            &key,
            &operation,
            &other_protocol
        ));
    }

    #[test]
    fn remote_method_signature_requires_exact_carrier_suffix() {
        let plan = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        };
        let carrier = LinkedCallableSignature::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([ParamModeIr::Value]),
            Box::new([plan]),
            Box::new([]),
            Box::new([]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("carrier signature is canonical");
        let operation = LinkedCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("empty service operation signature is canonical");
        let wrong_arity = LinkedCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("empty mismatch signature is canonical");

        assert!(remote_method_signature_matches_operation(
            &carrier, &operation
        ));
        assert!(!remote_method_signature_matches_operation(
            &wrong_arity,
            &operation
        ));
    }

    #[test]
    fn interface_child_kind_routing_keeps_local_and_remote_and_rejects_unknown() {
        let local = LinkedInterfaceTableKind::Local(
            LinkedLocalInterfaceTable::new(TypeIndex::new(0), Box::new([]))
                .expect("empty local table is canonical"),
        );
        let requirement = ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:caller"),
            service_requirement_slot: 0,
        };
        let remote = LinkedInterfaceTableKind::Remote(
            LinkedRemoteInterfaceTable::new(
                requirement,
                LinkedPublicInstanceKey::parse("instance:reader")
                    .expect("public instance key is canonical"),
                Box::new([]),
                ServiceProtocolIdentity::new("protocol:reader-v1"),
            )
            .expect("empty remote table is canonical"),
        );
        let requirement_only = LinkedInterfaceTableKind::Requirement(
            LinkedInterfaceRequirementTable::new(Box::new([]))
                .expect("empty requirement table is canonical"),
        );
        let callback = LinkedInterfaceTableKind::Callback(
            LinkedInterfaceRequirementTable::new(Box::new([]))
                .expect("empty callback table is canonical"),
        );

        assert!(interface_table_kind_is_child_executable(&local));
        assert!(interface_table_kind_is_child_executable(&remote));
        assert!(interface_table_kind_is_child_executable(&requirement_only));
        assert!(!interface_table_kind_is_child_executable(&callback));
    }
}
