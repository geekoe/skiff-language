//! Closed builtin receiver method dispatch for RuntimeProgram values.

use skiff_runtime_boundary::{date_value, value as boundary_bytes};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueKey},
};

use crate::error::{Result, RuntimeError};
use skiff_artifact_model::{BuiltinReceiverMethod, BuiltinReceiverOp, BuiltinReceiverRoot};
use skiff_runtime_linked_program::TypeAddr;

use super::{
    invocation::EvalProgramProjection,
    mutable_path::{
        apply_collection_mutation, map_key_from_runtime_value, CollectionMutation,
        CollectionMutationResult,
    },
    program_mutation::{program_mutable_receiver_handle, runtime_u64},
    runtime_ops::{
        runtime_array_items, runtime_debug_value_for_error, runtime_deep_clone, runtime_map_get,
        runtime_map_has, runtime_number_value, runtime_numeric,
    },
    runtime_value_view::RuntimeValueView,
};

pub struct ReceiverMethodDispatch<'a> {
    heap: &'a mut RequestHeap,
}

impl<'a> ReceiverMethodDispatch<'a> {
    pub fn new(heap: &'a mut RequestHeap) -> Self {
        Self { heap }
    }

    pub fn dispatch_op(
        &mut self,
        op: &BuiltinReceiverOp,
        receiver: RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        skiff_artifact_model::validate_supported_receiver_builtin_op(op).map_err(|error| {
            RuntimeError::InvalidArtifact(format!(
                "unsupported receiver builtin op {}: {}",
                op.canonical_key, error
            ))
        })?;
        let value = match op.receiver {
            BuiltinReceiverRoot::Array => {
                ArrayReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::Map => {
                MapReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::JsonObject => JsonObjectReceiverMethods::dispatch(
                op.method,
                &receiver,
                args.as_slice(),
                self.heap,
            )?,
            BuiltinReceiverRoot::StringText => {
                StringReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::Number => {
                NumberReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::Date => {
                DateReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::Duration => {
                DurationReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
            BuiltinReceiverRoot::Bytes => {
                BytesReceiverMethods::dispatch(op.method, &receiver, args.as_slice(), self.heap)?
            }
        };
        value.ok_or_else(|| {
            RuntimeError::Decode(format!(
                "receiver builtin {} is not valid for value {}",
                op.canonical_key,
                runtime_debug_value_for_error(&receiver, self.heap)
            ))
        })
    }
}

pub fn canonical_type_addr(
    program: EvalProgramProjection<'_>,
    addr: &TypeAddr,
) -> Result<TypeAddr> {
    program.canonical_type_addr(addr)
}

struct ArrayReceiverMethods;

impl ArrayReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Length
                | BuiltinReceiverMethod::Push
                | BuiltinReceiverMethod::Set
                | BuiltinReceiverMethod::Pop
                | BuiltinReceiverMethod::Clone
        ) {
            return Ok(None);
        }
        let Some(items) = runtime_array_items(receiver, heap)? else {
            return Ok(None);
        };

        match op_method {
            BuiltinReceiverMethod::Length => Ok(Some(RuntimeValue::Number(items.len() as f64))),
            BuiltinReceiverMethod::Push => {
                let item = args.first().cloned().unwrap_or(RuntimeValue::Null);
                let handle = program_mutable_receiver_handle(receiver, heap, "Array.push")?;
                apply_collection_mutation(heap, handle, CollectionMutation::ArrayPush(item))?;
                Ok(Some(RuntimeValue::Null))
            }
            BuiltinReceiverMethod::Set => {
                let index = args.first().and_then(runtime_u64).ok_or_else(|| {
                    RuntimeError::Decode(
                        "Array.set index must be a non-negative number".to_string(),
                    )
                })?;
                let item = args.get(1).cloned().unwrap_or(RuntimeValue::Null);
                let handle = program_mutable_receiver_handle(receiver, heap, "Array.set")?;
                apply_collection_mutation(
                    heap,
                    handle,
                    CollectionMutation::ArraySet {
                        index: index as usize,
                        value: item,
                    },
                )?;
                Ok(Some(RuntimeValue::Null))
            }
            BuiltinReceiverMethod::Pop => {
                let handle = program_mutable_receiver_handle(receiver, heap, "Array.pop")?;
                match apply_collection_mutation(heap, handle, CollectionMutation::ArrayPop)? {
                    CollectionMutationResult::Value(value) => Ok(Some(value)),
                    CollectionMutationResult::Unit | CollectionMutationResult::Existed(_) => {
                        Err(RuntimeError::Decode(
                            "Array.pop returned invalid mutation result".to_string(),
                        ))
                    }
                }
            }
            BuiltinReceiverMethod::Clone => runtime_deep_clone(receiver, heap).map(Some),
            _ => Ok(None),
        }
    }
}

struct MapReceiverMethods;

impl MapReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Length
                | BuiltinReceiverMethod::Get
                | BuiltinReceiverMethod::Has
                | BuiltinReceiverMethod::Set
                | BuiltinReceiverMethod::Delete
                | BuiltinReceiverMethod::Keys
                | BuiltinReceiverMethod::Clone
        ) {
            return Ok(None);
        }
        if !is_heap_map(receiver, heap)? {
            return Ok(None);
        }

        match op_method {
            BuiltinReceiverMethod::Length => Ok(Some(RuntimeValue::Number(
                RuntimeValueView::new(receiver, heap).map_like_len()? as f64,
            ))),
            BuiltinReceiverMethod::Get => {
                let key = args.first().unwrap_or(&RuntimeValue::Null);
                runtime_map_get(receiver, key, heap).map(Some)
            }
            BuiltinReceiverMethod::Has => {
                let key = args.first().unwrap_or(&RuntimeValue::Null);
                Ok(Some(RuntimeValue::Bool(runtime_map_has(
                    receiver, key, heap,
                )?)))
            }
            BuiltinReceiverMethod::Set => {
                if args.len() < 2 {
                    return Err(RuntimeError::Decode(
                        "Map.set requires key and value".to_string(),
                    ));
                }
                let key = map_key_from_runtime_value(&args[0], heap)?;
                let value = args.get(1).cloned().unwrap_or(RuntimeValue::Null);
                let handle = program_mutable_receiver_handle(receiver, heap, "Map.set")?;
                apply_collection_mutation(heap, handle, CollectionMutation::MapSet { key, value })?;
                Ok(Some(RuntimeValue::Null))
            }
            BuiltinReceiverMethod::Delete => {
                let key =
                    map_key_from_runtime_value(args.first().unwrap_or(&RuntimeValue::Null), heap)?;
                let handle = program_mutable_receiver_handle(receiver, heap, "Map.delete")?;
                match apply_collection_mutation(
                    heap,
                    handle,
                    CollectionMutation::MapDelete { key },
                )? {
                    CollectionMutationResult::Existed(existed) => {
                        Ok(Some(RuntimeValue::Bool(existed)))
                    }
                    CollectionMutationResult::Unit | CollectionMutationResult::Value(_) => {
                        Err(RuntimeError::Decode(
                            "Map.delete returned invalid mutation result".to_string(),
                        ))
                    }
                }
            }
            BuiltinReceiverMethod::Keys => {
                let keys = match receiver {
                    RuntimeValue::Heap(handle) => match heap.get(*handle)? {
                        HeapNode::Map(map) => map.keys().map(runtime_value_from_map_key).collect(),
                        HeapNode::Interface(value) => {
                            return Err(RuntimeError::Decode(format!(
                                "{} is not a Map receiver",
                                value.diagnostic_label()
                            )));
                        }
                        _ => return Ok(None),
                    },
                    _ => return Ok(None),
                };
                Ok(Some(RuntimeValue::Heap(heap.alloc_array(keys)?)))
            }
            BuiltinReceiverMethod::Clone => runtime_deep_clone(receiver, heap).map(Some),
            _ => Ok(None),
        }
    }
}

