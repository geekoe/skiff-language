use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, InOutPathEffect, InstructionSourceSite,
    PackageCallableId, PendingEffectCategory, ResumeErrorMode, SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedValueDropPlan, LinkedValueTransferPlan,
    TypeIndex,
};

use super::prove_effect_and_no_pending;
use crate::{
    admission::prove_admission,
    attribution::prove_source_attribution,
    concrete_values::prove_types_and_plans,
    control_flow::prove_control_flow_and_stack,
    tests::fixtures::{
        effect_graph::{
            analyzed, bottom, loader_backed_effect_graph,
            loader_backed_effect_graph_with_resume_swap, EffectGraphCallKind, EffectGraphFunction,
        },
        generous_limits,
    },
    verifier::prove_statement_schedule_for_test,
    verify, VerificationError, VerificationLocation, VerificationObligation, VerifiedResumeKind,
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
            trailing_return: false,
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
    let source = prove_source_attribution(&candidate).unwrap();
    let mut control = prove_control_flow_and_stack(
        &hydrated,
        &candidate,
        &concrete,
        admission.resume_binding(),
        &source,
        &limits,
    )
    .unwrap();
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
fn tail_call_local_uses_the_exact_post_fixed_effect_edge() {
    let mut caller = function(bottom(), Some(1));
    caller.call_kind = EffectGraphCallKind::Tail;
    verify_graph(vec![caller.clone(), function(bottom(), None)])
        .expect("an exact bottom tail edge must verify");

    let mut pending = bottom();
    pending.may_pending = true;
    pending.pending_effect_categories = vec![PendingEffectCategory::ServiceCall];
    let error = verify_graph(vec![caller, function(pending.clone(), None)])
        .expect_err("a tail caller may not underclaim target pending effects");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains("pending category")
    ));

    let mut pending_caller = function(pending.clone(), Some(1));
    pending_caller.call_kind = EffectGraphCallKind::Tail;
    let image = verify_graph(vec![pending_caller, function(pending, None)])
        .expect("a post-fixed pending tail edge must verify");
    assert!(!image
        .function_effects(FunctionIndex::new(0))
        .expect("caller effect facts")
        .no_pending());
}

#[test]
fn loader_backed_tail_is_a_terminal_cfg_instruction() {
    let mut caller = function(bottom(), Some(1));
    caller.call_kind = EffectGraphCallKind::Tail;
    caller.trailing_return = true;
    let error = verify_graph(vec![caller, function(bottom(), None)])
        .expect_err("an instruction after an exact tail call must be unreachable");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ControlFlow,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(1)
            && detail.contains("unreachable")
    ));
}

#[test]
fn tail_self_and_mutual_recursion_use_iterative_post_fixed_edges() {
    let mut recursive = function(bottom(), Some(0));
    recursive.call_kind = EffectGraphCallKind::Tail;
    verify_graph(vec![recursive]).expect("tail self recursion must not recurse in the verifier");

    let mut left = function(bottom(), Some(1));
    left.call_kind = EffectGraphCallKind::Tail;
    let mut right = function(bottom(), Some(0));
    right.call_kind = EffectGraphCallKind::Tail;
    verify_graph(vec![left, right])
        .expect("mutual tail recursion must use exact local post-fixed edges");
}

#[test]
fn emit_stream_requires_a_concrete_stack_item_before_its_resume_certificate() {
    let mut function = function(bottom(), None);
    function.call_kind = EffectGraphCallKind::Resume;
    let error = verify_graph(vec![function])
        .expect_err("EmitStream must consume a concrete FunctionStreamItem");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::StackAndSlotState,
            location,
            detail,
        } if location == VerificationLocation::Instruction {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
            }
            && detail.contains("operand-stack underflow")
    ));
}

#[test]
fn stream_next_stream_read_mints_a_nonempty_resume_certificate() {
    let image = verify_graph(vec![stream_function(stream_effects())])
        .expect("StreamNext/StreamRead must verify from exact stream authority");
    let [resume] = image.resume_sites().rows() else {
        panic!("exactly one resume certificate was expected")
    };
    assert_eq!(resume.function(), FunctionIndex::new(0));
    assert_eq!(resume.site(), InstructionIndex::new(0));
    assert_eq!(resume.resume(), InstructionIndex::new(1));
    assert_eq!(resume.expected_stack_height_before_result(), 0);
    assert_eq!(resume.result_type(), TypeIndex::new(0));
    assert_eq!(
        resume.result_plan(),
        &LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }
    );
    assert_eq!(resume.error_mode(), ResumeErrorMode::RaiseAtSite);
    assert_eq!(
        resume.original_site(),
        &InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
        }
    );
    assert_eq!(
        resume.kind(),
        &VerifiedResumeKind::StreamRead {
            endpoint_slot: FrameSlotIndex::new(0),
            item_type: TypeIndex::new(0),
        }
    );
    assert!(!image
        .function_effects(FunctionIndex::new(0))
        .expect("stream function effects")
        .no_pending());
}

#[test]
fn reachable_stream_next_cannot_mint_no_pending() {
    let error = verify_graph(vec![stream_function(bottom())])
        .expect_err("StreamNext requires canonical Stream pending authority");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains("Stream pending")
    ));
}

#[test]
fn swapped_resume_targets_fail_at_exact_hydration_binding() {
    let mut function = stream_function(stream_effects());
    function.call_kind = EffectGraphCallKind::StreamReadTwice;
    let functions = vec![function];
    let (hydrated, candidate) = loader_backed_effect_graph_with_resume_swap(functions);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("raw descriptor and typed resume row may not be swapped");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Instruction { function, instruction },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains("exact descriptor")
    ));
}

#[test]
fn repeated_stream_read_preserves_the_nonempty_stack_prefix() {
    let mut function = stream_function(stream_effects());
    function.call_kind = EffectGraphCallKind::StreamReadTwice;
    let image = verify_graph(vec![function])
        .expect("two exact stream reads with a retained prefix must verify");
    let [first, second] = image.resume_sites().rows() else {
        panic!("two resume certificates were expected")
    };
    assert_eq!(first.expected_stack_height_before_result(), 0);
    assert_eq!(second.site(), InstructionIndex::new(1));
    assert_eq!(second.resume(), InstructionIndex::new(2));
    assert_eq!(second.expected_stack_height_before_result(), 1);
    assert_eq!(second.result_type(), TypeIndex::new(0));
    assert_eq!(
        second.kind(),
        &VerifiedResumeKind::StreamRead {
            endpoint_slot: FrameSlotIndex::new(0),
            item_type: TypeIndex::new(0),
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
        trailing_return: false,
    }
}

fn stream_function(effects: CallableMayEffects) -> EffectGraphFunction {
    let mut function = function(effects, None);
    function.call_kind = EffectGraphCallKind::StreamRead;
    function
}

fn stream_effects() -> CallableMayEffects {
    let mut effects = bottom();
    effects.may_pending = true;
    effects.pending_effect_categories = vec![PendingEffectCategory::Stream];
    effects
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
