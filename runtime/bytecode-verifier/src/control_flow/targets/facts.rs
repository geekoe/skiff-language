use skiff_artifact_model::{CallableEffectSummary, PackageCallableId, ParamModeIr, PendingMode};
use skiff_runtime_linked_bytecode::{
    ActorMethodIndex, CallLoanLayoutIndex, FunctionIndex, HostEffectAdapterIndex, InstructionIndex,
    InterfaceTableIndex, IntrinsicIndex, LinkedBytecodeCandidate, ResumeSiteIndex,
    ServiceOperationIndex, TypeIndex,
};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Independently proved target and call facts in dense function/instruction
/// order. A `None` row is an instruction without a callable target.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactTargetAndCallFacts {
    calls_by_function: Box<[Box<[Option<ExactCallPlan>]>]>,
}

impl ExactTargetAndCallFacts {
    pub(in crate::control_flow) fn function_count(&self) -> usize {
        self.calls_by_function.len()
    }

    /// Seals a fully populated dense table after checking that its coordinates
    /// exactly match the candidate's function and instruction layout.
    #[allow(dead_code)]
    pub(super) fn try_from_dense(
        candidate: &LinkedBytecodeCandidate,
        calls_by_function: Vec<Vec<Option<ExactCallPlan>>>,
    ) -> Result<Self, VerificationError> {
        if calls_by_function.len() != candidate.functions().len() {
            return Err(dense_violation(
                VerificationLocation::Image,
                format!(
                    "exact call facts have {} function rows, expected {}",
                    calls_by_function.len(),
                    candidate.functions().len()
                ),
            ));
        }

        for (function, instructions) in candidate.functions().iter().zip(&calls_by_function) {
            let function_location = VerificationLocation::Function {
                function: function.index(),
            };
            if instructions.len() != function.instructions().len() {
                return Err(dense_violation(
                    function_location,
                    format!(
                        "exact call facts for function {} have {} instruction rows, expected {}",
                        function.index().get(),
                        instructions.len(),
                        function.instructions().len()
                    ),
                ));
            }

            for (ordinal, plan) in instructions.iter().enumerate() {
                let Some(plan) = plan else {
                    continue;
                };
                let instruction =
                    u32::try_from(ordinal)
                        .map(InstructionIndex::new)
                        .map_err(|_| {
                            dense_violation(
                                function_location,
                                "dense instruction ordinal does not fit u32",
                            )
                        })?;
                let instruction_location = VerificationLocation::Instruction {
                    function: function.index(),
                    instruction,
                };
                if plan.call_site.function != function.index()
                    || plan.call_site.instruction != instruction
                {
                    return Err(dense_violation(
                        instruction_location,
                        "exact call plan carries a mismatched call-site coordinate",
                    ));
                }
                if plan
                    .resume
                    .is_some_and(|resume| resume.function != function.index())
                {
                    return Err(dense_violation(
                        instruction_location,
                        "exact call plan carries a resume coordinate for another function",
                    ));
                }
                if plan
                    .loan_layout
                    .is_some_and(|layout| layout.function != function.index())
                {
                    return Err(dense_violation(
                        instruction_location,
                        "exact call plan carries a loan layout for another function",
                    ));
                }
            }
        }

        Ok(Self {
            calls_by_function: calls_by_function
                .into_iter()
                .map(Vec::into_boxed_slice)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    /// Returns the exact plan at one dense call-site coordinate.
    ///
    /// Missing functions, missing instructions and non-call instructions all
    /// remain `None`; consumers must already hold the independently bounded
    /// control-flow coordinate before asking for a call plan.
    #[allow(dead_code)]
    pub(crate) fn call_plan(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Option<&ExactCallPlan> {
        usize::try_from(function.get())
            .ok()
            .and_then(|function| self.calls_by_function.get(function))
            .and_then(|instructions| {
                usize::try_from(instruction.get())
                    .ok()
                    .and_then(|instruction| instructions.get(instruction))
            })
            .and_then(Option::as_ref)
    }
}

/// One exact verified call plan. Lifecycle plans are not copied here: every
/// position retains its `TypeIndex`, whose plan is uniquely owned by the P2
/// concrete-value facts passed beside this table.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactCallPlan {
    call_site: CallSiteCoordinate,
    target: ExactTargetCoordinate,
    effect: ExactEffectFacts,
    receiver_projection: ReceiverProjection,
    parameters: Box<[ExactParameterPosition]>,
    results: Box<[ExactResultPosition]>,
    pending: PendingPlan,
    resume: Option<ResumeCoordinate>,
    loan_layout: Option<CallLoanLayoutCoordinate>,
}

impl ExactCallPlan {
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        call_site: CallSiteCoordinate,
        target: ExactTargetCoordinate,
        effect: ExactEffectFacts,
        receiver_projection: ReceiverProjection,
        parameters: Box<[ExactParameterPosition]>,
        results: Box<[ExactResultPosition]>,
        pending: PendingPlan,
        resume: Option<ResumeCoordinate>,
        loan_layout: Option<CallLoanLayoutCoordinate>,
    ) -> Self {
        Self {
            call_site,
            target,
            effect,
            receiver_projection,
            parameters,
            results,
            pending,
            resume,
            loan_layout,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn call_site(&self) -> CallSiteCoordinate {
        self.call_site
    }

    #[allow(dead_code)]
    pub(crate) const fn target(&self) -> ExactTargetCoordinate {
        self.target
    }

    #[allow(dead_code)]
    pub(crate) const fn effect(&self) -> &ExactEffectFacts {
        &self.effect
    }

    #[allow(dead_code)]
    pub(crate) const fn receiver_projection(&self) -> ReceiverProjection {
        self.receiver_projection
    }

    #[allow(dead_code)]
    pub(crate) fn parameters(&self) -> &[ExactParameterPosition] {
        &self.parameters
    }

    #[allow(dead_code)]
    pub(crate) fn results(&self) -> &[ExactResultPosition] {
        &self.results
    }

    #[allow(dead_code)]
    pub(crate) const fn pending(&self) -> PendingPlan {
        self.pending
    }

    #[allow(dead_code)]
    pub(crate) const fn resume(&self) -> Option<ResumeCoordinate> {
        self.resume
    }

    #[allow(dead_code)]
    pub(crate) const fn loan_layout(&self) -> Option<CallLoanLayoutCoordinate> {
        self.loan_layout
    }
}

/// Exact callable identity and its independently authorized effect summary.
/// Unknown summaries remain unknown for every downstream proof.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExactEffectFacts {
    canonical_callable: PackageCallableId,
    summary: CallableEffectSummary,
}

impl ExactEffectFacts {
    #[allow(dead_code)]
    pub(super) const fn new(
        canonical_callable: PackageCallableId,
        summary: CallableEffectSummary,
    ) -> Self {
        Self {
            canonical_callable,
            summary,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn canonical_callable(&self) -> &PackageCallableId {
        &self.canonical_callable
    }

    #[allow(dead_code)]
    pub(crate) const fn summary(&self) -> &CallableEffectSummary {
        &self.summary
    }
}

/// Receiver position and provenance proved for one exact call.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiverProjection {
    None,
    ExplicitSelfFirst {
        receiver: TypeIndex,
        source: ReceiverSource,
    },
}

/// Source from which an explicit first receiver is projected.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiverSource {
    OrdinaryArgument,
    InterfaceCarrier,
}

/// Exact image-local coordinate selected for a call instruction.
///
/// `LocalFunction` is the currently direct exact-local form. Typed table
/// coordinates are retained separately so later verifier work never has to
/// infer a target kind from an integer.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExactTargetCoordinate {
    LocalFunction(FunctionIndex),
    ServiceOperation(ServiceOperationIndex),
    ActorMethod(ActorMethodIndex),
    InterfaceMethod {
        table: InterfaceTableIndex,
        method_ordinal: u32,
    },
    HostEffectAdapter(HostEffectAdapterIndex),
    Intrinsic(IntrinsicIndex),
}

/// P2-backed parameter position. Its lifecycle plan is selected uniquely by
/// `ty`; the parameter mode remains an independent call-ABI fact.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactParameterPosition {
    ty: TypeIndex,
    mode: ParamModeIr,
}

impl ExactParameterPosition {
    #[allow(dead_code)]
    pub(super) const fn new(ty: TypeIndex, mode: ParamModeIr) -> Self {
        Self { ty, mode }
    }

