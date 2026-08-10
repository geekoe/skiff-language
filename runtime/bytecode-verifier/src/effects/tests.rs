use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, InOutPathEffect, PackageCallableId,
    PendingEffectCategory,
};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use super::prove_effect_and_no_pending;
use crate::{
    admission::prove_admission,
    concrete_values::prove_types_and_plans,
    control_flow::prove_control_flow_and_stack,
    tests::fixtures::{
        effect_graph::{
            analyzed, bottom, loader_backed_effect_graph, EffectGraphCallKind, EffectGraphFunction,
        },
        generous_limits,
    },
    verifier::prove_statement_schedule_for_test,
    verify, VerificationError, VerificationLocation, VerificationObligation,
};

#[test]
fn analyzed_bottom_return_mints_dense_no_pending_facts() {
    let image = verify_graph(vec![function(bottom(), None)])
        .expect("analyzed bottom Return must complete the effect proof");
    let facts = image
        .function_effects(FunctionIndex::new(0))
        .expect("dense function effect facts");
    assert!(facts.no_pending());
    assert!(facts.effects().pending_effect_categories.is_empty());
}

#[test]
fn same_package_local_chain_and_self_recursion_are_post_fixed() {
    verify_graph(vec![function(bottom(), Some(1)), function(bottom(), None)])
        .expect("exact local chain must verify");
    verify_graph(vec![function(bottom(), Some(0))])
        .expect("self recursion needs no recursive verifier traversal");
}

#[test]
fn mutual_recursion_is_checked_by_local_edges() {
    verify_graph(vec![
        function(bottom(), Some(1)),
        function(bottom(), Some(0)),
    ])
    .expect("mutual recursion must satisfy the post-fixed certificate");
}

#[test]
fn unknown_never_becomes_no_pending_from_either_local_abi_value() {
    for may_suspend in [false, true] {
        let error = verify_graph(vec![EffectGraphFunction {
            summary: CallableEffectSummary::analysis_pending(),
            may_suspend,
            target: None,
            call_kind: EffectGraphCallKind::Ordinary,
        }])
        .expect_err("unknown canonical effects cannot mint a certificate");
        assert_eq!(
            error,
            VerificationError::ProofUnavailable {
                obligation: VerificationObligation::EffectAndNoPending,
                location: VerificationLocation::Function {
                    function: FunctionIndex::new(0),
                },
            }
        );
    }
}

#[test]
fn nonempty_inout_paths_remain_outside_the_first_slice() {
    let mut effects = bottom();
    effects.inout_path_effects.push(InOutPathEffect::default());
    let error = verify_graph(vec![function(effects, None)])
        .expect_err("InOut path effects are not proved by this slice");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Function {
                function: FunctionIndex::new(0),
            },
        }
    );
}

#[test]
fn every_callee_boolean_underclaim_is_rejected_at_the_call() {
    let cases = [
        (true, false, false, "escapesCallerValue"),
        (false, true, false, "requiresSameHeapIdentity"),
        (false, false, true, "invokesUnknownTarget"),
    ];
    for (escapes, same_heap, unknown_target, keyword) in cases {
        let mut callee = bottom();
        callee.escapes_caller_value = escapes;
        callee.requires_same_heap_identity = same_heap;
        callee.invokes_unknown_target = unknown_target;
        assert_call_underclaim(bottom(), callee, keyword);
    }
}

#[test]
fn callee_pending_category_underclaim_is_rejected_at_the_call() {
    let categories = [
        PendingEffectCategory::ServiceCall,
        PendingEffectCategory::ActorCall,
        PendingEffectCategory::InterfaceCall,
        PendingEffectCategory::NativeCall,
        PendingEffectCategory::Stream,
        PendingEffectCategory::HostEffect,
        PendingEffectCategory::Unknown,
    ];
    for category in categories {
        let mut callee = bottom();
        callee.may_pending = true;
        callee.pending_effect_categories = vec![category];
        assert_call_underclaim(bottom(), callee, "pending category");
    }
}

#[test]
fn conservative_caller_passes_but_no_pending_comes_from_its_canonical_categories() {
    let caller = all_effects();
    let image = verify_graph(vec![function(caller, Some(1)), function(bottom(), None)])
        .expect("caller over-approximation is sound");
    assert!(!image
        .function_effects(FunctionIndex::new(0))
        .expect("caller facts")
        .no_pending());
    assert!(image
        .function_effects(FunctionIndex::new(1))
        .expect("callee facts")
        .no_pending());
}