fn is_heap_map(receiver: &RuntimeValue, heap: &RequestHeap) -> Result<bool> {
    let RuntimeValue::Heap(handle) = receiver else {
        return Ok(false);
    };
    match heap.get(*handle)? {
        HeapNode::Map(_) => Ok(true),
        HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
            "{} is not a Map receiver",
            value.diagnostic_label()
        ))),
        _ => Ok(false),
    }
}

struct JsonObjectReceiverMethods;

impl JsonObjectReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Length
                | BuiltinReceiverMethod::Get
                | BuiltinReceiverMethod::Has
                | BuiltinReceiverMethod::Set
                | BuiltinReceiverMethod::Delete
                | BuiltinReceiverMethod::Clone
        ) {
            return Ok(None);
        }
        if !RuntimeValueView::new(receiver, heap).is_map_like()? {
            return Ok(None);
        }
        match op_method {
            BuiltinReceiverMethod::Length => Ok(Some(RuntimeValue::Number(
                RuntimeValueView::new(receiver, heap).map_like_len()? as f64,
            ))),
            BuiltinReceiverMethod::Get => Ok(Some(
                RuntimeValueView::new(receiver, heap)
                    .map_get(args.first().unwrap_or(&RuntimeValue::Null))?,
            )),
            BuiltinReceiverMethod::Has => Ok(Some(RuntimeValue::Bool(
                RuntimeValueView::new(receiver, heap)
                    .map_has(args.first().unwrap_or(&RuntimeValue::Null))?,
            ))),
            BuiltinReceiverMethod::Set => {
                MapReceiverMethods::dispatch(BuiltinReceiverMethod::Set, receiver, args, heap)
            }
            BuiltinReceiverMethod::Delete => {
                MapReceiverMethods::dispatch(BuiltinReceiverMethod::Delete, receiver, args, heap)
            }
            BuiltinReceiverMethod::Clone => runtime_deep_clone(receiver, heap).map(Some),
            _ => Ok(None),
        }
    }
}

