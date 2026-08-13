mod control;
mod merge;
mod transfer;
mod values;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    contract_for_opcode, ExceptionBehavior, ExceptionRegion, TypeRefIr, ValidatedFunction,
};
use skiff_runtime_linked_bytecode::{
    LinkedConstantEntry, LinkedFrameLayout, LinkedInstruction, LinkedInstructionTarget,
    LinkedSlotState, LinkedStackMapCandidate, LinkedStackValue, LinkedSwitchTable,
    LinkedValueTransferPlan, ResumeSiteIndex, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    link::dispatch::LinkedDispatchTables,
    types::TypeLinker,
    BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct MachineState {
    stack: Vec<LinkedStackValue>,
    slots: Vec<LinkedSlotState>,
    active_regions: Vec<skiff_runtime_linked_bytecode::ActiveRegionIndex>,
    writable_loans: Vec<skiff_runtime_linked_bytecode::LinkedWritableLoanState>,
}

pub(super) struct StackMapSource<'a> {
    package: &'a HydratedBytecodePackage,
    specialization: &'a SpecializationKey,
    function: &'a ValidatedFunction,
}

impl<'a> StackMapSource<'a> {
    pub(super) const fn new(
        package: &'a HydratedBytecodePackage,
        specialization: &'a SpecializationKey,
        function: &'a ValidatedFunction,
    ) -> Self {
        Self {
            package,
            specialization,
            function,
        }
    }
}

pub(super) struct StackMapLinked<'a> {
    instructions: &'a [LinkedInstruction],
    frame: &'a LinkedFrameLayout,
    all_frames: &'a [LinkedFrameLayout],
    switch_tables: &'a [LinkedSwitchTable],
    constants: &'a [LinkedConstantEntry],
    dispatch_tables: &'a LinkedDispatchTables,
}

impl<'a> StackMapLinked<'a> {
    pub(super) fn new(
        instructions: &'a [LinkedInstruction],
        frame: &'a LinkedFrameLayout,
        all_frames: &'a [LinkedFrameLayout],
        switch_tables: &'a [LinkedSwitchTable],
        constants: &'a [LinkedConstantEntry],
        dispatch_tables: &'a LinkedDispatchTables,
    ) -> Self {
        Self {
            instructions,
            frame,
            all_frames,
            switch_tables,
            constants,
            dispatch_tables,
        }
    }
}

struct StackMapContext<'a, 'limits> {
    source: StackMapSource<'a>,
    frame: &'a LinkedFrameLayout,
    all_frames: &'a [LinkedFrameLayout],
    constants: &'a [LinkedConstantEntry],
    dispatch_tables: &'a LinkedDispatchTables,
    type_linker: &'a mut TypeLinker<'limits>,
    substitutions: &'a BTreeMap<String, TypeRefIr>,
}

pub(super) fn build_stack_map(
    source: StackMapSource<'_>,
    linked: StackMapLinked<'_>,
    type_linker: &mut TypeLinker<'_>,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Result<LinkedStackMapCandidate, BytecodeLinkError> {
    let mut context = StackMapContext {
        source,
        frame: linked.frame,
        all_frames: linked.all_frames,
        constants: linked.constants,
        dispatch_tables: linked.dispatch_tables,
        type_linker,
        substitutions,
    };
    let function_location =
        function_location(context.source.package, context.source.specialization);
    validate_instruction_coverage(
        context.source.function,
        linked.instructions,
        function_location.clone(),
    )?;
    let states = compute_states(&mut context, &linked, function_location)?;

    merge::finish_stack_map(
        context.source.package,
        context.source.specialization,
        context.source.function,
        linked.instructions,
        context.frame,
        states,
    )
}

