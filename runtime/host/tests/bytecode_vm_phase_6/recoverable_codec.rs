use skiff_runtime_boundary::{
    binary::{decode_recoverable_payload_plan, encode_recoverable_payload_plan},
    payload::PayloadBoundary,
    type_descriptor::RuntimeTypePlanDescriptorExt,
};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(&serde_json::json!({
        "kind": "builtin",
        "name": "string",
        "args": [],
    }))
    .expect("production runtime type descriptor should build")
}

#[test]
fn recoverable_owner_internal_plain_roundtrip() {
    let plan = string_plan();
    let boundary = PayloadBoundary::runtime_internal();
    let bytes = encode_recoverable_payload_plan(
        &RuntimeValue::String("recoverable-ok".to_string()),
        &plan,
        &boundary,
        &RequestHeap::default(),
    )
    .expect("owner-internal recoverable codec should encode plain logical values");
    let decoded =
        decode_recoverable_payload_plan(&bytes, &plan, &boundary, &mut RequestHeap::default())
            .expect("owner-internal recoverable codec should decode plain logical values");
    assert_eq!(decoded, RuntimeValue::String("recoverable-ok".to_string()));
}

#[test]
fn recoverable_partial_decode_fails_closed() {
    let plan = string_plan();
    let boundary = PayloadBoundary::runtime_internal();
    let mut heap = RequestHeap::default();
    let error =
        decode_recoverable_payload_plan(b"not a recoverable envelope", &plan, &boundary, &mut heap)
            .expect_err("partial or malformed recoverable bytes must fail closed");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("recoverable")
            || error.to_string().to_ascii_lowercase().contains("decode"),
        "unexpected recoverable decode diagnostic: {error}"
    );
}