fn runtime_value_from_map_key(key: &RuntimeValueKey) -> RuntimeValue {
    match key {
        RuntimeValueKey::String(value) => RuntimeValue::String(value.clone()),
    }
}

fn runtime_string(value: &RuntimeValue) -> Option<&str> {
    match value {
        RuntimeValue::String(value) => Some(value.as_str()),
        _ => None,
    }
}

struct StringReceiverMethods;

impl StringReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Length
                | BuiltinReceiverMethod::Contains
                | BuiltinReceiverMethod::ReplaceAll
                | BuiltinReceiverMethod::Concat
                | BuiltinReceiverMethod::StartsWith
                | BuiltinReceiverMethod::EndsWith
                | BuiltinReceiverMethod::Lowercase
        ) {
            return Ok(None);
        }
        let Some(value) = RuntimeValueView::new(receiver, heap).string_payload()? else {
            return Ok(None);
        };

        match op_method {
            BuiltinReceiverMethod::Length => {
                Ok(Some(RuntimeValue::Number(value.chars().count() as f64)))
            }
            BuiltinReceiverMethod::Contains => {
                let needle = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("string.contains needle must be a string".to_string())
                })?;
                Ok(Some(RuntimeValue::Bool(value.contains(needle))))
            }
            BuiltinReceiverMethod::ReplaceAll => {
                let needle = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("string.replaceAll needle must be a string".to_string())
                })?;
                let replacement = args.get(1).and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode(
                        "string.replaceAll replacement must be a string".to_string(),
                    )
                })?;
                Ok(Some(RuntimeValue::String(
                    value.replace(needle, replacement),
                )))
            }
            BuiltinReceiverMethod::Concat => {
                let suffix = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("string.concat suffix must be a string".to_string())
                })?;
                Ok(Some(RuntimeValue::String(format!("{value}{suffix}"))))
            }
            BuiltinReceiverMethod::StartsWith => {
                let prefix = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("string.startsWith prefix must be a string".to_string())
                })?;
                Ok(Some(RuntimeValue::Bool(value.starts_with(prefix))))
            }
            BuiltinReceiverMethod::EndsWith => {
                let suffix = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("string.endsWith suffix must be a string".to_string())
                })?;
                Ok(Some(RuntimeValue::Bool(value.ends_with(suffix))))
            }
            BuiltinReceiverMethod::Lowercase => {
                Ok(Some(RuntimeValue::String(value.to_lowercase())))
            }
            _ => Ok(None),
        }
    }
}

