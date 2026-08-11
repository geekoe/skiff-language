mod checkpoint;

use skiff_artifact_model::{
    contract_for_opcode, CheckpointContract, ControlContract, ExceptionBehavior, ExceptionContract,
    OperandRole, PendingContract,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction, LinkedInstruction,
    LinkedInstructionTarget,
};

use super::{
    ControlFlowEdge, ControlFlowEdgeKind, ControlFlowFacts, ExactLocalInvocation,
    FunctionFlowFacts, ProgramPointState,
};
use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

/// Independently derives bounded CFG successors for every linked function.
///
pub(super) fn prove_control_flow(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<ControlFlowFacts, VerificationError> {
    let functions = candidate
        .functions()
        .iter()
        .map(|function| prove_function(candidate, function, limits))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ControlFlowFacts {
        functions: functions.into_boxed_slice(),
    })
}

fn prove_function(
    candidate: &LinkedBytecodeCandidate,
    function: &LinkedFunction,
    limits: &VerificationLimits,
) -> Result<FunctionFlowFacts, VerificationError> {
    let function_location = VerificationLocation::Function {
        function: function.index(),
    };
    if function.instructions().is_empty() {
        return Err(semantic_violation(
            VerificationObligation::ControlFlow,
            function_location,
            "function has no entry instruction",
        ));
    }
    if function.exception_regions().len() as u64 > limits.max_exception_regions_per_function {
        return Err(VerificationError::LimitExceeded {
            limit: VerificationLimit::ExceptionRegionsPerFunction,
            actual: function.exception_regions().len() as u64,
            max: limits.max_exception_regions_per_function,
            location: function_location,
        });
    }

    let mut successors = Vec::with_capacity(function.instructions().len());
    let mut exact_local_invocations = Vec::new();
    let mut checkpoints = Vec::with_capacity(function.instructions().len());
    let mut record_count = 0_u64;

    for (ordinal, instruction) in function.instructions().iter().enumerate() {
        let instruction_index = instruction_index(function, ordinal)?;
        let location = instruction_location(function, instruction_index);
        let contract = contract_for_opcode(instruction.opcode());
        let normal_targets = normal_targets(
            candidate,
            function,
            instruction_index,
            instruction,
            contract.control,
        )?;
        let exception_targets = exception_targets(function, instruction_index, contract.exception)?;
        let mut targets = normal_targets
            .into_iter()
            .map(|target| ControlFlowEdge {
                target,
                kind: ControlFlowEdgeKind::Ordinary,
            })
            .collect::<Vec<_>>();
        targets.extend(
            exception_targets
                .into_iter()
                .map(|(target, region)| ControlFlowEdge {
                    target,
                    kind: ControlFlowEdgeKind::Exceptional { region },
                }),
        );
        record_count = charge_records(
            record_count,
            targets.len(),
            limits.max_control_flow_edges_per_function,
            location,
        )?;
        successors.push(targets.into_boxed_slice());

        if let Some(target) = exact_local_target(instruction, contract.pending, location)? {
            record_count = charge_records(
                record_count,
                1,
                limits.max_control_flow_edges_per_function,
                location,
            )?;
            exact_local_invocations.push(ExactLocalInvocation {
                site: instruction_index,
                target,
            });
        }
        checkpoints.push(matches!(
            contract.checkpoint,
            CheckpointContract::Budget { .. }
        ));
    }

    prove_reachable(function, &successors)?;
    let cycle = checkpoint::first_cycle_without_checkpoint(&successors, &checkpoints).map_err(
        |detail| {
            semantic_violation(
                VerificationObligation::ControlFlow,
                function_location,
                detail,
            )
        },
    )?;
    if let Some(ordinal) = cycle {
        return Err(semantic_violation(
            VerificationObligation::BudgetCheckpoint,
            instruction_location(function, instruction_index(function, ordinal)?),
            "ordinary control-flow cycle does not pass through a budget checkpoint",
        ));
    }

    let empty_state = ProgramPointState {
        stack: Box::new([]),
        slots: Box::new([]),
        active_regions: Box::new([]),
        writable_loans: Box::new([]),
    };
    Ok(FunctionFlowFacts {
        states_before: vec![empty_state; function.instructions().len()].into_boxed_slice(),
        successors: successors.into_boxed_slice(),
        exact_local_invocations: exact_local_invocations.into_boxed_slice(),
        computed_max_operand_depth: 0,
    })
}

