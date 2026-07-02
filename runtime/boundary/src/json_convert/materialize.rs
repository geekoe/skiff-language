use std::io;

use serde_json::{Map, Value};

use crate::{
    date_value,
    error::{Result, RuntimeError},
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, HeapNode, InterfaceValue, RuntimeMap, RuntimeValue},
    runtime_value_graph::RuntimeValueGraph,
    stream::is_stream_value,
    type_descriptor::{
        unresolved_type_descriptor, RuntimeRecordFieldPlan as RecordField, RuntimeTypeNode,
        RuntimeTypePlan,
    },
    value::bytes_value,
};

use super::{
    context::{MaterializeContext, StreamHandleScope, STREAM_HANDLE_SCOPE_ERROR},
    keys::wire_key_from_runtime_key_plan,
    numbers::{integer_json, number_json},
    record::{
        runtime_object_fields_from_map, RecordProjectionSource, RecordProjectionValue,
        RuntimeRecordShape,
    },
    runtime_json::reject_reserved_legacy_json_metadata_key,
};

pub(super) fn to_wire_inner(
    heap: &RequestHeap,
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    context: &mut MaterializeContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;

    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => {
            to_wire_inner(heap, value, target, context, stream_scope, depth)
        }
        RuntimeTypeNode::Nullable(inner) => {
            if matches!(value, RuntimeValue::Null) {
                Ok(Value::Null)
            } else {
                to_wire_inner(heap, value, inner, context, stream_scope, depth)
            }
        }
        RuntimeTypeNode::Union(types) => {
            to_union_wire(heap, value, types, context, stream_scope, depth)
        }
        RuntimeTypeNode::LiteralString(literal) => match value {
            RuntimeValue::String(value) if value == literal => Ok(Value::String(value.clone())),
            _ => Err(RuntimeError::Decode(format!(
                "expected runtime literal string {literal:?}"
            ))),
        },
        RuntimeTypeNode::Representation {
            payload: payload_type,
            ..
        } => to_wire_inner(
            heap,
            value,
            payload_type,
            context,
            StreamHandleScope::nested(),
            depth + 1,
        ),
        RuntimeTypeNode::Json => materialize_json_value(heap, value, context, depth),
        RuntimeTypeNode::JsonObject => materialize_json_object(heap, value, context, depth),
        RuntimeTypeNode::Bytes => materialize_bytes_value(heap, value),
        RuntimeTypeNode::Date => match value {
            RuntimeValue::Date(ms) => Ok(Value::String(date_value::format_epoch_millis(
                *ms,
                "to_wire<Date>",
            )?)),
            _ => Err(RuntimeError::Decode("expected runtime Date".to_string())),
        },
        RuntimeTypeNode::String => match value {
            RuntimeValue::String(value) => Ok(Value::String(value.clone())),
            _ => Err(RuntimeError::Decode("expected runtime string".to_string())),
        },
        RuntimeTypeNode::Bool => match value {
            RuntimeValue::Bool(value) => Ok(Value::Bool(*value)),
            _ => Err(RuntimeError::Decode("expected runtime bool".to_string())),
        },
        RuntimeTypeNode::Integer => match value {
            RuntimeValue::Number(value) => integer_json(*value),
            _ => Err(RuntimeError::Decode("expected runtime integer".to_string())),
        },
        RuntimeTypeNode::Number => match value {
            RuntimeValue::Number(value) => number_json(*value),
            _ => Err(RuntimeError::Decode("expected runtime number".to_string())),
        },
        RuntimeTypeNode::Null => match value {
            RuntimeValue::Null => Ok(Value::Null),
            _ => Err(RuntimeError::Decode("expected runtime null".to_string())),
        },
        RuntimeTypeNode::Stream(_) => {
            if !stream_scope.allows_current_node() {
                return Err(RuntimeError::Decode(STREAM_HANDLE_SCOPE_ERROR.to_string()));
            }
            let value = materialize_json_value(heap, value, context, depth)?;
            if is_stream_value(&value) {
                Ok(value)
            } else {
                Err(RuntimeError::Decode(
                    "expected runtime Stream handle".to_string(),
                ))
            }
        }
        RuntimeTypeNode::Array(item_type) => {
            materialize_array(heap, value, item_type, context, depth)
        }
        RuntimeTypeNode::Map {
            key: key_type,
            value: value_type,
        } => materialize_map(heap, value, key_type, value_type, context, depth),
        RuntimeTypeNode::Record { fields, .. } => materialize_record(
            heap,
            value,
            expected_type,
            &fields,
            context,
            stream_scope,
            depth,
        ),
        RuntimeTypeNode::Unknown => Err(unresolved_type_descriptor(expected_type)),
    }
}

