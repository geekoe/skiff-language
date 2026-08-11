use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::ValidatedFunction;
use skiff_runtime_linked_bytecode::{
    InstructionIndex, LinkedFrameLayout, LinkedInstruction,
    LinkedProgramPointState, LinkedSlotState, LinkedStackMapCandidate, LinkedStackValue, SpecializationKey,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation};

use super::{function_location, obligation_error, MachineState};

pub(super) fn initial_state(
    frame: &LinkedFrameLayout,
    location: BytecodeLinkLocation,
) -> Result<MachineState, BytecodeLinkError> {
    let mut slots = vec![LinkedSlotState::Uninitialized; frame.slot_types().len()];
    for parameter in frame.parameters() {
        let slot = parameter.slot().get() as usize;
        let ty = frame.slot_types().get(slot).copied().ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("parameter slot {slot} is out of bounds"),
            )
        })?;
        let entry = slots.get_mut(slot).ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("parameter state slot {slot} is out of bounds"),
            )
        })?;
        *entry = LinkedSlotState::Live(LinkedStackValue::new(ty, parameter.plan().clone()));
    }
    Ok(MachineState {
        stack: Vec::new(),
        slots,
        active_regions: Vec::new(),
        writable_loans: Vec::new(),
    })
}

pub(super) fn merge_successors(
    package: &HydratedBytecodePackage,
    source: &ValidatedFunction,
    successors: Vec<(usize, MachineState)>,
    states: &mut BTreeMap<usize, MachineState>,
    pending: &mut BTreeSet<usize>,
    function_location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    for (successor, next) in successors {
        let successor_pc = source
            .instructions
            .get(successor)
            .map(|instruction| instruction.pc)
            .ok_or_else(|| {
                obligation_error(
                    function_location.clone(),
                    format!("successor instruction {successor} is out of bounds"),
                )
            })?;
        match states.get(&successor) {
            Some(existing) => {
                let merged = merge_machine_states(
                    package,
                    source,
                    successor_pc,
                    existing,
                    &next,
                )?;
                if &merged != existing {
                    states.insert(successor, merged);
                    pending.insert(successor);
                }
            }
            None => {
                states.insert(successor, next);
                pending.insert(successor);
            }
        }
    }
    Ok(())
}

fn merge_machine_states(
    package: &HydratedBytecodePackage,
    source: &ValidatedFunction,
    successor_pc: u32,
    existing: &MachineState,
    next: &MachineState,
) -> Result<MachineState, BytecodeLinkError> {
    if existing == next {
        return Ok(existing.clone());
    }
    if existing.stack != next.stack
        || existing.active_regions != next.active_regions
        || existing.writable_loans != next.writable_loans
    {
        return Err(obligation_error(
            BytecodeLinkLocation::Instruction {
                package: Box::new(package.reference().clone()),
                function_key: source.function_key.clone(),
                artifact_pc: successor_pc,
            },
            "control-flow predecessors produce different concrete stack or slot states"
                .to_string(),
        ));
    }
    if existing.slots.len() != next.slots.len() {
        return Err(obligation_error(
            BytecodeLinkLocation::Instruction {
                package: Box::new(package.reference().clone()),
                function_key: source.function_key.clone(),
                artifact_pc: successor_pc,
            },
            "control-flow predecessors produce different frame slot counts".to_string(),
        ));
    }
    let slots = existing
        .slots
        .iter()
        .zip(&next.slots)
        .map(|(left, right)| merge_slot_state(left, right))
        .collect();
    Ok(MachineState {
        stack: existing.stack.clone(),
        slots,
        active_regions: existing.active_regions.clone(),
        writable_loans: existing.writable_loans.clone(),
    })
}

fn merge_slot_state(left: &LinkedSlotState, right: &LinkedSlotState) -> LinkedSlotState {
    // A loop header can see a slot live only on the backedge and not yet
    // initialized on entry. Keep the merged state non-live; any read or drop
    // before a write fails closed, while a write converges both paths.
    if left == right {
        return left.clone();
    }
    match (left, right) {
        (LinkedSlotState::Uninitialized, _) | (_, LinkedSlotState::Uninitialized) => {
            LinkedSlotState::Uninitialized
        }
        (LinkedSlotState::Moved, _) | (_, LinkedSlotState::Moved) => LinkedSlotState::Moved,
        (LinkedSlotState::Live(left), LinkedSlotState::Live(_)) => LinkedSlotState::Live(left.clone()),
        (LinkedSlotState::Live(_), LinkedSlotState::Live(right)) => LinkedSlotState::Live(right.clone()),
    }
}

pub(super) fn finish_stack_map(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
    source: &ValidatedFunction,
    instructions: &[LinkedInstruction],
    frame: &LinkedFrameLayout,
    mut states: BTreeMap<usize, MachineState>,
) -> Result<LinkedStackMapCandidate, BytecodeLinkError> {
    let location = function_location(package, specialization);
    if states.len() != instructions.len() {
        let unreachable = instructions
            .len()
            .checked_sub(states.len())
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "computed more program-point states than admitted instructions".to_string(),
                )
            })?;
        return Err(obligation_error(
            location,
            format!("{unreachable} admitted instructions are unreachable from the function entry"),
        ));
    }
    let entries = (0..instructions.len())
        .map(|index| {
            let state = states.remove(&index).ok_or_else(|| {
                obligation_error(
                    function_location(package, specialization),
                    format!("instruction {index} has no computed entry state"),
                )
            })?;
            let instruction = u32::try_from(index).map_err(|_| {
                obligation_error(
                    function_location(package, specialization),
                    "instruction index does not fit u32".to_string(),
                )
            })?;
            Ok(LinkedProgramPointState::new(
                InstructionIndex::new(instruction),
                state.stack.into_boxed_slice(),
                state.slots.into_boxed_slice(),
                state.active_regions.into_boxed_slice(),
                state.writable_loans.into_boxed_slice(),
            ))
        })
        .collect::<Result<Vec<_>, BytecodeLinkError>>()?;
    LinkedStackMapCandidate::try_new(
        entries.into_boxed_slice(),
        instructions.len(),
        frame.slot_types().len(),
        source.max_operand_depth,
    )
    .map_err(|error| {
        obligation_error(
            function_location(package, specialization),
            error.to_string(),
        )
    })
}
