use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use crate::{
    admission::prove_admission, attribution::prove_source_attribution,
    concrete_values::prove_types_and_plans, control_flow::prove_control_flow_and_stack, verify,
    VerificationError, VerificationLocation, VerificationObligation,
};

use super::super::fixtures::{
    generous_limits, loader_backed_local_call, LocalCallCandidateCorruption, TARGET_FUNCTION_INDEX,
};

#[test]
fn loader_backed_local_target_authority_reaches_unknown_caller_effect() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("exact hydrated local authority must cross P3 target proof");

    assert!(matches!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Function { function },
        } if function == FunctionIndex::new(0)
    ));
}

#[test]
fn loader_backed_orchestration_retains_the_exact_call_plan() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let limits = generous_limits();
    let concrete = prove_types_and_plans(&hydrated, &candidate, &limits)
        .expect("loader-backed fixture has complete concrete value facts");
    let admission = prove_admission(&hydrated, &candidate, &limits).unwrap();
    let source = prove_source_attribution(&candidate).unwrap();
    let facts = prove_control_flow_and_stack(
        &hydrated,
        &candidate,
        &concrete,
        admission.resume_binding(),
        &source,
        &limits,
    )
    .expect("P3 orchestration must retain complete call and resume facts");

    assert!(facts.proves_exact_local_call(
        FunctionIndex::new(0),
        InstructionIndex::new(0),
        TARGET_FUNCTION_INDEX,
    ));
}