fn to_union_wire(
    heap: &RequestHeap,
    value: &RuntimeValue,
    types: &[RuntimeTypePlan],
    context: &mut MaterializeContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<Value> {
    if matches!(value, RuntimeValue::Null) && types.iter().any(is_null_plan) {
        return Ok(Value::Null);
    }
    let mut errors = Vec::new();
    for ty in types {
        if is_null_plan(ty) {
            continue;
        }
        let mut branch_context = context.clone();
        match to_wire_inner(heap, value, ty, &mut branch_context, stream_scope, depth) {
            Ok(output) => {
                *context = branch_context;
                return Ok(output);
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    Err(RuntimeError::Decode(format!(
        "runtime union value did not match any branch: {}",
        errors.join("; ")
    )))
}

fn materialize_json_value(
    heap: &RequestHeap,
    value: &RuntimeValue,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    match value {
        RuntimeValue::Null => Ok(Value::Null),
        RuntimeValue::Bool(value) => Ok(Value::Bool(*value)),
        RuntimeValue::Number(value) => number_json(*value),
        RuntimeValue::Date(ms) => Ok(Value::String(date_value::format_epoch_millis(
            *ms,
            "Json Date materialize",
        )?)),
        RuntimeValue::String(value) => Ok(Value::String(value.clone())),
        RuntimeValue::ActorRef(actor_ref) => Err(RuntimeError::Decode(format!(
            "actor ref {} cannot be materialized as JSON",
            actor_ref.actor_type_identity()
        ))),
        RuntimeValue::Heap(handle) => materialize_handle(heap, *handle, context, depth + 1),
    }
}

fn materialize_bytes_value(heap: &RequestHeap, value: &RuntimeValue) -> Result<Value> {
    Ok(bytes_value(RuntimeValueGraph::new(heap).bytes(value)?))
}

fn materialize_handle(
    heap: &RequestHeap,
    handle: HeapHandle,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    context.with_active_handle(handle, |context| {
        let node = heap.get(handle)?.clone();
        match node {
            HeapNode::Bytes(bytes) => Ok(bytes_value(bytes.as_slice())),
            HeapNode::Array(items) => Ok(Value::Array(
                items
                    .iter()
                    .map(|item| materialize_json_value(heap, item, context, depth + 1))
                    .collect::<Result<Vec<_>>>()?,
            )),
            HeapNode::Object(object) => {
                let mut output = Map::new();
                for (key, value) in object.fields() {
                    reject_reserved_legacy_json_metadata_key(key)?;
                    output.insert(
                        key.clone(),
                        materialize_json_value(heap, value, context, depth + 1)?,
                    );
                }
                Ok(Value::Object(output))
            }
            HeapNode::Map(map) => materialize_runtime_map(
                heap,
                &map,
                &json_object_key_plan(),
                &RuntimeTypePlan::json_value_plan(),
                context,
                depth,
            ),
            HeapNode::Interface(value) => Err(interface_boundary_error(
                &value,
                "ordinary JSON materialization",
            )),
        }
    })
}

fn materialize_json_object(
    heap: &RequestHeap,
    value: &RuntimeValue,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode(
            "expected heap map for JsonObject".to_string(),
        ));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        match node {
            HeapNode::Map(map) => materialize_runtime_map(
                heap,
                &map,
                &json_object_key_plan(),
                &RuntimeTypePlan::json_value_plan(),
                context,
                depth + 1,
            ),
            HeapNode::Interface(value) => Err(interface_boundary_error(
                &value,
                "JsonObject materialization",
            )),
            _ => Err(RuntimeError::Decode(
                "expected heap map for JsonObject".to_string(),
            )),
        }
    })
}

fn json_object_key_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new())
}