struct NumberReceiverMethods;

impl NumberReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        _args: &[RuntimeValue],
        _heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Floor
                | BuiltinReceiverMethod::Ceil
                | BuiltinReceiverMethod::Round
        ) {
            return Ok(None);
        }
        if !matches!(receiver, RuntimeValue::Number(_)) {
            return Ok(None);
        }

        match op_method {
            BuiltinReceiverMethod::Floor => Ok(Some(runtime_number_value(
                runtime_numeric(receiver)?.floor(),
            ))),
            BuiltinReceiverMethod::Ceil => Ok(Some(runtime_number_value(
                runtime_numeric(receiver)?.ceil(),
            ))),
            BuiltinReceiverMethod::Round => Ok(Some(runtime_number_value(
                runtime_numeric(receiver)?.round(),
            ))),
            _ => Ok(None),
        }
    }
}

struct DateReceiverMethods;

impl DateReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        args: &[RuntimeValue],
        _heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::ToEpochMilliseconds
                | BuiltinReceiverMethod::ToIsoString
                | BuiltinReceiverMethod::AddMilliseconds
                | BuiltinReceiverMethod::DiffMilliseconds
                | BuiltinReceiverMethod::Compare
                | BuiltinReceiverMethod::IsBefore
                | BuiltinReceiverMethod::IsAfter
        ) {
            return Ok(None);
        }
        let RuntimeValue::Date(ms) = receiver else {
            return Ok(None);
        };

        match op_method {
            BuiltinReceiverMethod::ToEpochMilliseconds => Ok(Some(runtime_integer_value(*ms))),
            BuiltinReceiverMethod::ToIsoString => Ok(Some(RuntimeValue::String(
                date_value::format_epoch_millis(*ms, "Date.toISOString")?,
            ))),
            BuiltinReceiverMethod::AddMilliseconds => {
                let delta = integer_arg_i64(args.first(), "Date.addMilliseconds")?;
                let value = ms.checked_add(delta).ok_or_else(|| {
                    RuntimeError::decode_target(
                        "Date.addMilliseconds",
                        "Date.addMilliseconds overflow",
                    )
                })?;
                Ok(Some(RuntimeValue::Date(date_value::validate_epoch_millis(
                    value,
                    "Date.addMilliseconds",
                )?)))
            }
            BuiltinReceiverMethod::DiffMilliseconds => {
                let other = date_arg(args.first(), "Date.diffMilliseconds")?;
                let diff = ms.checked_sub(other).ok_or_else(|| {
                    RuntimeError::decode_target(
                        "Date.diffMilliseconds",
                        "Date.diffMilliseconds overflow",
                    )
                })?;
                Ok(Some(runtime_integer_value(diff)))
            }
            BuiltinReceiverMethod::Compare => {
                let other = date_arg(args.first(), "Date.compare")?;
                Ok(Some(runtime_integer_value(match ms.cmp(&other) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                })))
            }
            BuiltinReceiverMethod::IsBefore => {
                let other = date_arg(args.first(), "Date.isBefore")?;
                Ok(Some(RuntimeValue::Bool(*ms < other)))
            }
            BuiltinReceiverMethod::IsAfter => {
                let other = date_arg(args.first(), "Date.isAfter")?;
                Ok(Some(RuntimeValue::Bool(*ms > other)))
            }
            _ => Ok(None),
        }
    }
}

