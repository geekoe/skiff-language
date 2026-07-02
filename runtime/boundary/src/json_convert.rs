mod coerce;
mod context;
mod keys;
mod materialize;
mod numbers;
mod record;
mod runtime_json;
mod wire_decode;

use serde_json::Value;

#[cfg(any(test, feature = "test-support"))]
use crate::{contract::RuntimeBoundaryContract, json::RuntimeBoundaryCodec, plan::BoundaryUse};
use crate::{
    error::{Result, RuntimeError},
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    type_descriptor::{RuntimeTypeNode, RuntimeTypePlan},
};
#[cfg(any(test, feature = "test-support"))]
use crate::type_descriptor::RuntimeTypePlanDescriptorExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryStreamHandlePolicy {
    ExternalBoundary,
    RuntimeOwnedHandleFields,
}

impl BoundaryStreamHandlePolicy {
    fn scope(self) -> context::StreamHandleScope {
        match self {
            Self::ExternalBoundary => context::StreamHandleScope::root(),
            Self::RuntimeOwnedHandleFields => {
                context::StreamHandleScope::runtime_owned_handle_root()
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn from_wire(
    json: &Value,
    expected_type: &Value,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = descriptor_plan(expected_type)?;
    RuntimeBoundaryContract::default()
        .codec_for_expected(&plan, BoundaryUse::TypedJson, "descriptor JSON boundary")
        .from_wire_json(json, heap)
}

pub fn decode_wire_plan_impl(
    json: &Value,
    plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    stream_policy: BoundaryStreamHandlePolicy,
) -> Result<RuntimeValue> {
    wire_decode::from_wire_inner_with_stream_scope(json, plan, heap, stream_policy.scope())
}

#[cfg(any(test, feature = "test-support"))]
pub fn to_wire(
    value: &RuntimeValue,
    expected_type: &Value,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let plan = descriptor_plan(expected_type)?;
    RuntimeBoundaryContract::default()
        .codec_for_expected(&plan, BoundaryUse::TypedJson, "descriptor JSON boundary")
        .to_wire_json(value, heap)
}

pub fn encode_wire_plan_impl(
    value: &RuntimeValue,
    plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    stream_policy: BoundaryStreamHandlePolicy,
) -> Result<Value> {
    let limits = heap.limits().clone();
    let mut context = context::MaterializeContext::new(limits);
    let output =
        materialize::to_wire_inner(heap, value, plan, &mut context, stream_policy.scope(), 0)?;
    let output_bytes = materialize::serialized_json_len(&output)?;
    heap.record_materialize_stats(context.max_depth(), output_bytes)?;
    Ok(output)
}

#[cfg(any(test, feature = "test-support"))]
pub fn coerce_runtime_value(
    value: &RuntimeValue,
    expected_type: &Value,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = descriptor_plan(expected_type)?;
    RuntimeBoundaryContract::default()
        .codec_for_expected(&plan, BoundaryUse::NativeArg, "descriptor runtime boundary")
        .coerce_runtime_value(value, heap)
}

pub fn coerce_runtime_value_plan_impl(
    value: &RuntimeValue,
    plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let limits = heap.limits().clone();
    let mut context = context::RuntimeCoerceContext::new(limits);
    coerce::coerce_runtime_value_inner(value, plan, heap, &mut context, 0)
}

#[cfg(any(test, feature = "test-support"))]
pub fn to_json_runtime_value(
    value: &RuntimeValue,
    expected_type: Option<&Value>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = expected_type.map(descriptor_plan).transpose()?;
    RuntimeBoundaryCodec::to_json_runtime_value(
        value,
        plan.as_ref(),
        BoundaryUse::JsonValueProjection,
        "std.json runtime value",
        heap,
    )
}

pub fn to_json_runtime_value_plan_impl(
    value: &RuntimeValue,
    expected_type: Option<&RuntimeTypePlan>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let typed = match expected_type {
        Some(expected_type) => coerce_runtime_value_plan_impl(value, expected_type, heap)?,
        None => value.clone(),
    };
    let limits = heap.limits().clone();
    let mut context = context::RuntimeCoerceContext::new(limits);
    runtime_json::strip_runtime_value_to_json(&typed, heap, &mut context, 0)
}

#[cfg(any(test, feature = "test-support"))]
pub fn encode_json_runtime_value(
    value: &RuntimeValue,
    expected_type: Option<&Value>,
    heap: &mut RequestHeap,
) -> Result<String> {
    let plan = expected_type.map(descriptor_plan).transpose()?;
    RuntimeBoundaryCodec::encode_json_text(
        value,
        plan.as_ref(),
        BoundaryUse::JsonValueProjection,
        "std.json encode",
        heap,
    )
}

pub fn encode_json_runtime_value_plan_impl(
    value: &RuntimeValue,
    expected_type: Option<&RuntimeTypePlan>,
    heap: &mut RequestHeap,
) -> Result<String> {
    let json = match expected_type {
        Some(expected_type) if is_direct_json_runtime_plan(expected_type) => {
            let json_value = to_json_runtime_value_plan_impl(value, Some(expected_type), heap)?;
            encode_wire_plan_impl(
                &json_value,
                expected_type,
                heap,
                BoundaryStreamHandlePolicy::ExternalBoundary,
            )?
        }
        Some(expected_type) => encode_wire_plan_impl(
            value,
            expected_type,
            heap,
            BoundaryStreamHandlePolicy::ExternalBoundary,
        )?,
        None => {
            let json_value = to_json_runtime_value_plan_impl(value, None, heap)?;
            let json_plan = RuntimeTypePlan::json_value_plan();
            encode_wire_plan_impl(
                &json_value,
                &json_plan,
                heap,
                BoundaryStreamHandlePolicy::ExternalBoundary,
            )?
        }
    };
    serde_json::to_string(&json).map_err(RuntimeError::from)
}

#[cfg(any(test, feature = "test-support"))]
pub fn decode_json_text_runtime_value(
    input: &str,
    expected_type: &Value,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = descriptor_plan(expected_type)?;
    RuntimeBoundaryContract::default()
        .codec_for_expected(&plan, BoundaryUse::JsonValueProjection, "std.json decode")
        .decode_json_text(input, heap)
}

pub fn decode_json_text_runtime_value_plan_impl(
    input: &str,
    expected_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    stream_policy: BoundaryStreamHandlePolicy,
) -> Result<RuntimeValue> {
    let json = serde_json::from_str(input).map_err(|error| {
        RuntimeError::decode_target(
            "std.json.decode",
            format!("std.json.decode decode failed: {error}"),
        )
    })?;
    decode_wire_plan_impl(&json, expected_type, heap, stream_policy)
}

fn is_direct_json_runtime_plan(plan: &RuntimeTypePlan) -> bool {
    matches!(
        plan.node(),
        RuntimeTypeNode::Json | RuntimeTypeNode::JsonObject
    )
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn descriptor_plan(expected_type: &Value) -> Result<RuntimeTypePlan> {
    RuntimeTypePlan::from_descriptor(expected_type)
}

#[cfg(test)]
mod tests;
