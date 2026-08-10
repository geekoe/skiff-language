mod cfg;
mod resume;
mod targets;
mod transfer;

use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate,
    TypeIndex, WritablePathIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits};

use self::targets::ExactTargetAndCallFacts;

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
    target: FunctionIndex,
}

impl ControlFlowFacts {
    pub(crate) fn proves_function_shape(
        &self,
        function: FunctionIndex,
        instruction_count: usize,
    ) -> bool {
        self.functions
            .get(function.get() as usize)
            .is_some_and(|facts| facts.states_before.len() == instruction_count)
    }

    pub(crate) fn proves_reachable_instruction(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> bool {
        self.functions
            .get(function.get() as usize)
            .and_then(|facts| facts.states_before.get(instruction.get() as usize))
            .is_some()
    }

    /// Iterates only the exact-local invocation coordinates independently
    /// derived by the CFG proof. The dense caller ordinal is kept as `usize`
    /// so conversion into the candidate's typed index remains fallible at the
    /// P3 boundary.
    fn exact_local_invocations(&self) -> impl Iterator<Item = (usize, ExactLocalInvocation)> + '_ {
        self.functions
            .iter()
            .enumerate()
            .flat_map(|(caller, facts)| {
                facts
                    .exact_local_invocations
                    .iter()
                    .copied()
                    .map(move |invocation| (caller, invocation))
            })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgramPointState {
    stack: Box<[AbstractValue]>,
    slots: Box<[AbstractSlotState]>,
    active_regions: Box<[ActiveRegionIndex]>,
    writable_loans: Box<[AbstractWritableLoan]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbstractValue {
    Concrete(TypeIndex),
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

/// P3 orchestration seam. Opcode semantics will be read only from the
/// canonical `OpcodeContract` when these proof bodies are implemented.
pub(crate) fn prove_control_flow_and_stack(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    limits: &VerificationLimits,
) -> Result<ControlFlowFacts, VerificationError> {
    let mut facts = cfg::prove_control_flow(candidate, limits)?;
    let targets = targets::prove_exact_targets_and_call_plans(
        hydrated,
        candidate,
        concrete_values,
        &facts,
        limits,
    )?;
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

#[cfg(test)]
pub(crate) fn prove_exact_local_call_plan_for_test(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    caller: FunctionIndex,
    site: InstructionIndex,
    target: FunctionIndex,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let control_flow = cfg::prove_control_flow(candidate, limits)?;
    targets::prove_exact_local_call_plan_for_test(
        candidate,
        concrete_values,
        &control_flow,
        caller,
        site,
        target,
    )
}

#[cfg(test)]
/// Isolated P3 seam. Callers establish the candidate/P1 binding premise;
/// loader-backed `verify` tests cover that premise end to end.
pub(crate) fn prove_stack_and_slot_state_for_test(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let concrete_values =
        crate::concrete_values::prove_types_and_plans(hydrated, candidate, limits)?;
    let mut control_flow = cfg::prove_control_flow(candidate, limits)?;
    let targets = targets::prove_exact_targets_and_call_plans(
        hydrated,
        candidate,
        &concrete_values,
        &control_flow,
        limits,
    )?;
    transfer::prove_stack_and_slot_state(
        candidate,
        &concrete_values,
        &targets,
        &mut control_flow,
        limits,
    )
}