fn exception_targets(
    function: &LinkedFunction,
    instruction_index: InstructionIndex,
    exception: ExceptionContract,
) -> Result<Vec<(InstructionIndex, usize)>, VerificationError> {
    if matches!(exception.behavior, ExceptionBehavior::None) {
        return Ok(Vec::new());
    }
    let Some((region, index)) = innermost_exception_region(function, instruction_index) else {
        return Ok(Vec::new());
    };
    Ok(vec![(region.handler(), index)])
}

fn innermost_exception_region(
    function: &LinkedFunction,
    instruction_index: InstructionIndex,
) -> Option<(&skiff_runtime_linked_bytecode::LinkedExceptionRegion, usize)> {
    function
        .exception_regions()
        .iter()
        .enumerate()
        .rev()
        .find(|(_, region)| {
            region.start().get() <= instruction_index.get()
                && instruction_index.get() < region.end().get()
        })
        .map(|(index, region)| (region, index))
}

fn normal_targets(
    candidate: &LinkedBytecodeCandidate,
    function: &LinkedFunction,
    instruction_index: InstructionIndex,
    instruction: &LinkedInstruction,
    control: ControlContract,
) -> Result<Vec<InstructionIndex>, VerificationError> {
    let location = instruction_location(function, instruction_index);
    let mut targets = match control {
        ControlContract::Fallthrough => vec![fallthrough(function, instruction_index)?],
        ControlContract::Jump { target } => {
            vec![branch_target(instruction, target, location)?]
        }
        ControlContract::Branch { target, .. } => vec![
            branch_target(instruction, target, location)?,
            fallthrough(function, instruction_index)?,
        ],
        ControlContract::Switch { table } => {
            let table_index = switch_table_target(instruction, table, location)?;
            let Some(table) = function.switch_tables().get(table_index) else {
                return Err(semantic_violation(
                    VerificationObligation::ControlFlow,
                    location,
                    "switch table target is out of bounds",
                ));
            };
            table
                .cases()
                .iter()
                .map(|case| case.target())
                .chain(std::iter::once(table.default_target()))
                .collect()
        }
        ControlContract::Return
        | ControlContract::TailCall
        | ControlContract::Raise
        | ControlContract::Rethrow => Vec::new(),
    };
    if instruction.opcode() == skiff_artifact_model::Opcode::StreamNext {
        let resume = resume_target(instruction, location)?;
        let row = candidate
            .resume_sites()
            .get(resume.get() as usize)
            .filter(|row| row.index() == resume)
            .ok_or_else(|| {
                semantic_violation(
                    VerificationObligation::ControlFlow,
                    location,
                    "StreamNext resume target is absent from the linked table",
                )
            })?;
        let end_resume = row.end_resume().ok_or_else(|| {
            semantic_violation(
                VerificationObligation::ControlFlow,
                location,
                "StreamNext lacks an end-resume CFG successor",
            )
        })?;
        targets.push(end_resume);
    }
    for target in &targets {
        if target.get() as usize >= function.instructions().len() {
            return Err(semantic_violation(
                VerificationObligation::ControlFlow,
                location,
                "ordinary control-flow target is out of bounds",
            ));
        }
    }
    targets.sort_unstable_by_key(|target| target.get());
    targets.dedup();
    Ok(targets)
}

