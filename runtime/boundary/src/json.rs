use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::{
    date_value,
    error::{Result, RuntimeError},
    json_convert::{self, BoundaryStreamHandlePolicy},
    map_key::RuntimeMapKeyShape,
    plan::BoundaryUse,
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
    stream::STREAM_ID_KEY,
    type_descriptor::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan},
    value::{bytes_payload, bytes_value},
};

pub const BYTES_BASE64_KEY: &str = "__skiffBytesBase64";

const HEAP_HANDLE_KEYS: &[&str] = &[
    "__skiffHeapHandle",
    "__skiffRuntimeHeapHandle",
    "__skiffRequestHeapHandle",
    "__skiffHeapRef",
    "__skiffRuntimeValue",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolJsonObject {
    PlainObject,
    Bytes { contains_internal_handle: bool },
    StreamHandle,
    InternalRuntimeHandle,
}

impl ProtocolJsonObject {
    pub fn classify(object: &Map<String, Value>) -> Self {
        let contains_internal_handle = contains_internal_handle(object);
        if object.contains_key(BYTES_BASE64_KEY) {
            return Self::Bytes {
                contains_internal_handle,
            };
        }
        if has_heap_handle(object) {
            return Self::InternalRuntimeHandle;
        }
        if is_canonical_stream_handle(object) {
            return Self::StreamHandle;
        }
        if object.contains_key(STREAM_ID_KEY) {
            return Self::InternalRuntimeHandle;
        }
        Self::PlainObject
    }

    pub fn contains_internal_runtime_handle(self) -> bool {
        match self {
            Self::Bytes {
                contains_internal_handle,
            } => contains_internal_handle,
            Self::StreamHandle | Self::InternalRuntimeHandle => true,
            Self::PlainObject => false,
        }
    }
}

pub fn is_internal_metadata_key(key: &str) -> bool {
    key == "$type"
        || key == "__type"
        || key == "__nominal"
        || key == "__collection"
        || HEAP_HANDLE_KEYS.contains(&key)
        || key == STREAM_ID_KEY
}

impl BoundaryUse {
    fn policy(self) -> BoundaryPolicy {
        match self {
            Self::TypedJson
            | Self::JsonValueProjection
            | Self::RuntimeBinary
            | Self::HttpRequest
            | Self::HttpResponse
            | Self::NativeArg
            | Self::ConfigValue
            | Self::DbResultDecode
            | Self::DbWriteProjection => BoundaryPolicy {
                stream_handles: BoundaryStreamHandlePolicy::ExternalBoundary,
                runtime_owned_handle_fields: false,
            },
            Self::NativeReturn => BoundaryPolicy {
                stream_handles: BoundaryStreamHandlePolicy::ExternalBoundary,
                runtime_owned_handle_fields: true,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoundaryPolicy {
    stream_handles: BoundaryStreamHandlePolicy,
    runtime_owned_handle_fields: bool,
}

impl BoundaryPolicy {
    fn external_stream_policy(self) -> BoundaryStreamHandlePolicy {
        self.stream_handles
    }

    fn runtime_owned_stream_policy(self, label: &str) -> Result<BoundaryStreamHandlePolicy> {
        if self.runtime_owned_handle_fields {
            Ok(BoundaryStreamHandlePolicy::RuntimeOwnedHandleFields)
        } else {
            Err(RuntimeError::Decode(format!(
                "{label}: boundary use does not allow runtime-owned internal handles"
            )))
        }
    }
}

#[allow(dead_code)]
pub enum BoundaryTypeNode<'a> {
    Nullable(&'a RuntimeTypePlan),
    Union(&'a [RuntimeTypePlan]),
    LiteralString(&'a str),
    Json,
    JsonObject,
    Bytes,
    Date,
    String,
    Bool,
    Number,
    Integer,
    Null,
    Stream(&'a RuntimeTypePlan),
    Array(&'a RuntimeTypePlan),
    Map {
        key: &'a RuntimeTypePlan,
        value: &'a RuntimeTypePlan,
    },
    Record {
        fields: &'a [RuntimeRecordFieldPlan],
    },
    Unknown,
}

pub struct RuntimeBoundaryCodec<'a> {
    plan: &'a RuntimeTypePlan,
    use_case: BoundaryUse,
    label: Cow<'a, str>,
}

impl<'a> RuntimeBoundaryCodec<'a> {
    /// Low-level codec constructor.
    ///
    /// Production boundary policy entry points should prefer
    /// [`crate::contract::RuntimeBoundaryContract`]. This constructor is kept
    /// public for low-level codec internals, focused codec tests, and current
    /// compatibility shims.
    pub fn new(
        plan: &'a RuntimeTypePlan,
        use_case: BoundaryUse,
        label: impl Into<Cow<'a, str>>,
    ) -> Self {
        Self {
            plan,
            use_case,
            label: label.into(),
        }
    }

    pub fn typed_node(&self) -> Result<BoundaryTypeNode<'a>> {
        let _ = self.policy();
        boundary_type_node(self.plan)
    }

    pub fn from_wire_json(&self, value: &Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
        json_convert::decode_wire_plan_impl(
            value,
            self.plan,
            heap,
            self.policy().external_stream_policy(),
        )
        .map_err(|error| self.add_context(error))
    }

    pub fn from_wire_json_internal_handle(
        &self,
        value: &Value,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let stream_policy = self
            .policy()
            .runtime_owned_stream_policy(self.label.as_ref())?;
        json_convert::decode_wire_plan_impl(value, self.plan, heap, stream_policy)
            .map_err(|error| self.add_context(error))
    }

    pub fn to_wire_json(&self, value: &RuntimeValue, heap: &mut RequestHeap) -> Result<Value> {
        json_convert::encode_wire_plan_impl(
            value,
            self.plan,
            heap,
            self.policy().external_stream_policy(),
        )
        .map_err(|error| self.add_context(error))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn to_wire_json_internal_handle(
        &self,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<Value> {
        let stream_policy = self
            .policy()
            .runtime_owned_stream_policy(self.label.as_ref())?;
        json_convert::encode_wire_plan_impl(value, self.plan, heap, stream_policy)
            .map_err(|error| self.add_context(error))
    }

    pub fn coerce_runtime_value(
        &self,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        json_convert::coerce_runtime_value_plan_impl(value, self.plan, heap)
            .map_err(|error| self.add_context(error))
    }

    pub fn decode_json_text(&self, input: &str, heap: &mut RequestHeap) -> Result<RuntimeValue> {
        json_convert::decode_json_text_runtime_value_plan_impl(
            input,
            self.plan,
            heap,
            self.policy().external_stream_policy(),
        )
        .map_err(|error| self.add_context(error))
    }

    pub fn encode_json_text_value(
        &self,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
    ) -> Result<String> {
        json_convert::encode_json_runtime_value_plan_impl(value, Some(self.plan), heap)
            .map_err(|error| self.add_context(error))
    }

    pub fn encode_json_text(
        value: &RuntimeValue,
        expected_type: Option<&RuntimeTypePlan>,
        use_case: BoundaryUse,
        label: impl Into<Cow<'a, str>>,
        heap: &mut RequestHeap,
    ) -> Result<String> {
        let label = label.into();
        match expected_type {
            Some(plan) => {
                RuntimeBoundaryCodec::new(plan, use_case, label).encode_json_text_value(value, heap)
            }
            None => json_convert::encode_json_runtime_value_plan_impl(value, None, heap)
                .map_err(|error| add_boundary_context(error, label.as_ref())),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn to_json_runtime_value(
        value: &RuntimeValue,
        expected_type: Option<&RuntimeTypePlan>,
        use_case: BoundaryUse,
        label: impl Into<Cow<'a, str>>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let label = label.into();
        match expected_type {
            Some(plan) => {
                let codec = RuntimeBoundaryCodec::new(plan, use_case, label);
                let coerced = codec.coerce_runtime_value(value, heap)?;
                json_convert::to_json_runtime_value_plan_impl(&coerced, None, heap)
                    .map_err(|error| codec.add_context(error))
            }
            None => json_convert::to_json_runtime_value_plan_impl(value, None, heap)
                .map_err(|error| add_boundary_context(error, label.as_ref())),
        }
    }

    fn policy(&self) -> BoundaryPolicy {
        self.use_case.policy()
    }

    fn add_context(&self, error: RuntimeError) -> RuntimeError {
        add_boundary_context(error, self.label.as_ref())
    }
}

pub fn runtime_map_key_shape(key_type: &RuntimeTypePlan) -> Result<RuntimeMapKeyShape> {
    RuntimeMapKeyShape::for_plan(erased_boundary_plan(key_type))
}

pub fn decode_untyped_wire_json(value: &Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    match value {
        Value::Null => Ok(RuntimeValue::Null),
        Value::Bool(value) => Ok(RuntimeValue::Bool(*value)),
        Value::Number(value) => value
            .as_f64()
            .map(RuntimeValue::Number)
            .ok_or_else(|| RuntimeError::Decode("number is not representable as f64".to_string())),
        Value::String(value) => Ok(RuntimeValue::String(value.clone())),
        Value::Array(items) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                result.push(decode_untyped_wire_json(item, heap)?);
            }
            Ok(RuntimeValue::Heap(heap.alloc_array(result)?))
        }
        Value::Object(object) => {
            if let Some(bytes) = bytes_payload(value) {
                return Ok(RuntimeValue::Heap(heap.alloc_bytes(bytes)?));
            }
            let mut fields = RuntimeObjectFields::new();
            for (key, value) in object {
                fields.insert(key.clone(), decode_untyped_wire_json(value, heap)?);
            }
            let object = RuntimeObject::unshaped(fields);
            Ok(RuntimeValue::Heap(heap.alloc_object(object)?))
        }
    }
}

pub fn encode_untyped_wire_json(value: &RuntimeValue, heap: &RequestHeap) -> Result<Value> {
    encode_untyped_wire_json_inner(value, heap, 0)
}

pub fn reject_reserved_legacy_metadata_key(key: &str) -> Result<()> {
    if is_reserved_legacy_metadata_key(key) {
        return Err(crate::error::RuntimeError::Decode(format!(
            "reserved Skiff metadata field {key} is not allowed at boundary"
        )));
    }
    Ok(())
}

fn boundary_type_node(plan: &RuntimeTypePlan) -> Result<BoundaryTypeNode<'_>> {
    Ok(match plan.node() {
        RuntimeTypeNode::Alias(target) => return boundary_type_node(target),
        RuntimeTypeNode::Nullable(inner) => BoundaryTypeNode::Nullable(inner),
        RuntimeTypeNode::Union(types) => BoundaryTypeNode::Union(types),
        RuntimeTypeNode::LiteralString(literal) => BoundaryTypeNode::LiteralString(literal),
        RuntimeTypeNode::Representation { payload, .. } => return boundary_type_node(payload),
        RuntimeTypeNode::Json => BoundaryTypeNode::Json,
        RuntimeTypeNode::JsonObject => BoundaryTypeNode::JsonObject,
        RuntimeTypeNode::Bytes => BoundaryTypeNode::Bytes,
        RuntimeTypeNode::Date => BoundaryTypeNode::Date,
        RuntimeTypeNode::String => BoundaryTypeNode::String,
        RuntimeTypeNode::Bool => BoundaryTypeNode::Bool,
        RuntimeTypeNode::Number => BoundaryTypeNode::Number,
        RuntimeTypeNode::Integer => BoundaryTypeNode::Integer,
        RuntimeTypeNode::Null => BoundaryTypeNode::Null,
        RuntimeTypeNode::Stream(item) => BoundaryTypeNode::Stream(item),
        RuntimeTypeNode::Array(item) => BoundaryTypeNode::Array(item),
        RuntimeTypeNode::Map { key, value } => BoundaryTypeNode::Map { key, value },
        RuntimeTypeNode::Record { fields, .. } => BoundaryTypeNode::Record { fields },
        RuntimeTypeNode::Unknown => BoundaryTypeNode::Unknown,
    })
}

fn erased_boundary_plan(mut plan: &RuntimeTypePlan) -> &RuntimeTypePlan {
    loop {
        match plan.node() {
            RuntimeTypeNode::Alias(target) => plan = target,
            RuntimeTypeNode::Representation { payload, .. } => plan = payload,
            _ => return plan,
        }
    }
}

fn encode_untyped_wire_json_inner(
    value: &RuntimeValue,
    heap: &RequestHeap,
    depth: usize,
) -> Result<Value> {
    if depth > heap.limits().max_materialize_depth {
        return Err(RuntimeError::ResourceLimitExceeded {
            resource: "requestHeap".to_string(),
            reason: "max materialize depth".to_string(),
            limit: heap.limits().max_materialize_depth,
            current: depth,
            requested_delta: 1,
        });
    }
    match value {
        RuntimeValue::Null => Ok(Value::Null),
        RuntimeValue::Bool(value) => Ok(Value::Bool(*value)),
        RuntimeValue::Number(value) => Ok(runtime_number_to_json(*value)),
        RuntimeValue::Date(ms) => Ok(Value::String(date_value::format_epoch_millis(
            *ms,
            "Date wire materialize",
        )?)),
        RuntimeValue::String(value) => Ok(Value::String(value.clone())),
        RuntimeValue::ActorRef(actor_ref) => Err(RuntimeError::Decode(format!(
            "actor ref {} cannot be encoded as wire value",
            actor_ref.actor_type_identity()
        ))),
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Bytes(bytes) => Ok(bytes_value(bytes.as_slice())),
            HeapNode::Array(items) => {
                let mut result = Vec::with_capacity(items.len());
                for item in items {
                    result.push(encode_untyped_wire_json_inner(item, heap, depth + 1)?);
                }
                Ok(Value::Array(result))
            }
            HeapNode::Object(object) => {
                let mut result = Map::new();
                for (key, value) in object.fields() {
                    result.insert(
                        key.clone(),
                        encode_untyped_wire_json_inner(value, heap, depth + 1)?,
                    );
                }
                Ok(Value::Object(result))
            }
            HeapNode::Map(map) => {
                let mut result = Map::new();
                for (key, value) in map {
                    result.insert(
                        key.string_payload().to_string(),
                        encode_untyped_wire_json_inner(value, heap, depth + 1)?,
                    );
                }
                Ok(Value::Object(result))
            }
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "{} cannot be encoded as wire value",
                value.diagnostic_label()
            ))),
        },
    }
}

fn runtime_number_to_json(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() <= max_safe_json_integer() {
        return Value::Number((value as i64).into());
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn max_safe_json_integer() -> f64 {
    9_007_199_254_740_991.0
}

pub fn add_boundary_context(error: RuntimeError, boundary: &str) -> RuntimeError {
    match error {
        RuntimeError::Decode(message) => RuntimeError::Decode(format!("{boundary}: {message}")),
        RuntimeError::InvalidArtifact(message) => {
            RuntimeError::InvalidArtifact(format!("{boundary}: {message}"))
        }
        other => other,
    }
}

fn is_reserved_legacy_metadata_key(key: &str) -> bool {
    key.strip_prefix("__skiff")
        .is_some_and(|suffix| suffix == "Type")
}

pub fn has_heap_handle(object: &Map<String, Value>) -> bool {
    HEAP_HANDLE_KEYS.iter().any(|key| object.contains_key(*key))
}

fn contains_internal_handle(object: &Map<String, Value>) -> bool {
    has_heap_handle(object) || object.contains_key(STREAM_ID_KEY)
}

fn is_canonical_stream_handle(object: &Map<String, Value>) -> bool {
    object.len() == 1 && object.get(STREAM_ID_KEY).and_then(Value::as_str).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        request_heap::{RequestHeap, RequestHeapLimits},
        runtime_value::{
            ActorRef, HeapHandle, HeapNode, InterfaceCarrier, InterfaceValue, RemoteOperationTable,
            RuntimeMap, RuntimeValue, RuntimeValueKey,
        },
    };

    use super::*;

    #[test]
    fn untyped_wire_decodes_bytes_arrays_and_objects() {
        let mut heap = RequestHeap::default();
        let value = decode_untyped_wire_json(
            &json!({
                "bytes": { "__skiffBytesBase64": "aGk=" },
                "items": [true, 2]
            }),
            &mut heap,
        )
        .expect("untyped wire should decode");

        let RuntimeValue::Heap(handle) = value else {
            panic!("root should be a heap object");
        };
        let HeapNode::Object(object) = heap.get(handle).expect("object handle") else {
            panic!("root handle should point to object");
        };
        assert_eq!(
            object.fields().get("items"),
            Some(&RuntimeValue::Heap(HeapHandle::new(1, 0)))
        );
        let RuntimeValue::Heap(bytes_handle) = object.fields().get("bytes").unwrap() else {
            panic!("bytes field should be a heap value");
        };
        let HeapNode::Bytes(bytes) = heap.get(*bytes_handle).expect("bytes handle") else {
            panic!("bytes handle should point to bytes");
        };
        assert_eq!(bytes.as_slice(), b"hi");
    }

    #[test]
    fn untyped_wire_encodes_dates_maps_and_heap_values() {
        let mut heap = RequestHeap::default();
        let mut map = RuntimeMap::new();
        map.insert(
            RuntimeValueKey::string("when"),
            RuntimeValue::Date(1_609_459_200_000),
        );
        map.insert(RuntimeValueKey::string("count"), RuntimeValue::Number(2.0));
        let handle = heap.alloc_map(map).expect("map alloc");

        let encoded = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
            .expect("untyped wire should encode");

        assert_eq!(
            encoded,
            json!({
                "count": 2,
                "when": "2021-01-01T00:00:00.000Z"
            })
        );
    }

    #[test]
    fn untyped_wire_encode_enforces_materialize_depth_limit() {
        let mut heap = RequestHeap::new(RequestHeapLimits {
            max_materialize_depth: 0,
            ..RequestHeapLimits::default()
        });
        let handle = heap
            .alloc_array(vec![RuntimeValue::String("too deep".to_string())])
            .expect("array alloc");

        let error = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
            .expect_err("nested item should exceed materialize depth");

        assert!(matches!(
            error,
            RuntimeError::ResourceLimitExceeded { reason, .. } if reason == "max materialize depth"
        ));
    }

    #[test]
    fn untyped_wire_rejects_actor_and_interface_values() {
        let heap = RequestHeap::default();
        let actor = RuntimeValue::ActorRef(ActorRef::new(
            "svc",
            "actor-type",
            "id-type",
            "v1",
            Vec::new(),
            "hash",
            None,
        ));
        let error = encode_untyped_wire_json(&actor, &heap).expect_err("actor should reject");
        assert!(error
            .to_string()
            .contains("actor ref actor-type cannot be encoded as wire value"));

        let mut heap = RequestHeap::default();
        let interface = InterfaceValue::new(
            "iface".to_string(),
            InterfaceCarrier::Remote {
                dependency_ref: "dep".to_string(),
                public_instance_key: "instance".to_string(),
                operations: RemoteOperationTable::new(
                    "ops".to_string(),
                    "iface".to_string(),
                    Vec::new(),
                ),
            },
        );
        let handle = heap.alloc_interface(interface).expect("interface alloc");
        let error = encode_untyped_wire_json(&RuntimeValue::Heap(handle), &heap)
            .expect_err("interface should reject");
        assert!(error
            .to_string()
            .contains("any interface iface (remote) cannot be encoded as wire value"));
    }
}
