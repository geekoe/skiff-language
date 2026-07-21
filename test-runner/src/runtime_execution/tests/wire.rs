use super::super::test_support::*;
use super::*;

#[test]
fn activation_receipt_and_canonical_pending_decode_strictly() {
    let receipt = decode_activation_receipt(&activation_receipt_body()).unwrap();
    assert_eq!(receipt.environment, ENVIRONMENT);
    assert_eq!(receipt.generation, 2);
    assert_eq!(receipt.assembly.assembly_identity.as_str(), ASSEMBLY_B);

    let health = decode_health_snapshot(&health_body(
        ENVIRONMENT,
        2,
        ASSEMBLY_B,
        valid_pending(),
        vec![replica(2, ASSEMBLY_B, "healthy", true)],
        vec![capability(REPLICA, true)],
    ))
    .unwrap();
    assert!(health.pending_activation);
}

#[test]
fn pending_token_generation_and_assembly_mutations_fail_closed() {
    assert_pending_mutations_fail(vec![
        (
            "invalid activation token",
            pending(
                Value::String("bad token".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a"]),
            ),
        ),
        (
            "non-string activation token",
            pending(
                serde_json::json!(7),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a"]),
            ),
        ),
        (
            "expected generation mismatch",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(1),
                serde_json::json!(2),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a"]),
            ),
        ),
        (
            "candidate generation mismatch",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(4),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a"]),
            ),
        ),
        (
            "non-canonical generation number",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2.0),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a"]),
            ),
        ),
        (
            "invalid assembly identity",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                "invalid-assembly",
                serde_json::json!(["runtime-a"]),
            ),
        ),
    ]);
}

#[test]
fn pending_candidate_must_remain_within_the_safe_generation_range() {
    const MAX_SAFE_GENERATION: u64 = 9_007_199_254_740_991;
    let pending_state = pending(
        Value::String("activation-max".to_string()),
        serde_json::json!(MAX_SAFE_GENERATION),
        serde_json::json!(MAX_SAFE_GENERATION + 1),
        ASSEMBLY_A,
        serde_json::json!(["runtime-a"]),
    );

    let result = decode_health_snapshot(&health_body(
        ENVIRONMENT,
        MAX_SAFE_GENERATION,
        ASSEMBLY_B,
        pending_state,
        Vec::new(),
        Vec::new(),
    ));

    assert!(result.is_err(), "unsafe pending generation was accepted");
}

#[test]
fn pending_participant_invariant_mutations_fail_closed() {
    assert_pending_mutations_fail(vec![
        (
            "empty participants",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!([]),
            ),
        ),
        (
            "duplicate participants",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime-a", "runtime-a"]),
            ),
        ),
        (
            "unsorted participants",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime-b", "runtime-a"]),
            ),
        ),
        (
            "invalid participant token",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!(["runtime a"]),
            ),
        ),
        (
            "non-string participant",
            pending(
                Value::String("activation-three".to_string()),
                serde_json::json!(2),
                serde_json::json!(3),
                ASSEMBLY_A,
                serde_json::json!([1]),
            ),
        ),
    ]);
}

#[test]
fn pending_exact_shape_mutations_fail_closed() {
    let mut unknown = valid_pending();
    unknown.as_object_mut().unwrap().insert(
        "legacyBuildId".to_string(),
        Value::String("legacy".to_string()),
    );
    let mut missing = valid_pending();
    missing
        .as_object_mut()
        .unwrap()
        .remove("candidateGeneration");
    assert_pending_mutations_fail(vec![
        ("unknown pending field", unknown),
        ("missing pending field", missing),
    ]);
}

fn assert_pending_mutations_fail(cases: Vec<(&'static str, Value)>) {
    for (name, pending) in cases {
        let result = decode_health_snapshot(&health_body(
            ENVIRONMENT,
            2,
            ASSEMBLY_B,
            pending,
            Vec::new(),
            Vec::new(),
        ));
        assert!(result.is_err(), "mutation {name} was accepted");
    }
}

#[test]
fn activation_state_safe_generation_and_identity_mutations_fail_closed() {
    for (name, generation, assembly, environment) in [
        (
            "unsafe generation",
            9_007_199_254_740_992,
            ASSEMBLY_B,
            ENVIRONMENT,
        ),
        (
            "invalid active assembly",
            2,
            "invalid-assembly",
            ENVIRONMENT,
        ),
        ("invalid environment", 2, ASSEMBLY_B, "../prod"),
    ] {
        let result = decode_health_snapshot(&health_body(
            environment,
            generation,
            assembly,
            Value::Null,
            Vec::new(),
            Vec::new(),
        ));
        assert!(result.is_err(), "mutation {name} was accepted");
    }
}

#[test]
fn health_unknown_missing_and_wrong_typed_fields_fail_closed() {
    let valid = || {
        serde_json::from_str::<Value>(&health_body(
            ENVIRONMENT,
            2,
            ASSEMBLY_B,
            Value::Null,
            vec![replica(2, ASSEMBLY_B, "healthy", true)],
            vec![capability(REPLICA, true)],
        ))
        .unwrap()
    };
    let mut unknown = valid();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("legacy".to_string(), Value::Bool(true));
    let mut missing = valid();
    missing.as_object_mut().unwrap().remove("replicas");
    let mut wrong_type = valid();
    wrong_type["replicas"][0]["connected"] = Value::String("true".to_string());

    for (name, value) in [
        ("unknown", unknown),
        ("missing", missing),
        ("wrong type", wrong_type),
    ] {
        assert!(
            decode_health_snapshot(&value.to_string()).is_err(),
            "{name} schema mutation was accepted"
        );
    }
}
