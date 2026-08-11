mod cfg;
mod resume;
mod tail;
mod targets;
mod transfer;

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedBytecodeCandidate,
    TypeIndex, WritablePathIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    admission::ExactResumeBinding,
    attribution::SourceAttributionFacts,
    resume::{VerifiedResumeKind, VerifiedResumeSites},
};
use crate::{concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits};

use self::targets::ExactTargetAndCallFacts;
pub(crate) use self::targets::{ExactCallPlan, ExactTargetCoordinate, PendingPlan};

/// Complete P3 orchestration result. Later gates consume these proof products
/// without rescanning linked target declarations or inferring resume absence.
#[derive(Debug)]
pub(crate) struct ControlFlowAndCallFacts {
    control_flow: ControlFlowFacts,
    exact_targets: ExactTargetAndCallFacts,
    tail_calls: tail::VerifiedTailCallFacts,
    instructions: VerifiedInstructionFacts,
    resume_sites: VerifiedResumeSites,
    stream_read_sites: Box<[(FunctionIndex, InstructionIndex)]>,
}

impl ControlFlowAndCallFacts {
    pub(crate) const fn control_flow(&self) -> &ControlFlowFacts {
        &self.control_flow
    }

    pub(crate) fn function_counts(&self) -> (usize, usize) {
        (
            self.control_flow.functions.len(),
            self.exact_targets.function_count(),
        )
    }

