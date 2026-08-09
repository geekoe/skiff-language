use skiff_artifact_model::{CallableEffectSummary, PackageCallableId};
use skiff_runtime_linked_bytecode::{
    CandidateTable, FunctionIndex, LinkedBytecodeCandidate, LinkedBytecodeCandidateParts,
    LinkedCallableEffectDeclaration, LinkedFrameLayout, LinkedFunction, LinkedFunctionTables,
    LinkedShapeEntry, ShapeIndex, SpecializationKey,
};

use crate::{
    verify, VerificationError, VerificationLimits, VerificationLocation, VerificationObligation,
};

fn limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: 0,
        max_total_instructions: 0,
        max_instructions_per_function: 0,
        max_frame_slots_per_function: 0,
        max_operand_depth: 0,
        max_control_flow_edges_per_function: 0,
        max_exception_regions_per_function: 0,
        max_switch_targets_per_function: 0,
        max_debug_entries_per_function: 0,
        max_image_table_entries: 0,
        max_arity: 0,
        max_callback_captures_per_callback: 0,
        max_type_nesting_depth: 0,
        max_constant_graph_edges: 0,
    }
}

fn empty_parts() -> LinkedBytecodeCandidateParts {
    LinkedBytecodeCandidateParts {
        functions: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        host_effect_adapters: Vec::new(),
        types: Vec::new(),
        shapes: Vec::new(),
        constants: Vec::new(),
        resume_sites: Vec::new(),
    }
}

fn candidate(parts: LinkedBytecodeCandidateParts) -> LinkedBytecodeCandidate {
    LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("security fixture must pass only candidate-local shape checks")
}

fn unverified_function() -> LinkedFunction {
    let callable = PackageCallableId::new("callable:unverified");
    LinkedFunction::new(
        FunctionIndex::new(0),
        SpecializationKey::new(callable.clone(), Box::new([]), None),
        Box::new([]),
        LinkedFrameLayout::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        )
        .expect("empty frame is locally well-shaped"),
        0,
        LinkedCallableEffectDeclaration::new(callable, CallableEffectSummary::analysis_pending()),
        LinkedFunctionTables::new(Box::new([]), Box::new([]), Box::new([]), Box::new([])),
    )
}

#[test]
fn completely_empty_candidate_is_the_only_currently_proved_image() {
    let image = verify(candidate(empty_parts()), &limits())
        .expect("an image with no code or linked data has no semantic execution path");

    assert!(image.functions().is_empty());
    assert!(image.candidate().exact_local_targets().is_empty());
    assert!(image.candidate().service_operations().is_empty());
    assert!(image.candidate().actor_methods().is_empty());
    assert!(image.candidate().interface_tables().is_empty());
    assert!(image.candidate().synthetic_callbacks().is_empty());
    assert!(image.candidate().host_effect_adapters().is_empty());
    assert!(image.candidate().types().is_empty());
    assert!(image.candidate().shapes().is_empty());
    assert!(image.candidate().constants().is_empty());
    assert!(image.candidate().resume_sites().is_empty());
}

#[test]
fn non_empty_program_fails_closed_before_a_verified_seal_exists() {
    let mut parts = empty_parts();
    parts.functions.push(unverified_function());

    let error = verify(candidate(parts), &limits())
        .expect_err("candidate shape checks are not semantic verification");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ControlFlow,
            location: VerificationLocation::Function {
                function: FunctionIndex::new(0),
            },
        }
    );
}

#[test]
fn non_code_linked_data_cannot_bypass_unimplemented_proofs() {
    let mut parts = empty_parts();
    parts
        .shapes
        .push(LinkedShapeEntry::new(ShapeIndex::new(0), Box::new([])));

    let error = verify(candidate(parts), &limits())
        .expect_err("unproved linked data must not receive a verified seal");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ConcreteTypeAndShape,
            location: VerificationLocation::Table {
                table: CandidateTable::Shapes,
                row: 0,
            },
        }
    );
}