    #[allow(dead_code)]
    pub(crate) const fn ty(&self) -> TypeIndex {
        self.ty
    }

    #[allow(dead_code)]
    pub(crate) const fn mode(&self) -> ParamModeIr {
        self.mode
    }
}

/// P2-backed result position. Its complete lifecycle plan is selected
/// uniquely by `ty` from the concrete-value facts.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExactResultPosition {
    ty: TypeIndex,
}

impl ExactResultPosition {
    #[allow(dead_code)]
    pub(super) const fn new(ty: TypeIndex) -> Self {
        Self { ty }
    }

    #[allow(dead_code)]
    pub(crate) const fn ty(&self) -> TypeIndex {
        self.ty
    }
}

/// Canonical pending classification carried by one proved call plan.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingPlan {
    Never,
    TransitiveTarget,
    RequiresNoPending,
    ActualWithResume(PendingMode),
}

/// Dense caller-local call-site coordinate.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallSiteCoordinate {
    function: FunctionIndex,
    instruction: InstructionIndex,
}

impl CallSiteCoordinate {
    #[allow(dead_code)]
    pub(super) const fn new(function: FunctionIndex, instruction: InstructionIndex) -> Self {
        Self {
            function,
            instruction,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    #[allow(dead_code)]
    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }
}

/// Exact resume descriptor and the instruction to which it resumes.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResumeCoordinate {
    function: FunctionIndex,
    descriptor: ResumeSiteIndex,
    instruction: InstructionIndex,
}

impl ResumeCoordinate {
    #[allow(dead_code)]
    pub(super) const fn new(
        function: FunctionIndex,
        descriptor: ResumeSiteIndex,
        instruction: InstructionIndex,
    ) -> Self {
        Self {
            function,
            descriptor,
            instruction,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    #[allow(dead_code)]
    pub(crate) const fn descriptor(&self) -> ResumeSiteIndex {
        self.descriptor
    }

    #[allow(dead_code)]
    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }
}

/// Function-local inout loan-layout coordinate.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallLoanLayoutCoordinate {
    function: FunctionIndex,
    layout: CallLoanLayoutIndex,
}

impl CallLoanLayoutCoordinate {
    #[allow(dead_code)]
    pub(super) const fn new(function: FunctionIndex, layout: CallLoanLayoutIndex) -> Self {
        Self { function, layout }
    }

    #[allow(dead_code)]
    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    #[allow(dead_code)]
    pub(crate) const fn layout(&self) -> CallLoanLayoutIndex {
        self.layout
    }
}

fn dense_violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}
