mod control;
mod merge;
mod transfer;
mod values;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{contract_for_opcode, TypeRefIr, ValidatedFunction};
use skiff_runtime_linked_bytecode::{
    LinkedConstantEntry, LinkedFrameLayout, LinkedInstruction, LinkedSlotState,
    LinkedStackMapCandidate, LinkedStackValue, LinkedSwitchTable, SpecializationKey,
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
        let (next, successors) =
            transfer_program_point(context, linked, index, state, function_location.clone())?;
        merge::merge_successors(
            context.source.package,
            context.source.function,
            successors,
            next,
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
) -> Result<(MachineState, Vec<usize>), BytecodeLinkError> {
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
        state,
        location.clone(),
    )?;
    if next.stack.len() > context.source.function.max_operand_depth as usize {
        return Err(obligation_error(
            location.clone(),
            format!(
                "computed operand depth {} exceeds declared max {}",
                next.stack.len(),
                context.source.function.max_operand_depth
            ),
        ));
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
    Ok((next, successors))
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
