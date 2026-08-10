mod cfg;
mod resume;
mod targets;
mod transfer;

use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, FrameSlotIndex, InstructionIndex, LinkedBytecodeCandidate, TypeIndex,
    WritablePathIndex,
};

use crate::{concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits};

/// Independently derived control-flow and abstract program-point facts.
///
/// Function and instruction slices use dense candidate index order. These
/// facts remain private so only the complete proof pipeline can consume them.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ControlFlowFacts {
    functions: Box<[FunctionFlowFacts]>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionFlowFacts {
    states_before: Box<[ProgramPointState]>,
    successors: Box<[Box<[ControlFlowEdge]>]>,
    exact_local_invocations: Box<[ExactLocalInvocation]>,
    computed_max_operand_depth: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExactLocalInvocation {
    site: InstructionIndex,
    target: skiff_runtime_linked_bytecode::FunctionIndex,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramPointState {
    stack: Box<[TypeIndex]>,
    slots: Box<[AbstractSlotState]>,
    active_regions: Box<[ActiveRegionIndex]>,
    writable_loans: Box<[AbstractWritableLoan]>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbstractSlotState {
    Uninitialized,
    Moved,
    Live(TypeIndex),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AbstractWritableLoan {
    root_slot: FrameSlotIndex,
    path: WritablePathIndex,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControlFlowEdge {
    target: InstructionIndex,
    kind: ControlFlowEdgeKind,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlFlowEdgeKind {
    Ordinary,
    Exceptional,
}

/// Private type-state token proving exact targets and call plans.
#[allow(dead_code)]
#[derive(Debug)]
struct ExactTargetAndCallFacts;

/// P3 orchestration seam. Opcode semantics will be read only from the
/// canonical `OpcodeContract` when these proof bodies are implemented.
pub(crate) fn prove_control_flow_and_stack(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    limits: &VerificationLimits,
) -> Result<ControlFlowFacts, VerificationError> {
    let mut facts = cfg::prove_control_flow(candidate, limits)?;
    let targets =
        targets::prove_exact_targets_and_call_plans(candidate, concrete_values, &facts, limits)?;
    transfer::prove_stack_and_slot_state(candidate, concrete_values, &targets, &mut facts, limits)?;
    resume::prove_resume_sites(candidate, concrete_values, &targets, &facts, limits)?;
    Ok(facts)
}

#[cfg(test)]
pub(crate) fn prove_control_flow_for_test(
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    cfg::prove_control_flow(candidate, limits).map(drop)
}
