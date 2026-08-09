use super::*;

#[test]
fn identity_and_positional_environment_are_frozen() {
    assert_eq!(
        value_lifecycle_policy_identity().fingerprint,
        VALUE_LIFECYCLE_POLICY_FINGERPRINT
    );
    assert_eq!(
        VALUE_LIFECYCLE_POLICY_FINGERPRINT,
        "2d478cdc3e2226f41b98dfa6cfbed5712b1587987e49c278f887e45486b3ec8a"
    );
    let environment = PositionalTypeEnvironment::new(
        vec!["T10".to_string(), "T2".to_string()],
        vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
    )
    .unwrap();
    let normalized = normalize_value_lifecycle_type(
        &TypeRefIr::TypeParam {
            name: "T10".to_string(),
        },
        &environment,
        &mut budget(),
    )
    .unwrap();
    assert_eq!(normalized, TypeRefIr::builtin("string"));
}

#[test]
fn every_policy_rule_group_changes_the_fingerprint() {
    let baseline = super::super::identity::policy_projection();
    let baseline_fingerprint = super::super::identity::fingerprint_projection(&baseline);
    for key in baseline.as_object().unwrap().keys() {
        let mut mutated = baseline.clone();
        mutated[key] = serde_json::Value::Null;
        assert_ne!(
            super::super::identity::fingerprint_projection(&mutated),
            baseline_fingerprint,
            "policy rule group {key} must affect the fingerprint"
        );
    }
}
