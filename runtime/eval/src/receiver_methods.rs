//! Closed builtin receiver method dispatch for RuntimeProgram values.

use skiff_runtime_boundary::{date_value, value as boundary_bytes};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueCarrier, RuntimeValueKey},
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
        runtime_array_item_carriers, runtime_array_items, runtime_debug_value_for_error,
        runtime_deep_clone, runtime_deep_clone_carrier, runtime_map_get, runtime_map_get_carrier,
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

    pub fn dispatch_op_carriers(
        &mut self,
        op: &BuiltinReceiverOp,
        receiver: RuntimeValueCarrier,
        args: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        skiff_artifact_model::validate_supported_receiver_builtin_op(op).map_err(|error| {
            RuntimeError::InvalidArtifact(format!(
                "unsupported receiver builtin op {}: {}",
                op.canonical_key, error
            ))
        })?;
        match op.receiver {
            BuiltinReceiverRoot::Array => {
                self.dispatch_array_carriers(op.method, receiver, args.as_slice())
            }
            BuiltinReceiverRoot::Map => {
                self.dispatch_map_carriers(op.method, receiver, args.as_slice())
            }
            BuiltinReceiverRoot::JsonObject => {
                self.dispatch_json_object_carriers(op.method, receiver, args.as_slice())
            }
            _ => self
                .dispatch_op(
                    op,
                    receiver.into_value(),
                    args.into_iter()
                        .map(RuntimeValueCarrier::into_value)
                        .collect(),
                )
                .map(Into::into),
        }
    }

    fn dispatch_array_carriers(
        &mut self,
        method: BuiltinReceiverMethod,
        receiver: RuntimeValueCarrier,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        let Some(items) = runtime_array_item_carriers(&receiver, self.heap)? else {
            return Err(RuntimeError::Decode("receiver is not an Array".to_string()));
        };
        let handle =
            program_mutable_receiver_handle(receiver.value(), self.heap, "Array receiver")?;
        match method {
            BuiltinReceiverMethod::Length => Ok(RuntimeValue::Number(items.len() as f64).into()),
            BuiltinReceiverMethod::Push => {
                self.heap.push_array_item_carrier(
                    handle,
                    args.first()
                        .cloned()
                        .unwrap_or_else(|| RuntimeValue::Null.into()),
                )?;
                Ok(RuntimeValue::Null.into())
            }
            BuiltinReceiverMethod::Set => {
                let index = args
                    .first()
                    .and_then(|value| runtime_u64(value.value()))
                    .ok_or_else(|| {
                        RuntimeError::Decode(
                            "Array.set index must be a non-negative number".to_string(),
                        )
                    })?;
                self.heap.set_array_item_carrier(
                    handle,
                    index as usize,
                    args.get(1)
                        .cloned()
                        .unwrap_or_else(|| RuntimeValue::Null.into()),
                )?;
                Ok(RuntimeValue::Null.into())
            }
            BuiltinReceiverMethod::Pop => {
                self.heap.pop_array_item_carrier(handle).map_err(Into::into)
            }
            BuiltinReceiverMethod::Clone => runtime_deep_clone_carrier(&receiver, self.heap),
            _ => Err(RuntimeError::InvalidArtifact(
                "unsupported Array receiver method reached carrier dispatch".to_string(),
            )),
        }
    }

    fn dispatch_map_carriers(
        &mut self,
        method: BuiltinReceiverMethod,
        receiver: RuntimeValueCarrier,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        if !is_heap_map(receiver.value(), self.heap)? {
            return Err(RuntimeError::Decode("receiver is not a Map".to_string()));
        }
        let handle = program_mutable_receiver_handle(receiver.value(), self.heap, "Map receiver")?;
        match method {
            BuiltinReceiverMethod::Length => Ok(RuntimeValue::Number(
                RuntimeValueView::new(receiver.value(), self.heap).map_like_len()? as f64,
            )
            .into()),
            BuiltinReceiverMethod::Get => runtime_map_get_carrier(
                &receiver,
                args.first()
                    .unwrap_or(&RuntimeValueCarrier::unidentified(RuntimeValue::Null)),
                self.heap,
            ),
            BuiltinReceiverMethod::Has => {
                let [key] = args else {
                    return Err(RuntimeError::Decode(
                        "Map.has requires exactly one key".to_string(),
                    ));
                };
                Ok(
                    RuntimeValue::Bool(runtime_map_has(receiver.value(), key.value(), self.heap)?)
                        .into(),
                )
            }
            BuiltinReceiverMethod::Set => {
                let [key, value] = args else {
                    return Err(RuntimeError::Decode(
                        "Map.set requires exactly one key and one value".to_string(),
                    ));
                };
                let key = map_key_from_runtime_value(key.value(), self.heap)?;
                self.heap
                    .set_map_entry_carrier(handle, key, value.clone())?;
                Ok(RuntimeValue::Null.into())
            }
            BuiltinReceiverMethod::Delete => {
                let key = map_key_from_runtime_value(
                    args.first()
                        .map(RuntimeValueCarrier::value)
                        .unwrap_or(&RuntimeValue::Null),
                    self.heap,
                )?;
                Ok(RuntimeValue::Bool(self.heap.delete_map_entry(handle, &key)?).into())
            }
            BuiltinReceiverMethod::Keys => {
                let HeapNode::Map(map) = self.heap.get(handle)? else {
                    unreachable!("Map receiver was checked above");
                };
                let keys = map
                    .keys()
                    .map(runtime_value_from_map_key)
                    .map(Into::into)
                    .collect();
                Ok(RuntimeValue::Heap(self.heap.alloc_array_carriers(keys)?).into())
            }
            BuiltinReceiverMethod::Clone => runtime_deep_clone_carrier(&receiver, self.heap),
            _ => Err(RuntimeError::InvalidArtifact(
                "unsupported Map receiver method reached carrier dispatch".to_string(),
            )),
        }
    }

    fn dispatch_json_object_carriers(
        &mut self,
        method: BuiltinReceiverMethod,
        receiver: RuntimeValueCarrier,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        if !RuntimeValueView::new(receiver.value(), self.heap).is_map_like()? {
            return Err(RuntimeError::Decode(
                "receiver is not a JsonObject".to_string(),
            ));
        }
        match method {
            BuiltinReceiverMethod::Get => runtime_map_get_carrier(
                &receiver,
                args.first()
                    .unwrap_or(&RuntimeValueCarrier::unidentified(RuntimeValue::Null)),
                self.heap,
            ),
            BuiltinReceiverMethod::Set => self.dispatch_map_like_set_carrier(&receiver, args),
            BuiltinReceiverMethod::Clone => runtime_deep_clone_carrier(&receiver, self.heap),
            _ => JsonObjectReceiverMethods::dispatch(
                method,
                receiver.value(),
                &args
                    .iter()
                    .cloned()
                    .map(RuntimeValueCarrier::into_value)
                    .collect::<Vec<_>>(),
                self.heap,
            )?
            .map(Into::into)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "unsupported JsonObject receiver method reached carrier dispatch".to_string(),
                )
            }),
        }
    }

    fn dispatch_map_like_set_carrier(
        &mut self,
        receiver: &RuntimeValueCarrier,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        let [key, value] = args else {
            return Err(RuntimeError::Decode(
                "JsonObject.set requires exactly one key and one value".to_string(),
            ));
        };
        let handle =
            program_mutable_receiver_handle(receiver.value(), self.heap, "JsonObject.set")?;
        let key = map_key_from_runtime_value(key.value(), self.heap)?;
        self.heap
            .set_map_entry_carrier(handle, key, value.clone())?;
        Ok(RuntimeValue::Null.into())
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
                let [key] = args else {
                    return Err(RuntimeError::Decode(
                        "Map.has requires exactly one key".to_string(),
                    ));
                };
                Ok(Some(RuntimeValue::Bool(runtime_map_has(
                    receiver, key, heap,
                )?)))
            }
            BuiltinReceiverMethod::Set => {
                let [key, value] = args else {
                    return Err(RuntimeError::Decode(
                        "Map.set requires exactly one key and one value".to_string(),
                    ));
                };
                let key = map_key_from_runtime_value(key, heap)?;
                let value = value.clone();
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
                let field = args.first().and_then(runtime_string).ok_or_else(|| {
                    RuntimeError::Decode("JsonObject.delete field must be a string".to_string())
                })?;
                let handle = program_mutable_receiver_handle(receiver, heap, "JsonObject.delete")?;
                Ok(Some(RuntimeValue::Bool(
                    heap.delete_object_field(handle, field)?,
                )))
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
        args: &[RuntimeValue],
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
        if matches!(op_method, BuiltinReceiverMethod::Ceil) && !args.is_empty() {
            return Err(RuntimeError::Decode(
                "number.ceil does not accept arguments".to_string(),
            ));
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
    skiff_artifact_model::is_runtime_receiver_native_binding_key(binding_key)
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
        args: &[RuntimeValue],
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
        if matches!(op_method, BuiltinReceiverMethod::ToHex) && !args.is_empty() {
            return Err(RuntimeError::Decode(
                "bytes.toHex does not accept arguments".to_string(),
            ));
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
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
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
    if value.abs() > MAX_SAFE_INTEGER {
        return Err(RuntimeError::Decode(format!(
            "{target} requires a safe integer"
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

    #[test]
    fn json_object_get_returns_null_for_missing_and_preserves_nested_heap_identity() {
        let mut heap = RequestHeap::default();
        let nested_object = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "value".to_string(),
                RuntimeValue::Number(1.0),
            )])))
            .unwrap();
        let nested_array = heap.alloc_array(vec![RuntimeValue::Number(2.0)]).unwrap();
        let receiver = RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                (
                    "scalar".to_string(),
                    RuntimeValue::String("text".to_string()),
                ),
                ("object".to_string(), RuntimeValue::Heap(nested_object)),
                ("array".to_string(), RuntimeValue::Heap(nested_array)),
            ])))
            .unwrap(),
        );

        for (key, expected) in [
            ("missing", RuntimeValue::Null),
            ("scalar", RuntimeValue::String("text".to_string())),
            ("object", RuntimeValue::Heap(nested_object)),
            ("array", RuntimeValue::Heap(nested_array)),
        ] {
            assert_eq!(
                ReceiverMethodDispatch::new(&mut heap)
                    .dispatch_op(
                        &receiver_op("JsonObject", "get"),
                        receiver.clone(),
                        vec![RuntimeValue::String(key.to_string())],
                    )
                    .unwrap(),
                expected,
                "{key}"
            );
        }

        assert!(
            heap.get(nested_object).is_ok() && heap.get(nested_array).is_ok(),
            "returned handles must still resolve to the receiver's original nested values"
        );
    }

    #[test]
    fn json_object_delete_mutates_the_same_object_and_reports_presence() {
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                (
                    "instructions".to_string(),
                    RuntimeValue::String("drop".to_string()),
                ),
                ("keep".to_string(), RuntimeValue::Bool(true)),
            ])))
            .unwrap();
        let receiver = RuntimeValue::Heap(handle);

        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("JsonObject", "delete"),
                    receiver.clone(),
                    vec![RuntimeValue::String("instructions".to_string())],
                )
                .expect("present JsonObject field should be deleted"),
            RuntimeValue::Bool(true)
        );
        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("JsonObject", "delete"),
                    receiver,
                    vec![RuntimeValue::String("instructions".to_string())],
                )
                .expect("missing JsonObject field should be reported"),
            RuntimeValue::Bool(false)
        );

        let HeapNode::Object(object) = heap.get(handle).unwrap() else {
            panic!("delete must preserve the receiver object heap node");
        };
        assert!(!object.fields().contains_key("instructions"));
        assert_eq!(object.fields().get("keep"), Some(&RuntimeValue::Bool(true)));
    }

    #[test]
    fn json_object_delete_rejects_a_non_string_field() {
        let mut heap = RequestHeap::default();
        let receiver = RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))
                .unwrap(),
        );

        let error = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &receiver_op("JsonObject", "delete"),
                receiver,
                vec![RuntimeValue::Number(1.0)],
            )
            .expect_err("JsonObject.delete must enforce its string field signature");
        assert!(error
            .to_string()
            .contains("JsonObject.delete field must be a string"));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use skiff_artifact_model::builtin_receiver_op_by_name;
    use skiff_runtime_model::runtime_value::HeapNode;

    fn receiver_op(root: &str, method: &str) -> BuiltinReceiverOp {
        builtin_receiver_op_by_name(root, method).expect("receiver op must exist")
    }

    #[test]
    fn array_push_mutates_the_same_heap_array_and_returns_null() {
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_array(vec![RuntimeValue::Number(1.0)])
            .expect("array allocation should succeed");
        let receiver = RuntimeValue::Heap(handle);

        let result = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &receiver_op("Array", "push"),
                receiver.clone(),
                vec![RuntimeValue::Number(2.0)],
            )
            .expect("Array.push should dispatch");

        assert_eq!(result, RuntimeValue::Null, "Array.push return must be null");
        assert_eq!(
            receiver,
            RuntimeValue::Heap(handle),
            "Array.push must preserve receiver identity"
        );
        let HeapNode::Array(items) = heap.get(handle).expect("array handle should remain live")
        else {
            panic!("Array.push receiver must remain an array");
        };
        assert_eq!(
            items,
            &vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)],
            "Array.push must mutate the caller-reachable receiver"
        );
    }

    #[test]
    fn map_get_preserves_nested_identity_and_optional_missing_behavior() {
        let mut heap = RequestHeap::default();
        let nested = heap
            .alloc_array(vec![RuntimeValue::String("nested".to_string())])
            .expect("nested array allocation should succeed");
        let map = BTreeMap::from([
            (RuntimeValueKey::string("scalar"), RuntimeValue::Number(7.0)),
            (
                RuntimeValueKey::string("nested"),
                RuntimeValue::Heap(nested),
            ),
        ]);
        let receiver = RuntimeValue::Heap(
            heap.alloc_map(map)
                .expect("Map receiver allocation should succeed"),
        );

        for (key, expected) in [
            ("scalar", RuntimeValue::Number(7.0)),
            ("nested", RuntimeValue::Heap(nested)),
            ("missing", RuntimeValue::Null),
        ] {
            let result = ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("Map", "get"),
                    receiver.clone(),
                    vec![RuntimeValue::String(key.to_string())],
                )
                .expect("Map.get should dispatch");
            assert_eq!(result, expected, "unexpected Map.get result for {key}");
        }
    }

    #[test]
    fn map_has_and_set_preserve_identity_and_nested_values() {
        let mut heap = RequestHeap::default();
        let old_nested = heap
            .alloc_array(vec![RuntimeValue::String("old".to_string())])
            .expect("old nested allocation should succeed");
        let new_nested = heap
            .alloc_array(vec![RuntimeValue::String("new".to_string())])
            .expect("new nested allocation should succeed");
        let map = BTreeMap::from([(
            RuntimeValueKey::string("present"),
            RuntimeValue::Heap(old_nested),
        )]);
        let handle = heap
            .alloc_map(map)
            .expect("Map receiver allocation should succeed");
        let receiver = RuntimeValue::Heap(handle);

        for (key, expected) in [("present", true), ("missing", false)] {
            let result = ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("Map", "has"),
                    receiver.clone(),
                    vec![RuntimeValue::String(key.to_string())],
                )
                .expect("Map.has should dispatch");
            assert_eq!(result, RuntimeValue::Bool(expected));
        }

        for key in ["present", "inserted"] {
            let result = ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &receiver_op("Map", "set"),
                    receiver.clone(),
                    vec![
                        RuntimeValue::String(key.to_string()),
                        RuntimeValue::Heap(new_nested),
                    ],
                )
                .expect("Map.set should dispatch");
            assert_eq!(result, RuntimeValue::Null);
            assert_eq!(receiver, RuntimeValue::Heap(handle));
        }

        let HeapNode::Map(map) = heap.get(handle).expect("Map handle should remain live") else {
            panic!("Map.set receiver must remain a map");
        };
        assert_eq!(
            map.get(&RuntimeValueKey::string("present")),
            Some(&RuntimeValue::Heap(new_nested))
        );
        assert_eq!(
            map.get(&RuntimeValueKey::string("inserted")),
            Some(&RuntimeValue::Heap(new_nested))
        );
    }

    #[test]
    fn map_has_and_set_reject_malformed_arity() {
        let mut heap = RequestHeap::default();
        let receiver = RuntimeValue::Heap(
            heap.alloc_map(BTreeMap::new())
                .expect("Map allocation should succeed"),
        );
        for (method, args) in [
            ("has", vec![]),
            (
                "has",
                vec![
                    RuntimeValue::String("one".to_string()),
                    RuntimeValue::String("two".to_string()),
                ],
            ),
            ("set", vec![RuntimeValue::String("key".to_string())]),
        ] {
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(&receiver_op("Map", method), receiver.clone(), args)
                .expect_err("malformed Map receiver call must fail closed");
        }
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
    fn date_add_milliseconds_preserves_range_and_typed_errors() {
        let mut heap = RequestHeap::default();
        let op = receiver_op("Date", "addMilliseconds");

        for (receiver, delta, expected) in [
            (1_000, 500.0, 1_500),
            (1_000, -500.0, 500),
            (
                date_value::MIN_EPOCH_MILLIS,
                0.0,
                date_value::MIN_EPOCH_MILLIS,
            ),
            (
                date_value::MAX_EPOCH_MILLIS,
                0.0,
                date_value::MAX_EPOCH_MILLIS,
            ),
        ] {
            assert_eq!(
                ReceiverMethodDispatch::new(&mut heap)
                    .dispatch_op(
                        &op,
                        RuntimeValue::Date(receiver),
                        vec![RuntimeValue::Number(delta)],
                    )
                    .expect("canonical Date.addMilliseconds should dispatch"),
                RuntimeValue::Date(expected)
            );
        }

        for (receiver, delta) in [
            (date_value::MAX_EPOCH_MILLIS, 1.0),
            (date_value::MIN_EPOCH_MILLIS, -1.0),
        ] {
            let error = ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(
                    &op,
                    RuntimeValue::Date(receiver),
                    vec![RuntimeValue::Number(delta)],
                )
                .expect_err("Date.addMilliseconds must reject an out-of-range Date");
            assert!(matches!(
                error,
                RuntimeError::DecodeTarget { ref target, .. }
                    if target == "Date.addMilliseconds"
            ));
        }

        for argument in [
            RuntimeValue::Number(0.5),
            RuntimeValue::Number(f64::INFINITY),
            RuntimeValue::Number(9_007_199_254_740_992.0),
            RuntimeValue::String("1".to_string()),
        ] {
            let error = ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(&op, RuntimeValue::Date(0), vec![argument])
                .expect_err("Date.addMilliseconds must require an integer argument");
            assert!(matches!(error, RuntimeError::Decode(_)));
        }
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

    #[test]
    fn number_ceil_preserves_numeric_boundaries_and_rejects_malformed_calls() {
        let mut heap = RequestHeap::default();
        let op = receiver_op("number", "ceil");

        for (input, expected) in [
            (1.25, RuntimeValue::Number(2.0)),
            (-1.25, RuntimeValue::Number(-1.0)),
            (
                9_007_199_254_740_991.0,
                RuntimeValue::Number(9_007_199_254_740_991.0),
            ),
            (f64::MAX, RuntimeValue::Number(f64::MAX)),
            (f64::INFINITY, RuntimeValue::Null),
        ] {
            assert_eq!(
                ReceiverMethodDispatch::new(&mut heap)
                    .dispatch_op(&op, RuntimeValue::Number(input), vec![])
                    .expect("canonical number.ceil should dispatch"),
                expected,
                "{input}"
            );
        }

        let wrong_receiver = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&op, RuntimeValue::String("1.25".to_string()), vec![])
            .expect_err("number.ceil must reject a non-number receiver");
        assert!(matches!(wrong_receiver, RuntimeError::Decode(_)));

        let wrong_arity = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(
                &op,
                RuntimeValue::Number(1.25),
                vec![RuntimeValue::Number(0.0)],
            )
            .expect_err("number.ceil must reject arguments");
        assert!(matches!(wrong_arity, RuntimeError::Decode(_)));
    }

    #[test]
    fn bytes_to_hex_dispatches_exact_receiver_and_rejects_malformed_calls() {
        let mut heap = RequestHeap::default();
        let op = receiver_op("bytes", "toHex");
        let receiver = RuntimeValue::Heap(
            heap.alloc_bytes(vec![0x00, 0x0f, 0xa5, 0xff])
                .expect("bytes fixture should allocate"),
        );

        assert_eq!(
            ReceiverMethodDispatch::new(&mut heap)
                .dispatch_op(&op, receiver.clone(), vec![])
                .expect("canonical bytes.toHex should dispatch"),
            RuntimeValue::String("000fa5ff".to_string())
        );

        let wrong_receiver = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&op, RuntimeValue::String("00".to_string()), vec![])
            .expect_err("bytes.toHex must reject a non-bytes receiver");
        assert!(matches!(wrong_receiver, RuntimeError::Decode(_)));

        let wrong_arity = ReceiverMethodDispatch::new(&mut heap)
            .dispatch_op(&op, receiver, vec![RuntimeValue::Number(0.0)])
            .expect_err("bytes.toHex must reject arguments");
        assert!(matches!(wrong_arity, RuntimeError::Decode(_)));
    }
}
