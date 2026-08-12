use serde_json::json;
use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

use crate::runtime_value_facade::{
    RequestHeap, RuntimeTypePlan, RuntimeTypePlanDescriptorExt, RuntimeValue,
};

use super::*;

fn test_plan() -> NativeCallPlan {
    let string_plan =
        RuntimeTypePlan::from_descriptor(&json!({"kind": "builtin", "name": "string", "args": []}))
            .unwrap();
    let null_plan =
        RuntimeTypePlan::from_descriptor(&json!({"kind": "builtin", "name": "null", "args": []}))
            .unwrap();
    NativeCallPlan::new(
        NativeBindingKey::from_static("test.native"),
        vec![string_plan],
        null_plan,
        NativeRequiredContext::None,
    )
}

#[test]
fn native_adapter_materializes_args_with_native_arg_use() {
    let plan = test_plan();
    let adapter = NativeBoundaryAdapter::new("test.native", &plan);
    let mut heap = RequestHeap::default();

    let value = adapter
        .coerce_arg(0, &RuntimeValue::String("ok".to_string()), "arg", &mut heap)
        .expect("native arg should coerce");

    assert_eq!(value, RuntimeValue::String("ok".to_string()));
    let conversion = adapter
        .arg_conversion_plan(0, BoundaryDirection::Coerce)
        .expect("arg conversion plan");
    assert_eq!(conversion.use_case(), BoundaryUse::NativeArg);
    assert_eq!(conversion.direction(), BoundaryDirection::Coerce);
}

#[test]
fn native_adapter_materializes_returns_with_native_return_use() {
    let plan = test_plan();
    let adapter = NativeBoundaryAdapter::new("test.native", &plan);
    let mut heap = RequestHeap::default();

    let value = adapter
        .decode_wire_return(&Value::Null, "return", &mut heap)
        .expect("native return should materialize");

    assert_eq!(value, RuntimeValue::Null);
    let conversion = adapter.return_conversion_plan(BoundaryDirection::Decode);
    assert_eq!(conversion.use_case(), BoundaryUse::NativeReturn);
    assert_eq!(conversion.direction(), BoundaryDirection::Decode);
}
