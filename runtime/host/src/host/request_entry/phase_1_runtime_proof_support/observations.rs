use skiff_artifact_model::Opcode;
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionEvent, BytecodeExecutionObservation, BytecodeRequestTerminal,
    FrozenOwnerDomain, VmObservedFrameRole,
};

const DEFAULT_RAW_FUEL_LIMIT: u64 =
    skiff_runtime_request::execution_budget::DEFAULT_INSTRUCTION_LIMIT;

/// Returns every missing or wrong Phase 1 proof obligation; green iff empty.
pub(in crate::host::request_entry) fn phase_1_observation_gaps(
    observations: &[BytecodeExecutionObservation],
) -> Vec<String> {
    assert_contiguous_correlation(observations);
    let mut gaps = Vec::new();

    if observations.len() != 11 {
        gaps.push(format!(
            "Phase 1 scalar VCP must emit exactly 11 observations, observed {}",
            observations.len()
        ));
        return gaps;
    }

    for (ordinal, observation) in observations.iter().enumerate() {
        let expected = expected_variant_name(ordinal);
        let matches_ordinal = match ordinal {
            0 => matches!(
                &observation.event,
                BytecodeExecutionEvent::DeploymentImageSelected(_)
            ),
            1 => matches!(
                &observation.event,
                BytecodeExecutionEvent::RouteEntryPinned(_)
            ),
            2 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmFunctionFrameEntered(entry)
                    if entry.role == VmObservedFrameRole::Root
            ),
            3 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmFirstInstructionDispatched(dispatch)
                    if dispatch.opcode == Opcode::LoadSlot
            ),
            4 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmLocalCallDispatched(_)
            ),
            5 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmFunctionFrameEntered(entry)
                    if entry.role == VmObservedFrameRole::FirstRootLocalCallee
            ),
            6 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmFunctionReturned(returned)
                    if returned.role == VmObservedFrameRole::FirstRootLocalCallee
            ),
            7 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmFunctionReturned(returned)
                    if returned.role == VmObservedFrameRole::Root
            ),
            8 => matches!(
                &observation.event,
                BytecodeExecutionEvent::VmBudgetAccounted(_)
            ),
            9 => matches!(
                &observation.event,
                BytecodeExecutionEvent::RequestTerminalClaimed(terminal)
                    if terminal.terminal == BytecodeRequestTerminal::Succeeded
            ),
            10 => matches!(
                &observation.event,
                BytecodeExecutionEvent::RequestCleanupComplete(_)
            ),
            _ => unreachable!("the eleven-event length check bounds every ordinal"),
        };
        if !matches_ordinal {
            gaps.push(format!(
                "ordinal {ordinal} must be {expected}, observed {:?}",
                observation.event
            ));
        }
    }

    if !gaps.is_empty() {
        return gaps;
    }

    check_typed_field_facts(observations, &mut gaps);
    gaps
}

fn assert_contiguous_correlation(observations: &[BytecodeExecutionObservation]) {
    assert!(!observations.is_empty(), "VCP must emit production facts");
    let correlation = &observations[0].correlation;
    for (ordinal, observation) in observations.iter().enumerate() {
        assert_eq!(&observation.correlation, correlation);
        assert_eq!(observation.ordinal, ordinal as u64);
    }
}

fn expected_variant_name(ordinal: usize) -> &'static str {
    match ordinal {
        0 => "DeploymentImageSelected",
        1 => "RouteEntryPinned",
        2 => "VmFunctionFrameEntered(Root)",
        3 => "VmFirstInstructionDispatched(LoadSlot)",
        4 => "VmLocalCallDispatched(root -> helper)",
        5 => "VmFunctionFrameEntered(FirstRootLocalCallee)",
        6 => "VmFunctionReturned(FirstRootLocalCallee)",
        7 => "VmFunctionReturned(Root)",
        8 => "VmBudgetAccounted",
        9 => "RequestTerminalClaimed(Succeeded)",
        10 => "RequestCleanupComplete",
        _ => unreachable!("the eleven-event length check bounds every ordinal"),
    }
}

