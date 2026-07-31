use super::*;

#[test]
fn type_plan_identity_defaults_absent_for_synthetic_plans() {
    let plan = RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, vec![]);

    assert!(!plan.has_identity());
    assert_eq!(plan.catch_identity(), None);
    assert_eq!(plan.interface_identity(), None);
    assert_eq!(plan.method_projection_identity(), None);
}

#[test]
fn record_field_identity_defaults_absent() {
    let field = RuntimeRecordFieldPlan::new(
        "displayName",
        RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, vec![]),
        true,
    );

    assert_eq!(field.identity(), None);
}