fn exact_local_target(
    instruction: &LinkedInstruction,
    pending: PendingContract,
    location: VerificationLocation,
) -> Result<Option<FunctionIndex>, VerificationError> {
    let role = match pending {
        PendingContract::Never => return Ok(None),
        PendingContract::TransitiveTarget { target }
        | PendingContract::NoPendingTarget { target, .. } => target,
        PendingContract::ActualWithResume { .. } => return Ok(None),
    };
    match resolved_target(instruction, role, location)? {
        LinkedInstructionTarget::Function(target) => Ok(Some(target)),
        _ => Err(semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "exact-local invocation did not resolve to a function",
        )),
    }
}

fn branch_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<InstructionIndex, VerificationError> {
    match resolved_target(instruction, role, location)? {
        LinkedInstructionTarget::Branch(target) => Ok(target),
        _ => Err(semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "branch role did not resolve to an instruction",
        )),
    }
}

fn resume_target(
    instruction: &LinkedInstruction,
    location: VerificationLocation,
) -> Result<skiff_runtime_linked_bytecode::ResumeSiteIndex, VerificationError> {
    match resolved_target(instruction, OperandRole::ResumeRef, location)? {
        LinkedInstructionTarget::ResumeSite(target) => Ok(target),
        _ => Err(semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "resume role did not resolve to a resume site",
        )),
    }
}

fn switch_table_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<usize, VerificationError> {
    match resolved_target(instruction, role, location)? {
        LinkedInstructionTarget::SwitchTable(target) => Ok(target.get() as usize),
        _ => Err(semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "switch role did not resolve to a switch table",
        )),
    }
}

fn resolved_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<LinkedInstructionTarget, VerificationError> {
    let contract = contract_for_opcode(instruction.opcode());
    let ordinal = contract.operand_position(role).ok_or_else(|| {
        semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "canonical control-flow role is absent from the opcode operands",
        )
    })?;
    let ordinal = u32::try_from(ordinal).map_err(|_| {
        semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "control-flow operand ordinal does not fit u32",
        )
    })?;
    instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| {
            semantic_violation(
                VerificationObligation::ControlFlow,
                location,
                "control-flow operand has no typed resolved target",
            )
        })
}

fn fallthrough(
    function: &LinkedFunction,
    instruction: InstructionIndex,
) -> Result<InstructionIndex, VerificationError> {
    let location = instruction_location(function, instruction);
    let next = instruction.get().checked_add(1).ok_or_else(|| {
        semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "fallthrough instruction index overflowed u32",
        )
    })?;
    if next as usize >= function.instructions().len() {
        return Err(semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "final instruction requires a fallthrough successor",
        ));
    }
    Ok(InstructionIndex::new(next))
}

fn prove_reachable(
    function: &LinkedFunction,
    successors: &[Box<[ControlFlowEdge]>],
) -> Result<(), VerificationError> {
    let mut reachable = vec![false; successors.len()];
    let mut work = vec![0_usize];
    reachable[0] = true;
    while let Some(source) = work.pop() {
        for edge in &successors[source] {
            let target = edge.target.get() as usize;
            if !reachable[target] {
                reachable[target] = true;
                work.push(target);
            }
        }
    }
    if let Some(unreachable) = reachable.iter().position(|reachable| !reachable) {
        return Err(semantic_violation(
            VerificationObligation::ControlFlow,
            instruction_location(function, instruction_index(function, unreachable)?),
            "instruction is unreachable from function entry 0",
        ));
    }
    Ok(())
}

fn charge_records(
    current: u64,
    additional: usize,
    max: u64,
    location: VerificationLocation,
) -> Result<u64, VerificationError> {
    let additional = u64::try_from(additional).map_err(|_| {
        semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "control-flow record count does not fit u64",
        )
    })?;
    let actual = current.checked_add(additional).ok_or_else(|| {
        semantic_violation(
            VerificationObligation::ControlFlow,
            location,
            "control-flow record count overflowed u64",
        )
    })?;
    if actual > max {
        return Err(VerificationError::LimitExceeded {
            limit: VerificationLimit::ControlFlowEdgesPerFunction,
            actual,
            max,
            location,
        });
    }
    Ok(actual)
}

