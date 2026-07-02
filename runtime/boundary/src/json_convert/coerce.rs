use crate::{
    error::{Result, RuntimeError},
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceValue, RuntimeMap, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
    stream::STREAM_ID_KEY,
    type_descriptor::{
        unresolved_type_descriptor, RuntimeRecordFieldPlan as RecordField, RuntimeTypeNode,
        RuntimeTypePlan,
    },
};

use super::{
    context::{RuntimeCoerceContext, StreamHandleScope, STREAM_HANDLE_SCOPE_ERROR},
    keys::{
        require_plain_runtime_key, runtime_key_from_wire_key_plan, wire_key_from_runtime_key_plan,
    },
    numbers::max_safe_json_integer,
    record::{
        runtime_object_fields_from_map, RecordProjectionSource, RecordProjectionValue,
        RuntimeRecordShape,
    },
    runtime_json::{reject_reserved_legacy_json_metadata_key, validate_json_runtime_value},
};

pub(super) fn coerce_runtime_value_inner(
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    coerce_runtime_value_scoped(
        value,
        expected_type,
        heap,
        context,
        StreamHandleScope::root(),
        depth,
    )
}

fn coerce_runtime_value_scoped(
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;

    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => {
            coerce_runtime_value_scoped(value, target, heap, context, stream_scope, depth)
        }
        RuntimeTypeNode::Nullable(inner) => {
            if matches!(value, RuntimeValue::Null) {
                Ok(RuntimeValue::Null)
            } else {
                coerce_runtime_value_scoped(value, inner, heap, context, stream_scope, depth)
            }
        }
        RuntimeTypeNode::Union(types) => {
            coerce_union_runtime(value, types, heap, context, stream_scope, depth)
        }
        RuntimeTypeNode::LiteralString(literal) => match value {
            RuntimeValue::String(value) if value == literal.as_str() => Ok(value.clone().into()),
            _ => Err(RuntimeError::Decode(format!(
                "expected runtime literal string {literal:?}"
            ))),
        },
        RuntimeTypeNode::Representation { payload, .. } => coerce_representation_runtime(
            value,
            payload,
            heap,
            context,
            StreamHandleScope::nested(),
            depth,
        ),
        RuntimeTypeNode::Json => {
            validate_json_runtime_value(value, heap, context, depth)?;
            Ok(value.clone())
        }
        RuntimeTypeNode::JsonObject => coerce_json_object_runtime(value, heap, context, depth),
        RuntimeTypeNode::Bytes => coerce_bytes_runtime(value, heap),
        RuntimeTypeNode::Date => match value {
            RuntimeValue::Date(ms) => Ok(RuntimeValue::Date(*ms)),
            _ => Err(RuntimeError::Decode("expected runtime Date".to_string())),
        },
        RuntimeTypeNode::String => match value {
            RuntimeValue::String(_) => Ok(value.clone()),
            _ => Err(RuntimeError::Decode("expected runtime string".to_string())),
        },
        RuntimeTypeNode::Bool => match value {
            RuntimeValue::Bool(_) => Ok(value.clone()),
            _ => Err(RuntimeError::Decode("expected runtime bool".to_string())),
        },
        RuntimeTypeNode::Integer => match value {
            RuntimeValue::Number(value) if is_safe_integer_runtime(*value) => {
                Ok(RuntimeValue::Number(*value))
            }
            RuntimeValue::Number(_) => Err(RuntimeError::Decode(
                "expected runtime safe integer".to_string(),
            )),
            _ => Err(RuntimeError::Decode("expected runtime integer".to_string())),
        },
        RuntimeTypeNode::Number => match value {
            RuntimeValue::Number(value) if value.is_finite() => Ok(RuntimeValue::Number(*value)),
            RuntimeValue::Number(_) => {
                Err(RuntimeError::Decode("number is not finite".to_string()))
            }
            _ => Err(RuntimeError::Decode("expected runtime number".to_string())),
        },
        RuntimeTypeNode::Null => match value {
            RuntimeValue::Null => Ok(RuntimeValue::Null),
            _ => Err(RuntimeError::Decode("expected runtime null".to_string())),
        },
        RuntimeTypeNode::Stream(_) => {
            if !stream_scope.allows_current_node() {
                return Err(RuntimeError::Decode(STREAM_HANDLE_SCOPE_ERROR.to_string()));
            }
            if runtime_is_stream_value(value, heap)? {
                Ok(value.clone())
            } else {
                Err(RuntimeError::Decode(
                    "expected runtime Stream handle".to_string(),
                ))
            }
        }
        RuntimeTypeNode::Array(item_type) => {
            coerce_array_runtime(value, expected_type, item_type, heap, context, depth)
        }
        RuntimeTypeNode::Map {
            key: key_type,
            value: value_type,
        } => coerce_map_runtime(
            value,
            expected_type,
            key_type,
            value_type,
            heap,
            context,
            depth,
        ),
        RuntimeTypeNode::Record { fields, .. } => coerce_record_runtime(
            value,
            expected_type,
            &fields,
            heap,
            context,
            stream_scope,
            depth,
        ),
        RuntimeTypeNode::Unknown => Err(unresolved_type_descriptor(expected_type)),
    }
}