pub fn is_runtime_receiver_native_binding_key(binding_key: &str) -> bool {
    matches!(
        binding_key,
        "core.date.toEpochMilliseconds"
            | "core.date.toISOString"
            | "core.date.addMilliseconds"
            | "core.date.diffMilliseconds"
            | "core.date.compare"
            | "core.date.isBefore"
            | "core.date.isAfter"
            | "core.duration.toMilliseconds"
    )
}

struct DurationReceiverMethods;

impl DurationReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        _args: &[RuntimeValue],
        _heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if op_method != BuiltinReceiverMethod::ToMilliseconds {
            return Ok(None);
        }
        let RuntimeValue::Number(value) = receiver else {
            return Ok(None);
        };
        if !value.is_finite() || value.fract() != 0.0 {
            return Err(RuntimeError::decode_target(
                "Duration.toMilliseconds",
                "Duration.toMilliseconds receiver must be an integer",
            ));
        }
        Ok(Some(runtime_integer_number_value(
            *value,
            "Duration.toMilliseconds",
        )?))
    }
}

struct BytesReceiverMethods;

impl BytesReceiverMethods {
    fn dispatch(
        op_method: BuiltinReceiverMethod,
        receiver: &RuntimeValue,
        _args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<Option<RuntimeValue>> {
        if !matches!(
            op_method,
            BuiltinReceiverMethod::Length
                | BuiltinReceiverMethod::ToBase64
                | BuiltinReceiverMethod::ToHex
                | BuiltinReceiverMethod::ToUtf8String
        ) {
            return Ok(None);
        }
        let Some(bytes) = RuntimeValueView::new(receiver, heap).bytes_payload()? else {
            return Ok(None);
        };

        match op_method {
            BuiltinReceiverMethod::Length => Ok(Some(RuntimeValue::Number(bytes.len() as f64))),
            BuiltinReceiverMethod::ToBase64 => Ok(Some(RuntimeValue::String(
                boundary_bytes::encode_base64(bytes.as_slice()),
            ))),
            BuiltinReceiverMethod::ToHex => {
                Ok(Some(RuntimeValue::String(hex::encode(bytes.as_slice()))))
            }
            BuiltinReceiverMethod::ToUtf8String => {
                let text = std::str::from_utf8(bytes.as_slice()).map_err(|error| {
                    RuntimeError::bytes_decode(
                        "bytes.toUtf8String",
                        format!("bytes.toUtf8String decode failed: {error}"),
                    )
                })?;
                Ok(Some(RuntimeValue::String(text.to_string())))
            }
            _ => Ok(None),
        }
    }
}

fn date_arg(value: Option<&RuntimeValue>, target: &str) -> Result<i64> {
    match value {
        Some(RuntimeValue::Date(ms)) => Ok(*ms),
        _ => Err(RuntimeError::Decode(format!(
            "{target} requires a Date argument"
        ))),
    }
}

fn integer_arg_i64(value: Option<&RuntimeValue>, target: &str) -> Result<i64> {
    let value = match value {
        Some(RuntimeValue::Number(value)) if value.is_finite() && value.fract() == 0.0 => *value,
        _ => {
            return Err(RuntimeError::Decode(format!(
                "{target} requires an integer argument"
            )))
        }
    };
    if value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(RuntimeError::Decode(format!(
            "{target} integer argument is outside i64 range"
        )));
    }
    Ok(value as i64)
}

fn runtime_integer_value(value: i64) -> RuntimeValue {
    RuntimeValue::Number(value as f64)
}

fn runtime_integer_number_value(value: f64, target: &str) -> Result<RuntimeValue> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if !value.is_finite() || value.fract() != 0.0 || value.abs() > MAX_SAFE_INTEGER {
        return Err(RuntimeError::Decode(format!(
            "{target} requires a safe integer"
        )));
    }
    Ok(runtime_integer_value(value as i64))
}

#[cfg(test)]
mod json_object_receiver_tests {
    use super::*;
    use skiff_artifact_model::builtin_receiver_op_by_name;
    use skiff_runtime_model::runtime_value::{RuntimeObject, RuntimeObjectFields};

