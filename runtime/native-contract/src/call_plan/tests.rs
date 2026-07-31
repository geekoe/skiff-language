use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

fn owner(type_index: usize) -> NamedUnionOwnerIdentity {
    NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
        addr: TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::loaded_file(0),
            type_index,
        },
        type_arguments: Vec::new(),
    })
}

#[test]
fn websocket_request_plan_alone_carries_exact_linked_error_owner() {
    let exact_owner = owner(7);
    let plan = NativeCallPlan::new(
        NativeBindingKey::from_static("std.websocket.requestJsonToConnection"),
        Vec::new(),
        RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
        NativeRequiredContext::Websocket,
    )
    .with_named_union_error_owner(exact_owner.clone())
    .expect("request binding should admit its linked named-union owner");

    assert_eq!(plan.named_union_error_owner(), Some(&exact_owner));

    let other = NativeCallPlan::new(
        NativeBindingKey::from_static("std.websocket.sendTextToConnection"),
        Vec::new(),
        RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
        NativeRequiredContext::Websocket,
    )
    .with_named_union_error_owner(owner(8))
    .expect_err("unrelated natives must not acquire a pseudo owner");
    assert!(other.contains("requestJsonToConnection"));
}