fn coerce_union_runtime(
    value: &RuntimeValue,
    types: &[RuntimeTypePlan],
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<RuntimeValue> {
    if matches!(value, RuntimeValue::Null) && types.iter().any(is_null_plan) {
        return Ok(RuntimeValue::Null);
    }
    let mut errors = Vec::new();
    for ty in types {
        if is_null_plan(ty) {
            continue;
        }
        let checkpoint = heap.checkpoint();
        let mut branch_context = context.clone();
        match coerce_runtime_value_scoped(value, ty, heap, &mut branch_context, stream_scope, depth)
        {
            Ok(output) => {
                *context = branch_context;
                return Ok(output);
            }
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                errors.push(error.to_string());
            }
        }
    }
    Err(RuntimeError::Decode(format!(
        "runtime union value did not match any branch: {}",
        errors.join("; ")
    )))
}

fn coerce_representation_runtime(
    value: &RuntimeValue,
    payload_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<RuntimeValue> {
    coerce_runtime_value_scoped(value, payload_type, heap, context, stream_scope, depth + 1)
}

fn coerce_bytes_runtime(value: &RuntimeValue, heap: &RequestHeap) -> Result<RuntimeValue> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected runtime bytes".to_string()));
    };
    match heap.get(*handle)? {
        HeapNode::Bytes(_) => Ok(value.clone()),
        _ => Err(RuntimeError::Decode("expected runtime bytes".to_string())),
    }
}

fn coerce_json_object_runtime(
    value: &RuntimeValue,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode(
            "expected heap map for JsonObject".to_string(),
        ));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        match node {
            HeapNode::Map(map) => {
                for (key, value) in &map {
                    require_plain_runtime_key(key)?;
                    reject_reserved_legacy_json_metadata_key(key.string_payload())?;
                    validate_json_runtime_value(value, heap, context, depth + 1)?;
                }
                Ok(value.clone())
            }
            HeapNode::Object(object) => {
                let mut map = RuntimeMap::new();
                for (key, value) in object.fields() {
                    reject_reserved_legacy_json_metadata_key(key)?;
                    validate_json_runtime_value(value, heap, context, depth + 1)?;
                    map.insert(RuntimeValueKey::string(key), value.clone());
                }
                Ok(RuntimeValue::Heap(heap.alloc_map(map)?))
            }
            HeapNode::Interface(value) => Err(interface_coerce_error(
                &value,
                "JsonObject runtime coercion",
            )),
            _ => Err(RuntimeError::Decode(
                "expected heap map for JsonObject".to_string(),
            )),
        }
    })
}

fn coerce_array_runtime(
    value: &RuntimeValue,
    _expected_type: &RuntimeTypePlan,
    item_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap array".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        match node {
            HeapNode::Array(items) => {
                let mut changed = false;
                let mut output = Vec::with_capacity(items.len());
                for item in &items {
                    let coerced = coerce_runtime_value_scoped(
                        item,
                        item_type,
                        heap,
                        context,
                        StreamHandleScope::nested(),
                        depth + 1,
                    )?;
                    changed |= coerced != *item;
                    output.push(coerced);
                }
                if !changed {
                    return Ok(value.clone());
                }
                Ok(RuntimeValue::Heap(heap.alloc_array(output)?))
            }
            HeapNode::Interface(value) => {
                Err(interface_coerce_error(&value, "array runtime coercion"))
            }
            _ => Err(RuntimeError::Decode("expected heap array".to_string())),
        }
    })
}

fn coerce_map_runtime(
    value: &RuntimeValue,
    _expected_type: &RuntimeTypePlan,
    key_type: &RuntimeTypePlan,
    value_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap map".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        match node {
            HeapNode::Map(map) => {
                coerce_existing_runtime_map(value, &map, key_type, value_type, heap, context, depth)
            }
            HeapNode::Object(object) => {
                let mut map = RuntimeMap::new();
                for (key, value) in object.fields() {
                    reject_reserved_legacy_json_metadata_key(key)?;
                    let key = runtime_key_from_wire_key_plan(key, key_type)?;
                    let value = coerce_runtime_value_scoped(
                        value,
                        value_type,
                        heap,
                        context,
                        StreamHandleScope::nested(),
                        depth + 1,
                    )?;
                    map.insert(key, value);
                }
                Ok(RuntimeValue::Heap(heap.alloc_map(map)?))
            }
            HeapNode::Interface(value) => {
                Err(interface_coerce_error(&value, "map runtime coercion"))
            }
            _ => Err(RuntimeError::Decode("expected heap map".to_string())),
        }
    })
}