fn check_typed_field_facts(observations: &[BytecodeExecutionObservation], gaps: &mut Vec<String>) {
    let events = observations
        .iter()
        .map(|observation| &observation.event)
        .collect::<Vec<_>>();
    let [BytecodeExecutionEvent::DeploymentImageSelected(_), BytecodeExecutionEvent::RouteEntryPinned(_), BytecodeExecutionEvent::VmFunctionFrameEntered(root_frame), BytecodeExecutionEvent::VmFirstInstructionDispatched(dispatched), BytecodeExecutionEvent::VmLocalCallDispatched(call), BytecodeExecutionEvent::VmFunctionFrameEntered(helper_frame), BytecodeExecutionEvent::VmFunctionReturned(helper_return), BytecodeExecutionEvent::VmFunctionReturned(root_return), BytecodeExecutionEvent::VmBudgetAccounted(budget), BytecodeExecutionEvent::RequestTerminalClaimed(terminal), BytecodeExecutionEvent::RequestCleanupComplete(cleanup)] =
        events.as_slice()
    else {
        unreachable!("the per-ordinal variant match already accepted the eleven-event sequence");
    };

    let root_function = dispatched.root_entry_function_index;

    if dispatched.current_function_index != root_function {
        gaps.push(format!(
            "the first successful dispatch must run inside the root entry function {}; observed currentFunctionIndex {}",
            root_function, dispatched.current_function_index
        ));
    }
    if dispatched.instruction_index != 0 {
        gaps.push(format!(
            "the first successful dispatch must be instructionIndex 0, observed {}",
            dispatched.instruction_index
        ));
    }
    if dispatched.opcode != Opcode::LoadSlot {
        gaps.push(format!(
            "the production VCP must read the real scalar parameter slot, observed {:?}",
            dispatched.opcode
        ));
    }
    if terminal.terminal != BytecodeRequestTerminal::Succeeded {
        gaps.push(format!(
            "the sole request terminal must be Succeeded, observed {:?}",
            terminal.terminal
        ));
    }

    if root_frame.function_index != root_function {
        gaps.push(format!(
            "the root frame must own the pinned root entry function {}, observed functionIndex {}",
            root_function, root_frame.function_index
        ));
    }
    if root_frame.frame_depth != 1 {
        gaps.push(format!(
            "the root frame must sit at frameDepth 1, observed {}",
            root_frame.frame_depth
        ));
    }
    if root_frame.slot_count == 0 {
        gaps.push("the root frame must report a positive slotCount".to_string());
    }

    let helper_function = call.callee_function_index;
    if call.caller_function_index != root_function {
        gaps.push(format!(
            "the selected CallLocal caller must be the root entry function {}, observed {}",
            root_function, call.caller_function_index
        ));
    }
    if helper_function == root_function {
        gaps.push(format!(
            "the selected CallLocal callee must differ from the root entry function {root_function}"
        ));
    }
    if call.caller_frame_depth != 1 {
        gaps.push(format!(
            "the selected CallLocal must leave the root caller at frameDepth 1, observed {}",
            call.caller_frame_depth
        ));
    }
    if call.callee_frame_depth != 2 {
        gaps.push(format!(
            "the selected CallLocal must install its callee at frameDepth 2, observed {}",
            call.callee_frame_depth
        ));
    }

    if helper_frame.function_index != helper_function {
        gaps.push(format!(
            "the selected callee frame must own the called function {}, observed functionIndex {}",
            helper_function, helper_frame.function_index
        ));
    }
    if helper_frame.frame_depth != 2 {
        gaps.push(format!(
            "the selected callee frame must sit at frameDepth 2, observed {}",
            helper_frame.frame_depth
        ));
    }

    if helper_return.function_index != helper_function {
        gaps.push(format!(
            "the helper normal return must be from the called function {}, observed functionIndex {}",
            helper_function, helper_return.function_index
        ));
    }
    if helper_return.caller_function_index != Some(root_function) {
        gaps.push(format!(
            "the helper normal return must resume the root entry function {}, observed callerFunctionIndex {:?}",
            root_function, helper_return.caller_function_index
        ));
    }
    if helper_return.remaining_frame_depth != 1 {
        gaps.push(format!(
            "the helper normal return must leave the root frame at remainingFrameDepth 1, observed {}",
            helper_return.remaining_frame_depth
        ));
    }

    if root_return.function_index != root_function {
        gaps.push(format!(
            "the root normal return must be from the root entry function {}, observed functionIndex {}",
            root_function, root_return.function_index
        ));
    }
    if root_return.caller_function_index != None {
        gaps.push(format!(
            "the root normal return must have no caller, observed callerFunctionIndex {:?}",
            root_return.caller_function_index
        ));
    }
    if root_return.remaining_frame_depth != 0 {
        gaps.push(format!(
            "the root normal return must leave remainingFrameDepth 0, observed {}",
            root_return.remaining_frame_depth
        ));
    }

    if budget.raw_executed_count == 0 {
        gaps.push("the VCP budget must report a positive rawExecutedCount".to_string());
    }
    if budget.charged_instruction_count != budget.raw_executed_count {
        gaps.push(format!(
            "chargedInstructionCount must equal rawExecutedCount {}, observed {}",
            budget.raw_executed_count, budget.charged_instruction_count
        ));
    }
    if budget.hard_limit != DEFAULT_RAW_FUEL_LIMIT {
        gaps.push(format!(
            "hardLimit must be the exact finite default {DEFAULT_RAW_FUEL_LIMIT}, observed {}",
            budget.hard_limit
        ));
    }
    if budget.poll_count == 0 {
        gaps.push("the VCP budget must report a positive pollCount".to_string());
    }
    if budget.raw_executed_count >= budget.hard_limit {
        gaps.push(format!(
            "rawExecutedCount {} must stay below hardLimit {}",
            budget.raw_executed_count, budget.hard_limit
        ));
    }

    let inventory = cleanup.owner_inventory;
    check_zero_never_created_domain(inventory.pending, "pending", gaps);
    check_zero_never_created_domain(inventory.resource, "resource", gaps);
    check_zero_never_created_domain(inventory.child, "child", gaps);

    // The typed event enum has no separate Pending/Resource/Child variant: the
    // per-ordinal match above accepted exactly the nine known variants in the
    // eleven-event sequence, so no domain-shaped event can hide outside it.
    assert_eq!(
        observations.len() - 1,
        10,
        "cleanup must be the final observation"
    );
}

fn check_zero_never_created_domain(domain: FrozenOwnerDomain, name: &str, gaps: &mut Vec<String>) {
    if domain.current != 0 {
        gaps.push(format!(
            "cleanup ownerInventory.{name}.current must be exactly 0, observed {}",
            domain.current
        ));
    }
    if domain.ever_created {
        gaps.push(format!(
            "cleanup ownerInventory.{name}.everCreated must be false"
        ));
    }
}