fn materialize_array(
    heap: &RequestHeap,
    value: &RuntimeValue,
    item_type: &RuntimeTypePlan,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap array".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        let HeapNode::Array(items) = node else {
            return match node {
                HeapNode::Interface(value) => {
                    Err(interface_boundary_error(&value, "array materialization"))
                }
                _ => Err(RuntimeError::Decode("expected heap array".to_string())),
            };
        };
        Ok(Value::Array(
            items
                .iter()
                .map(|item| {
                    to_wire_inner(
                        heap,
                        item,
                        item_type,
                        context,
                        StreamHandleScope::nested(),
                        depth + 1,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        ))
    })
}

fn materialize_map(
    heap: &RequestHeap,
    value: &RuntimeValue,
    key_type: &RuntimeTypePlan,
    value_type: &RuntimeTypePlan,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap map".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        let HeapNode::Map(map) = node else {
            return match node {
                HeapNode::Interface(value) => {
                    Err(interface_boundary_error(&value, "map materialization"))
                }
                _ => Err(RuntimeError::Decode("expected heap map".to_string())),
            };
        };
        materialize_runtime_map(heap, &map, key_type, value_type, context, depth + 1)
    })
}

fn materialize_runtime_map(
    heap: &RequestHeap,
    map: &RuntimeMap,
    key_type: &RuntimeTypePlan,
    value_type: &RuntimeTypePlan,
    context: &mut MaterializeContext,
    depth: usize,
) -> Result<Value> {
    let mut output = Map::new();
    for (key, value) in map {
        let wire_key = wire_key_from_runtime_key_plan(key, key_type)?;
        reject_reserved_legacy_json_metadata_key(&wire_key)?;
        output.insert(
            wire_key,
            to_wire_inner(
                heap,
                value,
                value_type,
                context,
                StreamHandleScope::nested(),
                depth + 1,
            )?,
        );
    }
    Ok(Value::Object(output))
}

fn materialize_record(
    heap: &RequestHeap,
    value: &RuntimeValue,
    record_plan: &RuntimeTypePlan,
    fields: &[RecordField],
    context: &mut MaterializeContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<Value> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap object".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        let shape = RuntimeRecordShape::new(fields);
        let object_fields = match node {
            HeapNode::Object(object) => object.fields().clone(),
            HeapNode::Map(map) => runtime_object_fields_from_map(map)?,
            HeapNode::Interface(value) => {
                return Err(interface_boundary_error(&value, "record materialization"));
            }
            _ => return Err(RuntimeError::Decode("expected heap object".to_string())),
        };
        let projection =
            shape.project_runtime_fields(&object_fields, RecordProjectionSource::Runtime)?;
        let mut output = Map::new();
        for projected in projection.into_fields() {
            reject_reserved_legacy_json_metadata_key(&projected.field.name)?;
            let value = match projected.value {
                RecordProjectionValue::Present(field_value) => to_wire_inner(
                    heap,
                    field_value,
                    &projected.field.ty,
                    context,
                    stream_scope.record_field(record_plan, &projected.field.name),
                    depth + 1,
                )?,
                RecordProjectionValue::MissingOptionalNull => Value::Null,
            };
            output.insert(projected.field.name.clone(), value);
        }
        Ok(Value::Object(output))
    })
}

pub(super) fn serialized_json_len(value: &Value) -> Result<usize> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.len)
}

fn is_null_plan(expected_type: &RuntimeTypePlan) -> bool {
    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => is_null_plan(target),
        RuntimeTypeNode::Null => true,
        _ => false,
    }
}

fn interface_boundary_error(value: &InterfaceValue, boundary: &str) -> RuntimeError {
    RuntimeError::Decode(format!(
        "{} cannot cross {boundary}",
        value.diagnostic_label()
    ))
}

#[derive(Default)]
struct CountingWriter {
    len: usize,
}

impl io::Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.len = self.len.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
