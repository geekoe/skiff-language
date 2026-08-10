use skiff_artifact_model::{LiteralIr, TypeRefIr};
use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{
    concrete_values::prove_types_and_plans,
    tests::{
        concrete_values::{assert_types_limit, type_entry},
        fixtures::{candidate_for_concrete_types, exact_hydration_with_types, generous_limits},
    },
    VerificationLimit,
};

#[test]
fn candidate_preflight_enforces_configured_node_and_byte_limits() {
    let raw = TypeRefIr::builtin("string");
    let wide = TypeRefIr::Union {
        items: vec![
            TypeRefIr::builtin("string"),
            TypeRefIr::builtin("string"),
            TypeRefIr::builtin("string"),
        ],
    };
    let (hydrated, candidate) = mismatched_candidate(raw.clone(), wide);
    let mut limits = generous_limits();
    limits.max_value_lifecycle_nodes = 3;
    let error = prove_types_and_plans(&hydrated, &candidate, &limits).unwrap_err();
    assert_types_limit(error, VerificationLimit::ValueLifecycleNodes, 0);

    let oversized = TypeRefIr::Literal {
        value: LiteralIr::String {
            value: "x".repeat(256),
        },
    };
    let (hydrated, candidate) = mismatched_candidate(raw, oversized);
    let mut limits = generous_limits();
    limits.max_value_lifecycle_canonical_bytes = 128;
    let error = prove_types_and_plans(&hydrated, &candidate, &limits).unwrap_err();
    assert_types_limit(error, VerificationLimit::ValueLifecycleCanonicalBytes, 0);
}

fn mismatched_candidate(
    raw: TypeRefIr,
    candidate_type: TypeRefIr,
) -> (HydratedDeploymentBytecode, LinkedBytecodeCandidate) {
    let hydrated = exact_hydration_with_types(vec![raw]);
    let linked = type_entry(&hydrated, 0, candidate_type, None);
    let candidate = candidate_for_concrete_types(&hydrated, vec![linked], Vec::new())
        .expect("bounded adversarial candidate passes local construction");
    (hydrated, candidate)
}
