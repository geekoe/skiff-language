use serde_json::json;

use super::*;
use crate::type_descriptor::RuntimeTypePlanDescriptorExt;

#[test]
fn map_key_shape_uses_type_plan() {
    let representation = json!({
        "kind": "representation",
        "name": "UserId",
        "representation": { "kind": "builtin", "name": "string", "args": [] }
    });
    let expected = RuntimeMapKeyShape::PlainString;

    let plan = RuntimeTypePlan::from_descriptor(&representation).expect("map key plan");
    assert_eq!(
        RuntimeMapKeyShape::for_plan(&plan).expect("map key shape"),
        expected
    );
}

#[test]
fn map_key_shape_erases_custom_named_keys_to_plain_strings() {
    let descriptor = json!({ "kind": "builtin", "name": "UserId", "args": [] });
    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("custom key plan");
    let shape = RuntimeMapKeyShape::for_plan(&plan).expect("custom key shape");

    assert_eq!(
        shape.decode_runtime_key("u1".to_string()),
        RuntimeValueKey::string("u1")
    );
}

#[test]
fn map_key_shape_rejects_numeric_representation_payloads() {
    let descriptor = json!({
        "kind": "representation",
        "name": "NumericId",
        "representation": { "kind": "builtin", "name": "number", "args": [] }
    });

    let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("numeric key plan");
    let error =
        RuntimeMapKeyShape::for_plan(&plan).expect_err("numeric key payload should be rejected");

    assert!(error
        .to_string()
        .contains("Map key representation payload must be string"));
}