fn compute_states(
    context: &mut StackMapContext<'_, '_>,
    linked: &StackMapLinked<'_>,
    function_location: BytecodeLinkLocation,
) -> Result<BTreeMap<usize, MachineState>, BytecodeLinkError> {
    let mut states = BTreeMap::from([(
        0usize,
        merge::initial_state(context.frame, function_location.clone())?,
    )]);
    let mut pending = BTreeSet::from([0usize]);

    while let Some(index) = pending.pop_first() {
        let state = states.get(&index).cloned().ok_or_else(|| {
            obligation_error(
                function_location.clone(),
                format!("pending instruction {index} has no entry state"),
            )
        })?;
        let successors =
            transfer_program_point(context, linked, index, state, function_location.clone())?;
        merge::merge_successors(
            context.source.package,
            context.source.function,
            successors,
            &mut states,
            &mut pending,
            function_location.clone(),
        )?;
    }
    Ok(states)
}

fn transfer_program_point(
    context: &mut StackMapContext<'_, '_>,
    linked: &StackMapLinked<'_>,
    index: usize,
    state: MachineState,
    function_location: BytecodeLinkLocation,
) -> Result<Vec<(usize, MachineState)>, BytecodeLinkError> {
    let artifact_pc = context
        .source
        .function
        .instructions
        .get(index)
        .map(|instruction| instruction.pc)
        .ok_or_else(|| {
            obligation_error(
                function_location.clone(),
                format!("decoded instruction {index} is out of bounds"),
            )
        })?;
    let instruction = linked.instructions.get(index).ok_or_else(|| {
        obligation_error(
            function_location,
            format!("linked instruction {index} is out of bounds"),
        )
    })?;
    let location = BytecodeLinkLocation::Instruction {
        package: Box::new(context.source.package.reference().clone()),
        function_key: context.source.function.function_key.clone(),
        artifact_pc,
    };
    let contract = contract_for_opcode(instruction.opcode());
    let next = transfer::apply_instruction(
        context,
        instruction,
        contract.typed,
        state.clone(),
        location.clone(),
    )?;
    check_operand_depth(&next, context.source.function.max_operand_depth, &location)?;
    if instruction.opcode() == skiff_artifact_model::Opcode::StreamNext {
        let resume_index = instruction
            .resolved_operands()
            .iter()
            .find_map(|resolved| match resolved.target() {
                LinkedInstructionTarget::ResumeSite(index) => {
                    Some(ResumeSiteIndex::new(index.get()))
                }
                _ => None,
            })
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "StreamNext resume target is absent".to_string(),
                )
            })?;
        let (resume, end_resume) = {
            let site = context
                .type_linker
                .resume_site(resume_index)
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "StreamNext resume target is absent".to_string(),
                    )
                })?;
            let end_resume = site.end_resume().ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "StreamNext resume target has no end successor".to_string(),
                )
            })?;
            (site.resume(), end_resume)
        };
        let (end_next, _) = transfer::apply_instruction_without_results(
            context,
            instruction,
            contract.typed,
            state.clone(),
            location.clone(),
        )?;
        check_operand_depth(
            &end_next,
            context.source.function.max_operand_depth,
            &location,
        )?;
        let resume_target = resume.get() as usize;
        let end_target = end_resume.get() as usize;
        if resume_target >= linked.instructions.len() || end_target >= linked.instructions.len() {
            return Err(obligation_error(
                location,
                "StreamNext successor target is out of bounds".to_string(),
            ));
        }
        let mut result = vec![(resume_target, next), (end_target, end_next)];
        if let Some(exceptional) = exceptional_successor(
            context,
            artifact_pc,
            &state,
            contract.exception.behavior,
            location.clone(),
        )? {
            result.push(exceptional);
        }
        return Ok(result);
    }
    let successors = control::successors(
        index,
        instruction,
        contract.control,
        linked.switch_tables,
        linked.instructions.len(),
        location.clone(),
    )?;
    if successors.is_empty() && !next.stack.is_empty() {
        return Err(obligation_error(
            location,
            "terminal instruction leaves values on the operand stack".to_string(),
        ));
    }
    let mut result = successors
        .into_iter()
        .map(|target| (target, next.clone()))
        .collect::<Vec<_>>();
    if let Some(exceptional) = exceptional_successor(
        context,
        artifact_pc,
        &state,
        contract.exception.behavior,
        location,
    )? {
        result.push(exceptional);
    }
    Ok(result)
}