fn instruction_index(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<InstructionIndex, VerificationError> {
    u32::try_from(ordinal)
        .map(InstructionIndex::new)
        .map_err(|_| {
            semantic_violation(
                VerificationObligation::ControlFlow,
                VerificationLocation::Function {
                    function: function.index(),
                },
                "dense instruction ordinal does not fit u32",
            )
        })
}

const fn instruction_location(
    function: &LinkedFunction,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: function.index(),
        instruction,
    }
}

fn semantic_violation(
    obligation: VerificationObligation,
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation,
        location,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{Opcode, PackageCallableId};
    use skiff_runtime_linked_bytecode::{
        ArtifactFunctionKey, FrameSlotIndex, LinkedCallableEffectDeclaration, LinkedCatchMatcher,
        LinkedExceptionRegion, LinkedFrameLayout, LinkedFunctionTables, LinkedProgramPointState,
        LinkedSlotState, LinkedStackMapCandidate, LinkedValueDropPlan, LinkedValueTransferPlan,
        SpecializationKey, TypeIndex,
    };

    use super::*;

    #[test]
    fn throw_and_rethrow_get_their_innermost_handler_edge() {
        let function = function_with_region();
        let throw_contract = contract_for_opcode(Opcode::Throw);
        let edges = exception_targets(
            &function,
            InstructionIndex::new(0),
            throw_contract.exception,
        )
        .unwrap();
        assert_eq!(edges, vec![(InstructionIndex::new(1), 0)]);

        let rethrow_contract = contract_for_opcode(Opcode::Rethrow);
        let edges = exception_targets(
            &function,
            InstructionIndex::new(0),
            rethrow_contract.exception,
        )
        .unwrap();
        assert_eq!(edges, vec![(InstructionIndex::new(1), 0)]);
    }

    fn function_with_region() -> LinkedFunction {
        let slot_types = Box::new([TypeIndex::new(0)]);
        let slot_plans = Box::new([LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }]);
        let frame = LinkedFrameLayout::new(
            slot_types,
            Box::new([]),
            Box::new([]),
            Box::new([]),
            slot_plans,
            Box::new([]),
            None,
        )
        .unwrap();
        let region = LinkedExceptionRegion::new(
            InstructionIndex::new(0),
            skiff_runtime_linked_bytecode::InstructionBoundaryIndex::new(2),
            InstructionIndex::new(1),
            0,
            Box::new([LinkedCatchMatcher::CatchAll]),
            FrameSlotIndex::new(0),
            TypeIndex::new(0),
            0,
        );
        let tables = LinkedFunctionTables::new(
            Box::new([region]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        );
        let states = (0..2)
            .map(|index| {
                LinkedProgramPointState::new(
                    InstructionIndex::new(index),
                    Box::new([]),
                    Box::new([LinkedSlotState::Uninitialized]),
                    Box::new([]),
                    Box::new([]),
                )
            })
            .collect::<Vec<_>>();
        let stack_map =
            LinkedStackMapCandidate::try_new(states.into_boxed_slice(), 2, 1, 0).unwrap();
        LinkedFunction::new(
            FunctionIndex::new(0),
            SpecializationKey::new(
                skiff_artifact_model::PackageBuildId::new("cfg-test"),
                ArtifactFunctionKey::parse("module::cfg").unwrap(),
                PackageCallableId::new("cfg"),
                Box::new([]),
                None,
            ),
            Box::new([
                LinkedInstruction::new(
                    Opcode::Throw,
                    Box::new([0]),
                    Box::new([skiff_runtime_linked_bytecode::LinkedResolvedOperand::new(
                        0,
                        LinkedInstructionTarget::Type(TypeIndex::new(0)),
                    )]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(Opcode::Return, Box::new([]), Box::new([]), 1).unwrap(),
            ]),
            frame,
            0,
            LinkedCallableEffectDeclaration::new(
                PackageCallableId::new("cfg"),
                skiff_artifact_model::CallableEffectSummary::analysis_pending(),
            ),
            tables,
            stack_map,
        )
    }
}