fn coerce_existing_runtime_map(
    original: &RuntimeValue,
    map: &RuntimeMap,
    key_type: &RuntimeTypePlan,
    value_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    let mut changed = false;
    let mut output = RuntimeMap::new();
    for (key, value) in map {
        let key_string = wire_key_from_runtime_key_plan(key, key_type)?;
        reject_reserved_legacy_json_metadata_key(&key_string)?;
        let coerced = coerce_runtime_value_scoped(
            value,
            value_type,
            heap,
            context,
            StreamHandleScope::nested(),
            depth + 1,
        )?;
        changed |= coerced != *value;
        output.insert(key.clone(), coerced);
    }
    if !changed {
        return Ok(original.clone());
    }
    Ok(RuntimeValue::Heap(heap.alloc_map(output)?))
}

fn coerce_record_runtime(
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    fields: &[RecordField],
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;
    reject_reserved_runtime_record_fields(fields)?;
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode("expected heap object".to_string()));
    };
    context.with_active_handle(*handle, |context| {
        let node = heap.get(*handle)?.clone();
        let shape = RuntimeRecordShape::for_plan(fields, expected_type.boundary_record_kind());
        match node {
            HeapNode::Object(object) => coerce_runtime_object_record(
                value,
                expected_type,
                &shape,
                RecordSourceKind::Object,
                object.fields().clone(),
                heap,
                context,
                stream_scope,
                depth,
            ),
            HeapNode::Map(map) => {
                let object_fields = runtime_object_fields_from_map(map)?;
                coerce_runtime_object_record(
                    value,
                    expected_type,
                    &shape,
                    RecordSourceKind::Map,
                    object_fields,
                    heap,
                    context,
                    stream_scope,
                    depth,
                )
            }
            HeapNode::Interface(value) => {
                Err(interface_coerce_error(&value, "record runtime coercion"))
            }
            _ => Err(RuntimeError::Decode("expected heap object".to_string())),
        }
    })
}

/// Heap representation the record value was sourced from. A `Record`-typed value
/// must always materialize as a `HeapNode::Object`; when the source is a
/// `HeapNode::Map` we cannot reuse it as-is even if no field needed coercion,
/// otherwise the coerced value would still be a `Map` and downstream object
/// accessors would reject it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordSourceKind {
    Object,
    Map,
}

fn coerce_runtime_object_record(
    original: &RuntimeValue,
    record_plan: &RuntimeTypePlan,
    shape: &RuntimeRecordShape<'_>,
    source_kind: RecordSourceKind,
    source_fields: RuntimeObjectFields,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    stream_scope: StreamHandleScope,
    depth: usize,
) -> Result<RuntimeValue> {
    reject_reserved_runtime_source_fields(&source_fields)?;
    let projection =
        shape.project_runtime_fields(&source_fields, RecordProjectionSource::Runtime)?;
    let mut changed = false;
    let mut output = RuntimeObjectFields::new();
    for projected in projection.into_fields() {
        let value = match projected.value {
            RecordProjectionValue::Present(field_value) => {
                let coerced = coerce_runtime_value_scoped(
                    field_value,
                    &projected.field.ty,
                    heap,
                    context,
                    stream_scope.record_field(record_plan, &projected.field.name),
                    depth + 1,
                )?;
                changed |= coerced != *field_value;
                coerced
            }
            RecordProjectionValue::MissingOptionalNull => {
                changed = true;
                RuntimeValue::Null
            }
        };
        output.insert(projected.field.name.clone(), value);
    }

    // Only reuse the original value untouched when it is already an object. A map
    // source must be re-materialized as an object even when its fields are
    // unchanged, since the coerced value's declared type is a record.
    if !changed && source_kind == RecordSourceKind::Object {
        return Ok(original.clone());
    }
    Ok(RuntimeValue::Heap(
        heap.alloc_object(shape.runtime_object(output))?,
    ))
}

fn reject_reserved_runtime_record_fields(fields: &[RecordField]) -> Result<()> {
    for field in fields {
        reject_reserved_legacy_json_metadata_key(&field.name)?;
    }
    Ok(())
}

fn reject_reserved_runtime_source_fields(fields: &RuntimeObjectFields) -> Result<()> {
    for key in fields.keys() {
        reject_reserved_legacy_json_metadata_key(key)?;
    }
    Ok(())
}

fn runtime_is_stream_value(value: &RuntimeValue, heap: &RequestHeap) -> Result<bool> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    Ok(match heap.get(*handle)? {
        HeapNode::Map(map) => matches!(
            map.get(&RuntimeValueKey::string(STREAM_ID_KEY)),
            Some(RuntimeValue::String(_))
        ),
        HeapNode::Object(object) => matches!(
            object.fields().get(STREAM_ID_KEY),
            Some(RuntimeValue::String(_))
        ),
        _ => false,
    })
}

fn interface_coerce_error(value: &InterfaceValue, boundary: &str) -> RuntimeError {
    RuntimeError::Decode(format!(
        "{} cannot cross {boundary}",
        value.diagnostic_label()
    ))
}

fn is_null_plan(expected_type: &RuntimeTypePlan) -> bool {
    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => is_null_plan(target),
        RuntimeTypeNode::Null => true,
        _ => false,
    }
}

fn is_safe_integer_runtime(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= max_safe_json_integer()
}
