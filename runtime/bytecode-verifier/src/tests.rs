use skiff_runtime_linked_bytecode::{
    ConstantIndex, LinkedBytecodeCandidate, LinkedBytecodeCandidateParts,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;
use skiff_runtime_model::vm_value::ValueSlot;

use crate::{
    verify_executable_facts, ExecutableFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation, VerifiedConstantHeap, VerifiedFunctionEffects,
    VerifiedStatementSchedule,
};

mod admission;
mod attribution;
mod concrete_values;
mod control_flow;
pub(crate) mod fixtures;
mod frozen_constants;
mod scalar;
mod stack_state;
mod tail_calls;
mod targets;

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
        max_statement_events_per_pc: 0,
        max_statement_events_per_function: 0,
        max_total_statement_events: 0,
        max_source_map_entries_per_function: 0,
        max_image_table_entries: 0,
        max_arity: 0,
        max_callback_captures_per_callback: 0,
        max_type_nesting_depth: 0,
        max_value_lifecycle_nodes: 0,
        max_value_lifecycle_canonical_bytes: 0,
        max_constant_graph_edges: 0,
    }
}

fn empty_candidate() -> LinkedBytecodeCandidate {
    LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
        packages: Vec::new(),
        functions: Vec::new(),
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_creates: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        callback_capture_layouts: Vec::new(),
        host_effect_adapters: Vec::new(),
        intrinsics: Vec::new(),
        types: Vec::new(),
        shapes: Vec::new(),
        constants: Vec::new(),
        constant_roots: Vec::new(),
        frozen_constant_nodes: Vec::new(),
        resume_sites: Vec::new(),
        writable_paths: Vec::new(),
    })
    .expect("empty candidate passes only local shape checks")
}

#[test]
fn verify_signature_consumes_exact_hydration_and_candidate() {
    let verify_fn: for<'a> fn(
        &'a HydratedDeploymentBytecode,
        &'a LinkedBytecodeCandidate,
        &'a VerificationLimits,
    ) -> Result<ExecutableFacts, VerificationError> = verify_executable_facts;

    let _ = verify_fn;
}

#[test]
fn verified_constant_heap_exposes_only_typed_read_access() {
    let get: fn(&VerifiedConstantHeap, ConstantIndex) -> Option<ValueSlot> =
        VerifiedConstantHeap::get;

    let _ = get;
}

#[test]
fn executable_facts_carry_only_read_access_to_the_statement_schedule() {
    let schedule: fn(&ExecutableFacts) -> &VerifiedStatementSchedule =
        ExecutableFacts::statement_schedule;

    let _ = schedule;
}

#[test]
fn executable_facts_expose_only_dense_read_access_to_function_effects() {
    let effects: fn(
        &ExecutableFacts,
        skiff_runtime_linked_bytecode::FunctionIndex,
    ) -> Option<&VerifiedFunctionEffects> = ExecutableFacts::function_effects;

    let _ = effects;
}

#[test]
fn empty_candidate_alone_cannot_mint_a_verified_seal() {
    let error = super::verifier::prove_candidate_semantics(&empty_candidate(), &limits())
        .expect_err("candidate-only proof must remain fail closed");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ConcreteTypeAndShape,
            location: VerificationLocation::Image,
        }
    );
}
