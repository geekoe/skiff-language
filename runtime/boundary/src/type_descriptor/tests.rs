use super::*;

#[test]
fn legacy_display_identity_fields_do_not_reconstruct_catch_identity() {
    let descriptor = json!({
        "kind": "builtin",
        "name": "pkg.UserView",
        "args": [],
        "identity": {
            "nominal": "type:pkg.UserView@1",
            "interface": "iface:pkg.Viewable@1",
            "union": "union:pkg.UserView@1",
            "methodProjection": "methodProjection:pkg.UserView.render@1"
        }
    });

    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("descriptor plan should build");

    assert_eq!(plan.catch_identity(), None);
    assert_eq!(plan.interface_identity(), Some("iface:pkg.Viewable@1"));
    assert_eq!(
        plan.method_projection_identity(),
        Some("methodProjection:pkg.UserView.render@1")
    );
}

#[test]
fn type_descriptor_record_field_identity_can_differ_from_display_name() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "displayName": {
                "kind": "builtin",
                "name": "string",
                "args": [],
                "fieldIdentity": "field:pkg.User.legal_name@1"
            }
        }
    });

    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("record plan should build");
    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("expected record plan");
    };

    assert_eq!(fields[0].name, "displayName");
    assert_eq!(fields[0].identity(), Some("field:pkg.User.legal_name@1"));
}

#[test]
fn legacy_union_display_identity_does_not_create_branch_identity() {
    let descriptor = json!({
        "kind": "union",
        "identity": {
            "union": "union:pkg.Result@1"
        },
        "items": [
            {
                "kind": "literal",
                "value": { "kind": "string", "value": "ok" },
                "identity": {
                    "unionBranch": "branch:pkg.Result.success@1"
                }
            },
            {
                "kind": "literal",
                "value": { "kind": "string", "value": "err" },
                "identity": {
                    "unionBranch": "branch:pkg.Result.failure@1"
                }
            }
        ]
    });

    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("union plan should build");
    let RuntimeTypeNode::Union(branches) = plan.node() else {
        panic!("expected union plan");
    };

    assert_eq!(plan.catch_identity(), None);
    assert_eq!(branches[0].catch_identity(), None);
    assert!(matches!(
        branches[0].node(),
        RuntimeTypeNode::LiteralString(value) if value == "ok"
    ));
}

#[test]
fn type_descriptor_without_identity_keeps_identity_absent() {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] }
        }
    });

    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("old descriptor should build");
    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("expected record plan");
    };

    assert!(!plan.has_identity());
    assert_eq!(fields[0].identity(), None);
}
