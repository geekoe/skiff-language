mod constants;

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    admission::prove_admission,
    attribution::{
        prove_source_attribution, prove_statement_attribution, VerifiedStatementSchedule,
    },
    concrete_values::prove_types_and_plans,
    control_flow,
    effects::{prove_effect_and_no_pending, VerifiedCallableEffects},
    resume::VerifiedResumeSites, VerificationError, VerificationLimits,
};

#[cfg(test)]
use crate::{VerificationLocation, VerificationObligation};

pub(super) use constants::prove_and_build_constant_heap;
pub use constants::VerifiedConstantHeap;

/// Construction-only output of the independent post-link verifier.
///
/// This value contains no owner, entry map, linked program or execution
/// method. The deployment linker consumes it immediately while atomically
/// constructing the sole `DeploymentExecutionImage`; it cannot be cached or
/// used to start a VM on its own.
#[derive(Debug)]
pub struct ExecutableFacts {
    constant_heap: VerifiedConstantHeap,
    statement_schedule: VerifiedStatementSchedule,
    callable_effects: VerifiedCallableEffects,
    resume_sites: VerifiedResumeSites,
}

impl ExecutableFacts {
    #[cfg(test)]
    pub(crate) const fn constant_heap(&self) -> &VerifiedConstantHeap {
        &self.constant_heap
    }

    #[cfg(test)]
    pub(crate) const fn statement_schedule(&self) -> &VerifiedStatementSchedule {
        &self.statement_schedule
    }

    #[cfg(test)]
    pub(crate) fn function_effects(
        &self,
        function: skiff_runtime_linked_bytecode::FunctionIndex,
    ) -> Option<&crate::VerifiedFunctionEffects> {
        self.callable_effects.function(function)
    }

    #[cfg(test)]
    pub(crate) const fn resume_sites(&self) -> &VerifiedResumeSites {
        &self.resume_sites
    }

    pub fn into_parts(
        self,
    ) -> (
        VerifiedConstantHeap,
        VerifiedStatementSchedule,
        VerifiedCallableEffects,
        VerifiedResumeSites,
    ) {
        (
            self.constant_heap,
            self.statement_schedule,
            self.callable_effects,
            self.resume_sites,
        )
    }
}

/// Independently verifies one exact hydration/candidate pair and returns only
/// construction facts. The inputs remain linker-owned throughout this call;
/// no verifier-owned image or seal is minted.
pub fn verify_executable_facts(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<ExecutableFacts, VerificationError> {
    let admission = prove_admission(hydrated, candidate, limits)?;
    let concrete_values = prove_types_and_plans(hydrated, candidate, limits)?;
    let source = prove_source_attribution(candidate)?;
    let control_flow = control_flow::prove_control_flow_and_stack(
        hydrated,
        candidate,
        &concrete_values,
        admission.resume_binding(),
        &source,
        limits,
    )?;
    let statement_schedule = prove_statement_attribution(
        candidate,
        admission.statement_binding(),
        &source,
        control_flow.control_flow(),
        limits,
    )?;
    let callable_effects = prove_effect_and_no_pending(
        admission.effect_binding(),
        &control_flow,
        &statement_schedule,
    )?;
    let constant_heap = prove_and_build_constant_heap(hydrated, candidate)?;
    let resume_sites = control_flow.into_resume_sites();
    Ok(ExecutableFacts {
        constant_heap,
        statement_schedule,
        callable_effects,
        resume_sites,
    })
}

#[cfg(test)]
pub(super) fn prove_statement_schedule_for_test(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<VerifiedStatementSchedule, VerificationError> {
    let admission = prove_admission(hydrated, candidate, limits)?;
    let concrete_values = prove_types_and_plans(hydrated, candidate, limits)?;
    let source = prove_source_attribution(candidate)?;
    let control_flow = control_flow::prove_control_flow_and_stack(
        hydrated,
        candidate,
        &concrete_values,
        admission.resume_binding(),
        &source,
        limits,
    )?;
    prove_statement_attribution(
        candidate,
        admission.statement_binding(),
        &source,
        control_flow.control_flow(),
        limits,
    )
}

/// A candidate alone can never enter semantic verification because concrete
/// type resolution requires the exact admitted hydration.
#[cfg(test)]
pub(super) fn prove_candidate_semantics(
    _candidate: &LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location: VerificationLocation::Image,
    })
}
