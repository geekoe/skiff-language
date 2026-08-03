use std::collections::HashSet;

use skiff_runtime_model::{
    request_heap::RequestHeap,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
    value::{HeapHandle, HeapNode, RuntimeValue},
};

use super::codec_error;
use crate::{date_value, json, service_linkable::ServiceLinkableMaterializationError};

const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

pub(super) fn value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    plan: &RuntimeTypePlan,
) -> Result<bool, ServiceLinkableMaterializationError> {
    value_matches_plan(value, heap, plan, &mut HashSet::new())
}

fn value_matches_plan(
    value: &RuntimeValue,
    heap: &RequestHeap,
    plan: &RuntimeTypePlan,
    active: &mut HashSet<HeapHandle>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    match plan.node() {
        RuntimeTypeNode::Alias(inner) => value_matches_plan(value, heap, inner, active),
        RuntimeTypeNode::Nullable(inner) => {
            Ok(matches!(value, RuntimeValue::Null)
                || value_matches_plan(value, heap, inner, active)?)
        }
        RuntimeTypeNode::Union(variants) => {
            let mut matched = 0usize;
            for variant in variants {
                if value_matches_plan(value, heap, variant, active)? {
                    matched += 1;
                }
            }
            if matched > 1 {
                return Err(ServiceLinkableMaterializationError::AmbiguousStructuralUnion);
            }
            Ok(matched == 1)
        }
        RuntimeTypeNode::LiteralString(literal) => {
            Ok(matches!(value, RuntimeValue::String(actual) if actual == literal))
        }
        RuntimeTypeNode::Representation { payload, .. } => {
            value_matches_plan(value, heap, payload, active)
        }
        RuntimeTypeNode::Json => json_value_matches(value, heap, active),
        RuntimeTypeNode::JsonObject => json_object_matches(value, heap, active),
        RuntimeTypeNode::Bytes => {
            matches_heap_node(value, heap, |node| matches!(node, HeapNode::Bytes(_)))
        }
        RuntimeTypeNode::Date => Ok(matches!(
            value,
            RuntimeValue::Date(milliseconds) if date_value::is_valid_epoch_millis(*milliseconds)
        )),
        RuntimeTypeNode::String => Ok(matches!(value, RuntimeValue::String(_))),
        RuntimeTypeNode::Bool => Ok(matches!(value, RuntimeValue::Bool(_))),
        RuntimeTypeNode::Number => {
            Ok(matches!(value, RuntimeValue::Number(number) if number.is_finite()))
        }
        RuntimeTypeNode::Integer => Ok(matches!(
            value,
            RuntimeValue::Number(number) if is_safe_integer(*number)
        )),
        RuntimeTypeNode::Null => Ok(matches!(value, RuntimeValue::Null)),
        RuntimeTypeNode::Stream(_) | RuntimeTypeNode::TaskRef | RuntimeTypeNode::Unknown => {
            Ok(false)
        }
        RuntimeTypeNode::Array(item) => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            with_active_handle(*handle, active, |active| {
                let HeapNode::Array(items) = heap.get(*handle).map_err(model_error)? else {
                    return Ok(false);
                };
                for item_value in items {
                    if !value_matches_plan(item_value, heap, item, active)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
        }
        RuntimeTypeNode::Map {
            key: _,
            value: value_plan,
        } => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            with_active_handle(*handle, active, |active| {
                let HeapNode::Map(map) = heap.get(*handle).map_err(model_error)? else {
                    return Ok(false);
                };
                for (key, item) in map {
                    reject_runtime_field(key.string_payload())?;
                    if !value_matches_plan(item, heap, value_plan, active)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
        }
        RuntimeTypeNode::Record { fields, .. } => {
            let RuntimeValue::Heap(handle) = value else {
                return Ok(false);
            };
            with_active_handle(*handle, active, |active| {
                let HeapNode::Object(object) = heap.get(*handle).map_err(model_error)? else {
                    return Ok(false);
                };
                if object.fields().len() != fields.len() {
                    return Ok(false);
                }
                for (name, field) in object.fields() {
                    reject_runtime_field(name)?;
                    let Some(field_plan) = fields.iter().find(|field| field.name == *name) else {
                        return Ok(false);
                    };
                    if !value_matches_plan(field, heap, &field_plan.ty, active)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
        }
    }
}

fn json_value_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    active: &mut HashSet<HeapHandle>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    match value {
        RuntimeValue::Null | RuntimeValue::Bool(_) | RuntimeValue::String(_) => Ok(true),
        RuntimeValue::Number(number) => Ok(number.is_finite()),
        RuntimeValue::Date(_) | RuntimeValue::ActorRef(_) => Ok(false),
        RuntimeValue::Heap(handle) => with_active_handle(*handle, active, |active| {
            match heap.get(*handle).map_err(model_error)? {
                HeapNode::Array(items) => {
                    for item in items {
                        if !json_value_matches(item, heap, active)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                HeapNode::Map(map) => json_map_matches(map, heap, active),
                HeapNode::Object(_) | HeapNode::Bytes(_) => Ok(false),
                HeapNode::Interface(interface) => Err(
                    ServiceLinkableMaterializationError::DetachedInterfaceCarrier {
                        carrier: interface.carrier().kind_label(),
                    },
                ),
                HeapNode::Exception(_) => Ok(false),
            }
        }),
    }
}

fn json_object_matches(
    value: &RuntimeValue,
    heap: &RequestHeap,
    active: &mut HashSet<HeapHandle>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    with_active_handle(*handle, active, |active| {
        match heap.get(*handle).map_err(model_error)? {
            HeapNode::Map(map) => json_map_matches(map, heap, active),
            HeapNode::Interface(interface) => Err(
                ServiceLinkableMaterializationError::DetachedInterfaceCarrier {
                    carrier: interface.carrier().kind_label(),
                },
            ),
            _ => Ok(false),
        }
    })
}

fn json_map_matches(
    map: &skiff_runtime_model::value::RuntimeMap,
    heap: &RequestHeap,
    active: &mut HashSet<HeapHandle>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    for (key, value) in map {
        reject_runtime_field(key.string_payload())?;
        if !json_value_matches(value, heap, active)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn reject_runtime_field(field_name: &str) -> Result<(), ServiceLinkableMaterializationError> {
    json::reject_reserved_legacy_metadata_key(field_name).map_err(codec_error)
}

fn with_active_handle<T>(
    handle: HeapHandle,
    active: &mut HashSet<HeapHandle>,
    visit: impl FnOnce(&mut HashSet<HeapHandle>) -> Result<T, ServiceLinkableMaterializationError>,
) -> Result<T, ServiceLinkableMaterializationError> {
    if !active.insert(handle) {
        return Err(ServiceLinkableMaterializationError::CyclicValueGraph);
    }
    let result = visit(active);
    active.remove(&handle);
    result
}

fn matches_heap_node(
    value: &RuntimeValue,
    heap: &RequestHeap,
    predicate: impl FnOnce(&HeapNode) -> bool,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    Ok(predicate(heap.get(*handle).map_err(model_error)?))
}

fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER
}

fn model_error(
    error: skiff_runtime_model::error::RuntimeModelError,
) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::RuntimeModel {
        message: error.to_string(),
    }
}