/// The exceptional successor of one program point, mirroring the verifier's
/// `ControlFlowEdgeKind::Exceptional` contract: an instruction with non-`None`
/// exception behavior inside an exception region can hand control to the
/// innermost region's handler with its operand stack truncated to the
/// handler's declared stack height.
fn exceptional_successor(
    context: &StackMapContext<'_, '_>,
    artifact_pc: u32,
    before: &MachineState,
    behavior: ExceptionBehavior,
    location: BytecodeLinkLocation,
) -> Result<Option<(usize, MachineState)>, BytecodeLinkError> {
    if matches!(behavior, ExceptionBehavior::None) {
        return Ok(None);
    }
    let Some(region) = innermost_exception_region(
        &context.source.function.exception_regions,
        artifact_pc,
    ) else {
        return Ok(None);
    };
    let handler = context
        .source
        .function
        .instructions
        .iter()
        .position(|instruction| instruction.pc == region.handler_pc)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!(
                    "exception handler pc {} has no decoded instruction",
                    region.handler_pc
                ),
            )
        })?;
    Ok(Some((
        handler,
        exception_handler_state(context, before, region, location)?,
    )))
}

/// The innermost artifact exception region containing `pc`, in declaration
/// order (innermost last), exactly as the verifier's region lookup.
fn innermost_exception_region(
    regions: &[ExceptionRegion],
    pc: u32,
) -> Option<&ExceptionRegion> {
    regions
        .iter()
        .rev()
        .find(|region| region.start_pc <= pc && pc < region.end_pc)
}

/// The handler entry state produced by one exceptional edge, matching the
/// verifier's `exception_state`: the throw-site stack truncates to the
/// handler height, the catch slot turns `Live` with its linked frame type and
/// plan, and handler entry starts with no active regions or writable loans.
fn exception_handler_state(
    context: &StackMapContext<'_, '_>,
    before: &MachineState,
    region: &ExceptionRegion,
    location: BytecodeLinkLocation,
) -> Result<MachineState, BytecodeLinkError> {
    handler_state_from(
        before,
        region,
        context.frame.slot_types(),
        context.frame.slot_plans(),
    )
    .map_err(|detail| obligation_error(location, detail))
}

/// Pure handler-state transformation: the throw-site operand stack truncates
/// to the region's handler height and the catch slot becomes `Live` with its
/// exact linked frame type and plan. Handler entry carries no active regions
/// or writable loans, matching the verifier's exceptional-edge state.
fn handler_state_from(
    before: &MachineState,
    region: &ExceptionRegion,
    slot_types: &[TypeIndex],
    slot_plans: &[LinkedValueTransferPlan],
) -> Result<MachineState, String> {
    let height = region.handler_stack_height as usize;
    if height > before.stack.len() {
        return Err(format!(
            "exception handler stack height {height} exceeds source stack {}",
            before.stack.len()
        ));
    }
    let catch_slot = region.catch_slot as usize;
    let catch_type = slot_types
        .get(catch_slot)
        .copied()
        .ok_or_else(|| format!("exception catch slot {catch_slot} has no linked frame type"))?;
    let catch_plan = slot_plans
        .get(catch_slot)
        .cloned()
        .ok_or_else(|| format!("exception catch slot {catch_slot} has no linked frame plan"))?;
    let mut slots = before.slots.clone();
    let slot = slots
        .get_mut(catch_slot)
        .ok_or_else(|| format!("exception catch slot {catch_slot} is out of bounds"))?;
    *slot = LinkedSlotState::Live(LinkedStackValue::new(catch_type, catch_plan));
    Ok(MachineState {
        stack: before.stack[..height].to_vec(),
        slots,
        active_regions: Vec::new(),
        writable_loans: Vec::new(),
    })
}

fn check_operand_depth(
    state: &MachineState,
    max: u32,
    location: &BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if state.stack.len() > max as usize {
        return Err(obligation_error(
            location.clone(),
            format!(
                "computed operand depth {} exceeds declared max {}",
                state.stack.len(),
                max
            ),
        ));
    }
    Ok(())
}

