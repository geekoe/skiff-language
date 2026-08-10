mod checkpoint;

use skiff_artifact_model::{
    contract_for_opcode, CheckpointContract, ControlContract, OperandRole, PendingContract,
};
use skiff_runtime_linked_bytecode::{
    CandidateTable, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction,
    LinkedInstruction, LinkedInstructionTarget,
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
        .map(|function| prove_function(function, limits))
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(resume) = candidate.resume_sites().first() {
        return Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ResumeSite,
            location: VerificationLocation::Table {
                table: CandidateTable::ResumeSites,
                row: resume.index().get(),
            },
        });
    }

    Ok(ControlFlowFacts {
        functions: functions.into_boxed_slice(),
    })
}

fn prove_function(
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
    if !function.exception_regions().is_empty() || !function.active_regions().is_empty() {
        return Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExceptionRegion,
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
        let targets = normal_targets(function, instruction_index, instruction, contract.control)?;
        record_count = charge_records(
            record_count,
            targets.len(),
            limits.max_control_flow_edges_per_function,
            location,
        )?;
        successors.push(
            targets
                .into_iter()
                .map(|target| ControlFlowEdge {
                    target,
                    kind: ControlFlowEdgeKind::Ordinary,
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );

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

fn normal_targets(
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
        PendingContract::ActualWithResume { .. } => {
            return Err(VerificationError::ProofUnavailable {
                obligation: VerificationObligation::ResumeSite,
                location,
            });
        }
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