    pub(crate) fn exact_call_plan(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<&ExactCallPlan> {
        self.exact_targets.call_plan(function, instruction)
    }

    pub(crate) fn proved_tail_call_target(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<FunctionIndex> {
        self.tail_calls
            .proof(function, instruction)
            .map(tail::VerifiedTailCallProof::target)
    }

    pub(crate) fn instruction_rows(&self) -> &[VerifiedFunctionInstructions] {
        &self.instructions.functions
    }

    pub(crate) fn proves_stream_read(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> bool {
        self.stream_read_sites
            .binary_search(&(function, instruction))
            .is_ok()
    }

    pub(crate) fn proves_pending_resume(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        mode: skiff_artifact_model::PendingMode,
    ) -> bool {
        self.resume_sites.rows().iter().any(|row| {
            row.function() == function
                && row.site() == instruction
                && matches!(
                    (mode, row.kind()),
                    (
                        skiff_artifact_model::PendingMode::StreamRead,
                        crate::resume::VerifiedResumeKind::StreamRead { .. }
                    ) | (
                        skiff_artifact_model::PendingMode::StreamBackpressure,
                        crate::resume::VerifiedResumeKind::StreamBackpressure
                    ) | (
                        skiff_artifact_model::PendingMode::ServiceBoundary,
                        crate::resume::VerifiedResumeKind::ServiceBoundary
                    ) | (
                        skiff_artifact_model::PendingMode::ActorBoundary,
                        crate::resume::VerifiedResumeKind::ActorBoundary
                    ) | (
                        skiff_artifact_model::PendingMode::InterfaceBoundary,
                        crate::resume::VerifiedResumeKind::InterfaceBoundary
                    ) | (
                        skiff_artifact_model::PendingMode::CallbackBoundary,
                        crate::resume::VerifiedResumeKind::CallbackBoundary
                    ) | (
                        skiff_artifact_model::PendingMode::HostEffect,
                        crate::resume::VerifiedResumeKind::HostEffect
                    )
                )
        })
    }

    pub(crate) fn into_resume_sites(self) -> VerifiedResumeSites {
        self.resume_sites
    }

    #[cfg(test)]
    pub(crate) fn corrupt_call_effect_for_test(
        &mut self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        canonical_callable: skiff_artifact_model::PackageCallableId,
        summary: skiff_artifact_model::CallableEffectSummary,
    ) -> bool {
        self.exact_targets.corrupt_effect_for_test(
            function,
            instruction,
            canonical_callable,
            summary,
        )
    }

    #[cfg(test)]
    pub(crate) fn proves_exact_local_call(
        &self,
        caller: FunctionIndex,
        site: InstructionIndex,
        target: FunctionIndex,
    ) -> bool {
        self.exact_targets
            .call_plan(caller, site)
            .is_some_and(|plan| {
                plan.target() == targets::ExactTargetCoordinate::LocalFunction(target)
            })
    }
}

/// Dense opcode snapshot sealed only after the complete P3 proof succeeds.
#[derive(Debug)]
struct VerifiedInstructionFacts {
    functions: Box<[VerifiedFunctionInstructions]>,
}

/// One dense function's verified opcode sequence. Callable operands remain
/// exclusively authoritative through `ExactTargetAndCallFacts`.
#[derive(Debug)]
pub(crate) struct VerifiedFunctionInstructions {
    function: FunctionIndex,
    opcodes: Box<[Opcode]>,
}

impl VerifiedFunctionInstructions {
    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) fn opcodes(&self) -> &[Opcode] {
        &self.opcodes
    }
}

impl VerifiedInstructionFacts {
    fn try_from_candidate(
        candidate: &LinkedBytecodeCandidate,
        control_flow: &ControlFlowFacts,
    ) -> Result<Self, VerificationError> {
        let functions = candidate
            .functions()
            .iter()
            .enumerate()
            .map(|(ordinal, function)| {
                let expected = u32::try_from(ordinal)
                    .map(FunctionIndex::new)
                    .map_err(|_| {
                        instruction_snapshot_violation(
                            crate::VerificationLocation::Image,
                            "dense opcode function ordinal does not fit u32",
                        )
                    })?;
                let location = crate::VerificationLocation::Function { function: expected };
                if function.index() != expected
                    || !control_flow.proves_function_shape(expected, function.instructions().len())
                {
                    return Err(instruction_snapshot_violation(
                        location,
                        "dense opcode snapshot disagrees with proved CFG shape",
                    ));
                }
                Ok(VerifiedFunctionInstructions {
                    function: expected,
                    opcodes: function
                        .instructions()
                        .iter()
                        .map(|instruction| instruction.opcode())
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            functions: functions.into_boxed_slice(),
        })
    }
}

fn instruction_snapshot_violation(
    location: crate::VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: crate::VerificationObligation::ControlFlow,
        location,
        detail: detail.into(),
    }
}

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
    Exceptional { region: usize },
}

/// P3 orchestration seam. Opcode semantics will be read only from the
/// canonical `OpcodeContract` when these proof bodies are implemented.
pub(crate) fn prove_control_flow_and_stack(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    exact_resumes: &ExactResumeBinding,
    source: &SourceAttributionFacts,
    limits: &VerificationLimits,
) -> Result<ControlFlowAndCallFacts, VerificationError> {
    let mut facts = cfg::prove_control_flow(candidate, limits)?;
    let resume_preflight = resume::prove_contracts(candidate, exact_resumes)?;
    let targets = targets::prove_exact_targets_and_call_plans(
        hydrated,
        candidate,
        concrete_values,
        &facts,
        limits,
    )?;
    let tail_calls = transfer::prove_stack_and_slot_state(
        candidate,
        concrete_values,
        &targets,
        &mut facts,
        limits,
    )?;
    let resume_sites = resume::prove_resume_states(
        candidate,
        exact_resumes,
        &resume_preflight,
        concrete_values,
        &facts,
        &targets,
        source,
    )?;
    let stream_read_sites = resume_sites
        .rows()
        .iter()
        .filter_map(|row| {
            matches!(row.kind(), VerifiedResumeKind::StreamRead { .. })
                .then_some((row.function(), row.site()))
        })
        .collect::<Vec<_>>();
    let mut unique_stream_read_sites = stream_read_sites.clone();
    unique_stream_read_sites.sort_unstable();
    unique_stream_read_sites.dedup();
    if unique_stream_read_sites.len() != stream_read_sites.len() {
        return Err(VerificationError::SemanticViolation {
            obligation: crate::VerificationObligation::ResumeSite,
            location: crate::VerificationLocation::Image,
            detail: "resume certificates do not have unique stream-read coordinates".to_string(),
        });
    }
    let instructions = VerifiedInstructionFacts::try_from_candidate(candidate, &facts)?;
    Ok(ControlFlowAndCallFacts {
        control_flow: facts,
        exact_targets: targets,
        tail_calls,
        instructions,
        resume_sites,
        stream_read_sites: unique_stream_read_sites.into_boxed_slice(),
    })
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
    .map(drop)
}