fn validate_instruction_coverage(
    source: &ValidatedFunction,
    instructions: &[LinkedInstruction],
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if instructions.is_empty() {
        return Err(obligation_error(
            location,
            "reachable function has no instruction entry".to_string(),
        ));
    }
    if instructions.len() != source.instructions.len() {
        return Err(obligation_error(
            location,
            "linked instruction coverage differs from the admitted decoded function".to_string(),
        ));
    }
    Ok(())
}

fn function_location(
    package: &HydratedBytecodePackage,
    specialization: &SpecializationKey,
) -> BytecodeLinkLocation {
    BytecodeLinkLocation::Function {
        package: Box::new(package.reference().clone()),
        function_key: specialization.artifact_function_key().as_str().to_string(),
    }
}

fn obligation_error(location: BytecodeLinkLocation, detail: String) -> BytecodeLinkError {
    BytecodeLinkError::UnsatisfiedObligation {
        obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
        location,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_linked_bytecode::{
        LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
    };

    use super::{handler_state_from, innermost_exception_region, ExceptionRegion, LinkedSlotState,
                 LinkedStackValue, MachineState};

    fn region(start_pc: u32, end_pc: u32, handler_pc: u32, height: u32, catch_slot: u32) -> ExceptionRegion {
        ExceptionRegion {
            start_pc,
            end_pc,
            handler_pc,
            handler_stack_height: height,
            catch_matchers: Vec::new(),
            catch_slot,
            catch_slot_type_ref: catch_slot,
            cleanup_depth: 0,
        }
    }

    fn machine(stack: usize, slots: usize) -> MachineState {
        MachineState {
            stack: (0..stack)
                .map(|index| {
                    LinkedStackValue::new(
                        TypeIndex::new(index as u32),
                        LinkedValueTransferPlan::SnapshotShare {
                            drop: LinkedValueDropPlan::Trivial,
                        },
                    )
                })
                .collect(),
            slots: vec![LinkedSlotState::Uninitialized; slots],
            active_regions: Vec::new(),
            writable_loans: Vec::new(),
        }
    }

    fn plans(len: usize) -> Vec<LinkedValueTransferPlan> {
        vec![
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            };
            len
        ]
    }

    #[test]
    fn innermost_exception_region_prefers_the_deepest_containing_region() {
        let outer = region(0, 20, 20, 0, 1);
        let inner = region(4, 8, 8, 0, 2);
        let regions = vec![outer, inner];
        assert_eq!(
            innermost_exception_region(&regions, 5)
                .expect("pc 5 is inside both regions")
                .handler_pc,
            8
        );
        assert_eq!(
            innermost_exception_region(&regions, 2)
                .expect("pc 2 is inside the outer region only")
                .handler_pc,
            20
        );
        assert!(innermost_exception_region(&regions, 21).is_none());
    }

    #[test]
    fn handler_state_truncates_the_stack_and_livens_the_catch_slot() {
        let before = machine(3, 4);
        let region = region(0, 4, 4, 1, 2);
        let slot_types: Vec<TypeIndex> = (0..4).map(TypeIndex::new).collect();
        let handler = handler_state_from(&before, &region, &slot_types, &plans(4))
            .expect("handler state is well formed");
        assert_eq!(handler.stack.len(), 1);
        assert_eq!(handler.stack[0].ty(), TypeIndex::new(0));
        assert_eq!(
            handler.slots[2],
            LinkedSlotState::Live(LinkedStackValue::new(
                TypeIndex::new(2),
                LinkedValueTransferPlan::SnapshotShare {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                },
            ))
        );
        assert!(handler.active_regions.is_empty());
        assert!(handler.writable_loans.is_empty());
        assert_eq!(handler.slots[0], LinkedSlotState::Uninitialized);
    }

    #[test]
    fn handler_state_fails_closed_on_stack_and_slot_mismatches() {
        let before = machine(1, 2);
        let too_tall = region(0, 2, 2, 2, 1);
        let slot_types: Vec<TypeIndex> = (0..2).map(TypeIndex::new).collect();
        assert!(handler_state_from(&before, &too_tall, &slot_types, &plans(2)).is_err());

        let out_of_bounds = region(0, 2, 2, 0, 5);
        assert!(handler_state_from(&before, &out_of_bounds, &slot_types, &plans(2)).is_err());
    }
}