    fn receiver_op(root: &str, method: &str) -> BuiltinReceiverOp {
        builtin_receiver_op_by_name(root, method).expect("receiver op must exist")
    }

    #[test]
    fn json_object_receiver_reads_object_heap_nodes() {
        let mut heap = RequestHeap::default();
        let object = RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "flag".to_string(),
            RuntimeValue::Bool(true),
        )]));
        let object_value = RuntimeValue::Heap(heap.alloc_object(object).unwrap());

        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("JsonObject", "length"),
                    object_value.clone(),
                    vec![]
                )
                .expect("JsonObject.length should read object fields"),
            RuntimeValue::Number(1.0)
        );
        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("JsonObject", "has"),
                    object_value.clone(),
                    vec![RuntimeValue::String("flag".to_string())],
                )
                .expect("JsonObject.has should read object fields"),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("JsonObject", "get"),
                    object_value,
                    vec![RuntimeValue::String("flag".to_string())],
                )
                .expect("JsonObject.get should read object fields"),
            RuntimeValue::Bool(true)
        );
    }
}

#[cfg(all(test, any()))]
mod tests {
    use super::*;
    use skiff_artifact_model::builtin_receiver_op_by_name;

    fn receiver_op(root: &str, method: &str) -> BuiltinReceiverOp {
        builtin_receiver_op_by_name(root, method).expect("receiver op must exist")
    }

    #[test]
    fn string_replace_all_receiver_method_dispatches() {
        let mut heap = RequestHeap::default();
        let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("string", "replaceAll"),
                    RuntimeValue::String("a-b-a".to_string()),
                    vec![
                        RuntimeValue::String("a".to_string()),
                        RuntimeValue::String("z".to_string())
                    ],
                )
                .expect("string.replaceAll should dispatch"),
            RuntimeValue::String("z-b-z".to_string())
        );
    }

    #[test]
    fn date_receiver_methods_dispatch() {
        let mut heap = RequestHeap::default();
        let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "toEpochMilliseconds"),
                    RuntimeValue::Date(1_000),
                    vec![]
                )
                .expect("toEpochMilliseconds should dispatch"),
            runtime_integer_value(1_000)
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "toISOString"),
                    RuntimeValue::Date(1_000),
                    vec![]
                )
                .expect("toISOString should dispatch"),
            RuntimeValue::String("1970-01-01T00:00:01.000Z".to_string())
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "addMilliseconds"),
                    RuntimeValue::Date(1_000),
                    vec![RuntimeValue::Number(500.0)],
                )
                .expect("addMilliseconds should dispatch"),
            RuntimeValue::Date(1_500)
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "diffMilliseconds"),
                    RuntimeValue::Date(1_500),
                    vec![RuntimeValue::Date(1_000)],
                )
                .expect("diffMilliseconds should dispatch"),
            runtime_integer_value(500)
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "compare"),
                    RuntimeValue::Date(1_000),
                    vec![RuntimeValue::Date(1_500)],
                )
                .expect("compare should dispatch"),
            runtime_integer_value(-1)
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "isBefore"),
                    RuntimeValue::Date(1_000),
                    vec![RuntimeValue::Date(1_500)],
                )
                .expect("isBefore should dispatch"),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Date", "isAfter"),
                    RuntimeValue::Date(1_500),
                    vec![RuntimeValue::Date(1_000)],
                )
                .expect("isAfter should dispatch"),
            RuntimeValue::Bool(true)
        );
    }

    #[test]
    fn duration_receiver_methods_dispatch_erased_milliseconds() {
        let mut heap = RequestHeap::default();
        let mut dispatch = ReceiverMethodDispatch::new(&mut heap);

        assert_eq!(
            dispatch
                .dispatch_op(
                    &receiver_op("Duration", "toMilliseconds"),
                    RuntimeValue::Number(2_000.0),
                    vec![]
                )
                .expect("Duration.toMilliseconds should dispatch"),
            runtime_integer_value(2_000)
        );
    }
}