#[test]
fn call_plan_effect_owner_mismatch_is_rejected_defensively() {
    let functions = vec![function(bottom(), Some(1)), function(bottom(), None)];
    let (hydrated, candidate) = loader_backed_effect_graph(functions);
    let limits = generous_limits();
    let admission = prove_admission(&hydrated, &candidate, &limits).unwrap();
    let concrete = prove_types_and_plans(&hydrated, &candidate, &limits).unwrap();
    let mut control =
        prove_control_flow_and_stack(&hydrated, &candidate, &concrete, &limits).unwrap();
    let schedule = prove_statement_schedule_for_test(&hydrated, &candidate, &limits).unwrap();
    assert!(control.corrupt_call_effect_for_test(
        FunctionIndex::new(0),
        InstructionIndex::new(0),
        PackageCallableId::new("pkg-callable:corrupt"),
        analyzed(bottom()),
    ));
    let error =
        prove_effect_and_no_pending(admission.effect_binding(), &control, &schedule).unwrap_err();
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains("effect owner")
    ));
}

#[test]
fn deep_local_chain_uses_iterative_dense_scans() {
    const FUNCTION_COUNT: usize = 1024;
    let functions = (0..FUNCTION_COUNT)
        .map(|ordinal| {
            let target = (ordinal + 1 < FUNCTION_COUNT).then_some((ordinal + 1) as u32);
            function(bottom(), target)
        })
        .collect();
    let image = verify_graph(functions).expect("deep chains must not recurse in the verifier");
    assert_eq!(
        image
            .function_effects(FunctionIndex::new((FUNCTION_COUNT - 1) as u32))
            .map(|facts| facts.no_pending()),
        Some(true),
    );
}

#[test]
fn tail_call_local_still_fails_at_the_earlier_stack_gate() {
    let mut caller = function(bottom(), Some(1));
    caller.call_kind = EffectGraphCallKind::Tail;
    let error = verify_graph(vec![caller, function(bottom(), None)])
        .expect_err("TailCallLocal remains outside the stack slice");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::StackAndSlotState,
            location: VerificationLocation::Instruction {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
            },
        }
    );
}

#[test]
fn actual_with_resume_still_fails_at_the_earlier_resume_gate() {
    let mut function = function(bottom(), None);
    function.call_kind = EffectGraphCallKind::Resume;
    let error = verify_graph(vec![function])
        .expect_err("ActualWithResume remains outside the resume slice");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ResumeSite,
            location: VerificationLocation::Instruction {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
            },
        }
    );
}

#[test]
fn call_local_inout_still_fails_at_the_earlier_target_plan_gate() {
    let mut caller = function(bottom(), Some(1));
    caller.call_kind = EffectGraphCallKind::InOut;
    let error = verify_graph(vec![caller, function(bottom(), None)])
        .expect_err("CallLocalInOut remains outside the target-plan slice");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
            location: VerificationLocation::Instruction {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
            },
        }
    );
}

fn verify_graph(
    functions: Vec<EffectGraphFunction>,
) -> Result<crate::VerifiedLinkedBytecodeImage, VerificationError> {
    let (hydrated, candidate) = loader_backed_effect_graph(functions);
    verify(hydrated, candidate, &generous_limits())
}

fn function(effects: CallableMayEffects, target: Option<u32>) -> EffectGraphFunction {
    EffectGraphFunction {
        may_suspend: effects.may_pending,
        summary: analyzed(effects),
        target,
        call_kind: EffectGraphCallKind::Ordinary,
    }
}

fn assert_call_underclaim(caller: CallableMayEffects, callee: CallableMayEffects, keyword: &str) {
    let error = verify_graph(vec![function(caller, Some(1)), function(callee, None)])
        .expect_err("caller underclaim must be rejected");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains(keyword)
    ));
}

fn all_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: true,
        requires_same_heap_identity: true,
        invokes_unknown_target: true,
        may_pending: true,
        pending_effect_categories: vec![
            PendingEffectCategory::ServiceCall,
            PendingEffectCategory::ActorCall,
            PendingEffectCategory::InterfaceCall,
            PendingEffectCategory::NativeCall,
            PendingEffectCategory::Stream,
            PendingEffectCategory::HostEffect,
            PendingEffectCategory::Unknown,
        ],
        inout_path_effects: Vec::new(),
    }
}
