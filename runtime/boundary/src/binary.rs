#[cfg(any(test, feature = "test-support"))]
use serde_json::Value;

use crate::{
    date_value,
    error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode, Result, RuntimeError},
    json::{
        reject_reserved_legacy_metadata_key, runtime_map_key_shape, BoundaryTypeNode,
        RuntimeBoundaryCodec,
    },
    payload::{PayloadBoundary, PayloadServiceRef, PayloadTrust},
    plan::BoundaryUse,
    recoverable::{RecoverableBehaviorHooks, RecoverableBoundaryCodec},
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceValue, RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue,
        RuntimeValueKey,
    },
    runtime_value_graph::RuntimeValueGraph,
    type_descriptor::{RuntimeRecordFieldPlan, RuntimeTypePlan},
};
use skiff_runtime_model::recoverable::{
    RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableServiceRef,
    RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
};

#[cfg(any(test, feature = "test-support"))]
use crate::type_descriptor::RuntimeTypePlanDescriptorExt;

const MAGIC: &[u8; 4] = b"SKPV";
const VERSION: u8 = 2;

const TAG_NULL: u8 = 0;
const TAG_BOOL_FALSE: u8 = 1;
const TAG_BOOL_TRUE: u8 = 2;
const TAG_NUMBER: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_BYTES: u8 = 5;
const TAG_ARRAY: u8 = 6;
const TAG_OBJECT: u8 = 7;
const TAG_MAP: u8 = 8;
const TAG_INTERFACE: u8 = 9;
const TAG_DATE: u8 = 10;

/// `&Value`-keyed convenience wrapper around [`encode_payload_plan`], retained
/// only for tests. Production callers (service dispatch, program invocation,
/// request runner) build a [`RuntimeTypePlan`] via `from_linked` and call
/// [`encode_payload_plan`] directly, so this `.plan()`-from-`&Value` round-trip
/// has no production use.
#[cfg(any(test, feature = "test-support"))]
pub fn encode_payload(
    value: &RuntimeValue,
    expected_type: &Value,
    heap: &RequestHeap,
) -> Result<Vec<u8>> {
    let plan = RuntimeTypePlan::from_descriptor(expected_type)?;
    encode_payload_plan(value, &plan, &PayloadBoundary::runtime_internal(), heap)
}

/// Plan-accepting variant of [`encode_payload`]: encodes against an already-built
/// [`RuntimeTypePlan`], skipping the internal `.plan()` step. [`encode_payload`]
/// is a thin wrapper that builds the plan from a `&Value` and delegates here, so
/// the encode logic lives in one place.
pub fn encode_payload_plan(
    value: &RuntimeValue,
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
) -> Result<Vec<u8>> {
    encode_payload_plan_inner(value, plan, boundary, heap)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "encode"))
}

/// `&Value`-keyed convenience wrapper around [`decode_payload_plan`], retained
/// only for tests. Production callers build a [`RuntimeTypePlan`] via
/// `from_linked` and call [`decode_payload_plan`] directly, so this
/// `.plan()`-from-`&Value` round-trip has no production use.
#[cfg(any(test, feature = "test-support"))]
pub fn decode_payload(
    bytes: &[u8],
    expected_type: &Value,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = RuntimeTypePlan::from_descriptor(expected_type)?;
    decode_payload_plan(bytes, &plan, &PayloadBoundary::runtime_internal(), heap)
}

/// Plan-accepting variant of [`decode_payload`]: decodes against an already-built
/// [`RuntimeTypePlan`], skipping the internal `.plan()` step. [`decode_payload`]
/// is a thin wrapper that builds the plan from a `&Value` and delegates here, so
/// the decode logic lives in one place.
pub fn decode_payload_plan(
    bytes: &[u8],
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    decode_payload_plan_inner(bytes, plan, boundary, heap)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "decode"))
}

/// Encodes an explicit recoverable payload slot using the current runtime shape
/// as a diagnostics-only expected plan.
///
/// This is the non-DB integration helper for task/queue/runtime-wire/public/
/// materialization call sites whose compiler artifact has not yet bridged a
/// durable recoverable expected plan into this crate. Ordinary service/public
/// payloads must keep using [`encode_payload_plan`].
pub fn encode_recoverable_payload_plan(
    value: &RuntimeValue,
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
) -> Result<Vec<u8>> {
    let expected =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(plan);
    encode_recoverable_payload(value, &expected, boundary, heap)
}

/// Decodes an explicit recoverable payload slot using the current runtime shape
/// as a diagnostics-only expected plan.
///
/// Ordinary service/public payloads must keep using [`decode_payload_plan`], so
/// recoverable envelope bytes are never accepted implicitly.
pub fn decode_recoverable_payload_plan(
    bytes: &[u8],
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let expected =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(plan);
    decode_recoverable_payload(bytes, &expected, boundary, heap)
}

/// Encodes an explicit recoverable payload slot with an artifact-authored
/// expected plan supplied by the caller.
pub fn encode_recoverable_payload(
    value: &RuntimeValue,
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
) -> Result<Vec<u8>> {
    let context = recoverable_payload_context(boundary);
    RecoverableBoundaryCodec::encode(value, expected, &context, heap)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "recoverable encode"))
}

/// Decodes an explicit recoverable payload slot with an artifact-authored
/// expected plan supplied by the caller.
pub fn decode_recoverable_payload(
    bytes: &[u8],
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let context = recoverable_payload_context(boundary);
    RecoverableBoundaryCodec::decode(bytes, expected, &context, heap)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "recoverable decode"))
}

/// Behavior-aware encode entry for same-service owner-internal explicit slots.
/// Callers must pass production hooks; the fail-closed hook intentionally does
/// not synthesize behavior recovery.
pub fn encode_recoverable_payload_with_behavior(
    value: &RuntimeValue,
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<Vec<u8>> {
    let context = recoverable_payload_context(boundary);
    RecoverableBoundaryCodec::encode_with_behavior(value, expected, &context, heap, behavior_hooks)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "recoverable encode"))
}

/// Behavior-aware decode entry for same-service owner-internal explicit slots.
/// Untrusted/cross-service boundaries still reject behavior before hooks run.
pub fn decode_recoverable_payload_with_behavior(
    bytes: &[u8],
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<RuntimeValue> {
    let context = recoverable_payload_context(boundary);
    RecoverableBoundaryCodec::decode_with_behavior(bytes, expected, &context, heap, behavior_hooks)
        .map_err(|error| attach_payload_boundary_context(error, boundary, "recoverable decode"))
}

pub fn recoverable_payload_context(
    boundary: &PayloadBoundary,
) -> RuntimeRecoverableBoundaryContext {
    let mut context = RuntimeRecoverableBoundaryContext::new(
        recoverable_payload_kind(boundary.kind()),
        recoverable_trust_boundary(boundary.trust()),
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    if let Some(origin) = boundary.origin_service() {
        context = context.with_origin_service(recoverable_service_ref(origin));
    }
    if let Some(target) = boundary.target_service() {
        context = context.with_target_service(recoverable_service_ref(target));
    }
    context
}

fn recoverable_payload_kind(
    kind: crate::payload::PayloadBoundaryKind,
) -> RuntimeRecoverableBoundaryKind {
    match kind {
        crate::payload::PayloadBoundaryKind::TaskDispatchPayload => {
            RuntimeRecoverableBoundaryKind::TaskDispatchPayload
        }
        crate::payload::PayloadBoundaryKind::QueueWorkItemPayload => {
            RuntimeRecoverableBoundaryKind::QueueWorkItemPayload
        }
        crate::payload::PayloadBoundaryKind::RuntimeWirePayload => {
            RuntimeRecoverableBoundaryKind::RuntimeWirePayload
        }
        crate::payload::PayloadBoundaryKind::OutboundServiceCall
        | crate::payload::PayloadBoundaryKind::InboundServiceCall
        | crate::payload::PayloadBoundaryKind::ServiceResponse => {
            RuntimeRecoverableBoundaryKind::ServicePayload
        }
        crate::payload::PayloadBoundaryKind::PublicApiPayload => {
            RuntimeRecoverableBoundaryKind::PublicApiPayload
        }
        crate::payload::PayloadBoundaryKind::MaterializationPayload => {
            RuntimeRecoverableBoundaryKind::MaterializationPayload
        }
        crate::payload::PayloadBoundaryKind::RuntimeInternal => {
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload
        }
        crate::payload::PayloadBoundaryKind::WebsocketRequest
        | crate::payload::PayloadBoundaryKind::StreamItem => {
            RuntimeRecoverableBoundaryKind::RecoverableEnvelopeSlot
        }
    }
}

fn encode_payload_plan_inner(
    value: &RuntimeValue,
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
) -> Result<Vec<u8>> {
    let mut encoder = PayloadEncoder {
        output: Vec::with_capacity(128),
        boundary,
        heap,
    };
    encoder.output.extend_from_slice(MAGIC);
    encoder.output.push(VERSION);
    encoder.encode_typed(value, plan)?;
    Ok(encoder.output)
}

fn decode_payload_plan_inner(
    bytes: &[u8],
    plan: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    if bytes.len() < 5 || &bytes[0..4] != MAGIC {
        return Err(RuntimeError::Decode(
            "runtime payload bytes missing SKPV magic".to_string(),
        ));
    }
    if bytes[4] != VERSION {
        return Err(RuntimeError::Decode(format!(
            "unsupported runtime payload version {}",
            bytes[4]
        )));
    }
    let mut decoder = PayloadDecoder {
        input: bytes,
        offset: 5,
        boundary,
        heap,
    };
    let value = decoder.decode_typed(plan)?;
    if decoder.offset != bytes.len() {
        return Err(RuntimeError::Decode(format!(
            "runtime payload has {} trailing byte(s)",
            bytes.len() - decoder.offset
        )));
    }
    Ok(value)
}

fn attach_payload_boundary_context(
    error: RuntimeError,
    boundary: &PayloadBoundary,
    operation: &str,
) -> RuntimeError {
    let message = format!(
        "runtime payload {operation} failed at {}: {error}",
        boundary.diagnostic_label()
    );
    match error {
        RuntimeError::Decode(_) | RuntimeError::DecodeTarget { .. } => {
            RuntimeError::Decode(message)
        }
        RuntimeError::Unsupported(_) => RuntimeError::Unsupported(message),
        other => other,
    }
}

struct PayloadEncoder<'a> {
    output: Vec<u8>,
    boundary: &'a PayloadBoundary,
    heap: &'a RequestHeap,
}

impl PayloadEncoder<'_> {
    fn encode_typed(
        &mut self,
        value: &RuntimeValue,
        expected_type: &RuntimeTypePlan,
    ) -> Result<()> {
        if let Some(value) = self.interface_value(value)? {
            return Err(interface_recoverable_envelope_encode_error(
                value,
                expected_type,
                self.boundary,
            ));
        }

        let codec =
            RuntimeBoundaryCodec::new(expected_type, BoundaryUse::RuntimeBinary, "runtime binary");
        match codec.typed_node()? {
            BoundaryTypeNode::Nullable(inner) => {
                if matches!(value, RuntimeValue::Null) {
                    self.output.push(0);
                    return Ok(());
                } else {
                    self.output.push(1);
                    self.encode_typed(value, inner)
                }
            }
            BoundaryTypeNode::Union(types) => {
                if types.len() > u8::MAX as usize + 1 {
                    return Err(RuntimeError::Decode(format!(
                        "runtime payload union has {} branches; maximum is 256",
                        types.len()
                    )));
                } else {
                    let mut errors = Vec::new();
                    for (index, ty) in types.iter().enumerate() {
                        let checkpoint = self.output.len();
                        match self.encode_typed(value, ty) {
                            Ok(()) => {
                                self.output.insert(checkpoint, index as u8);
                                return Ok(());
                            }
                            Err(error) => {
                                self.output.truncate(checkpoint);
                                errors.push(error.to_string());
                            }
                        }
                    }
                    Err(RuntimeError::Decode(format!(
                        "runtime payload union value did not match any branch: {}",
                        errors.join("; ")
                    )))
                }
            }
            BoundaryTypeNode::LiteralString(literal) => match value {
                RuntimeValue::String(actual) if actual.as_str() == literal => {
                    self.write_string(actual)
                }
                _ => Err(RuntimeError::Decode(format!(
                    "expected runtime literal string {literal:?}"
                ))),
            },
            BoundaryTypeNode::Json | BoundaryTypeNode::JsonObject => {
                self.encode_any(value, expected_type)
            }
            BoundaryTypeNode::Bytes => {
                let bytes = RuntimeValueGraph::new(self.heap).bytes(value)?;
                self.write_tag(TAG_BYTES);
                self.write_bytes_raw(bytes)
            }
            BoundaryTypeNode::Date => match value {
                RuntimeValue::Date(ms) => {
                    date_value::validate_epoch_millis(*ms, "runtime payload Date")?;
                    self.write_tag(TAG_DATE);
                    self.write_i64(*ms);
                    Ok(())
                }
                _ => Err(RuntimeError::Decode("expected runtime Date".to_string())),
            },
            BoundaryTypeNode::String => match value {
                RuntimeValue::String(text) => self.write_string(text),
                _ => Err(RuntimeError::Decode("expected runtime string".to_string())),
            },
            BoundaryTypeNode::Bool => match value {
                RuntimeValue::Bool(false) => {
                    self.write_tag(TAG_BOOL_FALSE);
                    Ok(())
                }
                RuntimeValue::Bool(true) => {
                    self.write_tag(TAG_BOOL_TRUE);
                    Ok(())
                }
                _ => Err(RuntimeError::Decode("expected runtime bool".to_string())),
            },
            BoundaryTypeNode::Integer | BoundaryTypeNode::Number => match value {
                RuntimeValue::Number(number) if number.is_finite() => {
                    self.write_tag(TAG_NUMBER);
                    self.output.extend_from_slice(&number.to_le_bytes());
                    Ok(())
                }
                _ => Err(RuntimeError::Decode("expected runtime number".to_string())),
            },
            BoundaryTypeNode::Null => match value {
                RuntimeValue::Null => {
                    self.write_tag(TAG_NULL);
                    Ok(())
                }
                _ => Err(RuntimeError::Decode("expected runtime null".to_string())),
            },
            BoundaryTypeNode::Stream(_) => Err(RuntimeError::Unsupported(
                "runtime payload codec does not encode Stream handles".to_string(),
            )),
            BoundaryTypeNode::Array(item_type) => {
                let items = RuntimeValueGraph::new(self.heap).array(value)?;
                self.write_tag(TAG_ARRAY);
                self.write_len(items.len())?;
                for item in items {
                    self.encode_typed(item, item_type)?;
                }
                Ok(())
            }
            BoundaryTypeNode::Map {
                key: key_type,
                value: value_type,
            } => {
                let map = RuntimeValueGraph::new(self.heap).map(value)?;
                self.write_tag(TAG_MAP);
                self.write_len(map.len())?;
                for (key, item) in map {
                    self.write_runtime_key(key, key_type)?;
                    self.encode_typed(item, value_type)?;
                }
                Ok(())
            }
            BoundaryTypeNode::Record { fields } => {
                let record = RuntimeRecordFieldSource::from_value(
                    self.heap,
                    value,
                    expected_type,
                    self.boundary,
                )?;
                record.reject_extra_fields(fields)?;
                self.write_tag(TAG_OBJECT);
                let present_fields = fields
                    .iter()
                    .filter(|field| record.get(&field.name).is_some())
                    .collect::<Vec<_>>();
                self.write_len(present_fields.len())?;
                for field in present_fields {
                    self.write_string_raw(&field.name)?;
                    match record.get(&field.name) {
                        Some(value) => self.encode_typed(value, &field.ty)?,
                        None => unreachable!("present_fields only contains object fields"),
                    }
                }
                for field in fields {
                    if field.required && record.get(&field.name).is_none() {
                        return Err(RuntimeError::Decode(format!(
                            "record field {} is required",
                            field.name
                        )));
                    }
                }
                Ok(())
            }
            BoundaryTypeNode::Unknown => Err(RuntimeError::InvalidArtifact(format!(
                "unsupported runtime payload type descriptor {}",
                expected_type.label()
            ))),
        }
    }

    fn encode_any(&mut self, value: &RuntimeValue, expected_type: &RuntimeTypePlan) -> Result<()> {
        match value {
            RuntimeValue::Null => {
                self.write_tag(TAG_NULL);
                Ok(())
            }
            RuntimeValue::Bool(false) => {
                self.write_tag(TAG_BOOL_FALSE);
                Ok(())
            }
            RuntimeValue::Bool(true) => {
                self.write_tag(TAG_BOOL_TRUE);
                Ok(())
            }
            RuntimeValue::Number(number) if number.is_finite() => {
                self.write_tag(TAG_NUMBER);
                self.output.extend_from_slice(&number.to_le_bytes());
                Ok(())
            }
            RuntimeValue::Number(_) => Err(RuntimeError::Decode(
                "cannot encode non-finite number".to_string(),
            )),
            RuntimeValue::String(text) => self.write_string(text),
            RuntimeValue::Date(ms) => {
                date_value::validate_epoch_millis(*ms, "runtime payload Date")?;
                self.write_tag(TAG_DATE);
                self.write_i64(*ms);
                Ok(())
            }
            RuntimeValue::ActorRef(actor_ref) => Err(RuntimeError::Decode(format!(
                "actor ref {} cannot be encoded in runtime payload",
                actor_ref.actor_type_identity()
            ))),
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Bytes(bytes) => {
                    self.write_tag(TAG_BYTES);
                    self.write_bytes_raw(bytes.as_slice())
                }
                HeapNode::Array(items) => {
                    self.write_tag(TAG_ARRAY);
                    self.write_len(items.len())?;
                    for item in items {
                        self.encode_any(item, expected_type)?;
                    }
                    Ok(())
                }
                HeapNode::Object(object) => {
                    self.write_tag(TAG_OBJECT);
                    self.write_len(object.fields().len())?;
                    for (key, item) in object.fields() {
                        reject_reserved_legacy_metadata_key(key)?;
                        self.write_string_raw(key)?;
                        self.encode_any(item, expected_type)?;
                    }
                    Ok(())
                }
                HeapNode::Map(map) => {
                    self.write_tag(TAG_MAP);
                    self.write_len(map.len())?;
                    for (key, item) in map {
                        self.write_runtime_key_any(key)?;
                        self.encode_any(item, expected_type)?;
                    }
                    Ok(())
                }
                HeapNode::Interface(value) => Err(interface_recoverable_envelope_encode_error(
                    value,
                    expected_type,
                    self.boundary,
                )),
                HeapNode::Exception(_) => Err(RuntimeError::Decode(
                    "request-local exception cannot be encoded in a runtime payload".to_string(),
                )),
            },
        }
    }

    fn interface_value(&self, value: &RuntimeValue) -> Result<Option<&InterfaceValue>> {
        let RuntimeValue::Heap(handle) = value else {
            return Ok(None);
        };
        match self.heap.get(*handle)? {
            HeapNode::Interface(value) => Ok(Some(value)),
            _ => Ok(None),
        }
    }

    fn write_runtime_key(
        &mut self,
        key: &RuntimeValueKey,
        key_type: &RuntimeTypePlan,
    ) -> Result<()> {
        let key_shape = runtime_map_key_shape(key_type)?;
        let encoded = key_shape.encode_runtime_key(key)?;
        reject_reserved_legacy_metadata_key(encoded)?;
        self.write_string_raw(encoded)
    }

    fn write_runtime_key_any(&mut self, key: &RuntimeValueKey) -> Result<()> {
        match key {
            RuntimeValueKey::String(value) => {
                reject_reserved_legacy_metadata_key(value)?;
                self.output.push(0);
                self.write_string_raw(value)
            }
        }
    }

    fn write_tag(&mut self, tag: u8) {
        self.output.push(tag);
    }

    fn write_i64(&mut self, value: i64) {
        self.output.extend_from_slice(&value.to_le_bytes());
    }

    fn write_string(&mut self, value: &str) -> Result<()> {
        self.write_tag(TAG_STRING);
        self.write_string_raw(value)
    }

    fn write_string_raw(&mut self, value: &str) -> Result<()> {
        self.write_bytes_raw(value.as_bytes())
    }

    fn write_bytes_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.write_len(bytes.len())?;
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    fn write_len(&mut self, len: usize) -> Result<()> {
        let len = u32::try_from(len)
            .map_err(|_| RuntimeError::Decode("runtime payload length exceeds u32".to_string()))?;
        self.output.extend_from_slice(&len.to_le_bytes());
        Ok(())
    }
}

enum RuntimeRecordFieldSource<'a> {
    Object(&'a RuntimeObjectFields),
    Map(&'a RuntimeMap),
}

impl RuntimeRecordFieldSource<'_> {
    fn from_value<'a>(
        heap: &'a RequestHeap,
        value: &RuntimeValue,
        expected_type: &RuntimeTypePlan,
        boundary: &PayloadBoundary,
    ) -> Result<RuntimeRecordFieldSource<'a>> {
        match value {
            RuntimeValue::Heap(handle) => match heap.get(*handle)? {
                HeapNode::Object(object) => Ok(RuntimeRecordFieldSource::Object(object.fields())),
                HeapNode::Map(map) => Ok(RuntimeRecordFieldSource::Map(map)),
                HeapNode::Interface(value) => Err(interface_recoverable_envelope_encode_error(
                    value,
                    expected_type,
                    boundary,
                )),
                _ => Err(RuntimeError::Decode("expected runtime object".to_string())),
            },
            _ => Err(RuntimeError::Decode("expected runtime object".to_string())),
        }
    }

    fn get(&self, name: &str) -> Option<&RuntimeValue> {
        match self {
            Self::Object(fields) => fields.get(name),
            Self::Map(map) => map.get(&RuntimeValueKey::string(name)),
        }
    }

    fn reject_extra_fields(&self, fields: &[RuntimeRecordFieldPlan]) -> Result<()> {
        let allowed = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        match self {
            Self::Object(object) => {
                for key in object.keys() {
                    reject_reserved_legacy_metadata_key(key)?;
                    if !allowed.contains(key.as_str()) {
                        return Err(RuntimeError::Decode(format!(
                            "record field {key} is not declared by descriptor"
                        )));
                    }
                }
            }
            Self::Map(map) => {
                for key in map.keys() {
                    let key = key.string_payload();
                    reject_reserved_legacy_metadata_key(key)?;
                    if !allowed.contains(key) {
                        return Err(RuntimeError::Decode(format!(
                            "record field {key} is not declared by descriptor"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn interface_recoverable_envelope_encode_error(
    value: &InterfaceValue,
    expected_type: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
) -> RuntimeError {
    if let crate::runtime_value::InterfaceCarrier::CallbackCapability(carrier) = value.carrier() {
        let context = runtime_binary_recoverable_context(boundary);
        let expected =
            RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                expected_type,
            );
        return crate::persistent::callback_capability_not_recoverable_error(
            carrier, "$", &context, &expected,
        );
    }
    interface_recoverable_envelope_error(
        RecoverableBoundaryErrorCode::UnsupportedEncode,
        "encode",
        expected_type,
        boundary,
        Some(value),
    )
}

fn interface_recoverable_envelope_decode_error(
    expected_type: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
) -> RuntimeError {
    interface_recoverable_envelope_error(
        RecoverableBoundaryErrorCode::UnsupportedDecode,
        "decode",
        expected_type,
        boundary,
        None,
    )
}

fn interface_recoverable_envelope_error(
    code: RecoverableBoundaryErrorCode,
    operation: &str,
    expected_type: &RuntimeTypePlan,
    boundary: &PayloadBoundary,
    value: Option<&InterfaceValue>,
) -> RuntimeError {
    let context = runtime_binary_recoverable_context(boundary);
    let expected =
        RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
            expected_type,
        );
    let value_label = value
        .map(InterfaceValue::diagnostic_label)
        .unwrap_or_else(|| "any I value".to_string());
    RecoverableBoundaryError::new(
        code,
        format!(
            "recoverable {operation} is unsupported for {} boundary with {} storage lane and expected type {}; {value_label} requires a recoverable envelope and real envelope encoding is not implemented",
            context.kind,
            context.storage_lane,
            expected.diagnostic_label()
        ),
        &context,
        &expected,
    )
    .into()
}

fn runtime_binary_recoverable_context(
    boundary: &PayloadBoundary,
) -> RuntimeRecoverableBoundaryContext {
    let mut context = RuntimeRecoverableBoundaryContext::new(
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        recoverable_trust_boundary(boundary.trust()),
        RuntimeRecoverableStorageLane::RecoverableEnvelope,
    )
    .with_explicit_recoverable_slot();
    if let Some(origin) = boundary.origin_service() {
        context = context.with_origin_service(recoverable_service_ref(origin));
    }
    if let Some(target) = boundary.target_service() {
        context = context.with_target_service(recoverable_service_ref(target));
    }
    context
}

fn recoverable_trust_boundary(trust: PayloadTrust) -> RuntimeRecoverableTrustBoundary {
    match trust {
        PayloadTrust::OwnerInternal => RuntimeRecoverableTrustBoundary::OwnerInternal,
        PayloadTrust::CrossService => RuntimeRecoverableTrustBoundary::CrossService,
        PayloadTrust::ExternalUntrusted => RuntimeRecoverableTrustBoundary::ExternalUntrusted,
    }
}

fn recoverable_service_ref(service: &PayloadServiceRef) -> RuntimeRecoverableServiceRef {
    RuntimeRecoverableServiceRef {
        service_id: service.service_id().to_string(),
        version: service.version().map(str::to_string),
        build_id: service.build_id().map(str::to_string),
    }
}

struct PayloadDecoder<'a> {
    input: &'a [u8],
    offset: usize,
    boundary: &'a PayloadBoundary,
    heap: &'a mut RequestHeap,
}

impl PayloadDecoder<'_> {
    fn decode_typed(&mut self, expected_type: &RuntimeTypePlan) -> Result<RuntimeValue> {
        let codec =
            RuntimeBoundaryCodec::new(expected_type, BoundaryUse::RuntimeBinary, "runtime binary");
        match codec.typed_node()? {
            BoundaryTypeNode::Nullable(inner) => match self.read_u8()? {
                0 => Ok(RuntimeValue::Null),
                1 => self.decode_typed(inner),
                tag => Err(RuntimeError::Decode(format!(
                    "runtime payload nullable discriminant must be 0 or 1, got {tag}"
                ))),
            },
            BoundaryTypeNode::Union(types) => {
                let branch = self.read_u8()? as usize;
                let Some(ty) = types.get(branch) else {
                    return Err(RuntimeError::Decode(format!(
                        "runtime payload union branch {branch} is out of range"
                    )));
                };
                self.decode_typed(ty)
            }
            BoundaryTypeNode::LiteralString(literal) => {
                let value = self.decode_string_value()?;
                match value {
                    RuntimeValue::String(actual) if actual.as_str() == literal => {
                        Ok(RuntimeValue::String(actual))
                    }
                    _ => Err(RuntimeError::Decode(format!(
                        "expected runtime literal string {literal:?}"
                    ))),
                }
            }
            BoundaryTypeNode::Json | BoundaryTypeNode::JsonObject => self.decode_any(expected_type),
            BoundaryTypeNode::Bytes => {
                self.expect_tag(TAG_BYTES)?;
                let bytes = self.read_bytes_raw()?.to_vec();
                Ok(RuntimeValue::Heap(self.heap.alloc_bytes(bytes)?))
            }
            BoundaryTypeNode::Date => self.decode_date_value(),
            BoundaryTypeNode::String => self.decode_string_value(),
            BoundaryTypeNode::Bool => match self.read_tag()? {
                TAG_BOOL_FALSE => Ok(RuntimeValue::Bool(false)),
                TAG_BOOL_TRUE => Ok(RuntimeValue::Bool(true)),
                tag => Err(RuntimeError::Decode(format!(
                    "expected runtime bool tag, got {tag}"
                ))),
            },
            BoundaryTypeNode::Integer | BoundaryTypeNode::Number => self.decode_number_value(),
            BoundaryTypeNode::Null => {
                self.expect_tag(TAG_NULL)?;
                Ok(RuntimeValue::Null)
            }
            BoundaryTypeNode::Stream(_) => Err(RuntimeError::Unsupported(
                "runtime payload codec does not decode Stream handles".to_string(),
            )),
            BoundaryTypeNode::Array(item_type) => {
                self.expect_tag(TAG_ARRAY)?;
                let len = self.read_len()?;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.decode_typed(item_type)?);
                }
                Ok(RuntimeValue::Heap(self.heap.alloc_array(items)?))
            }
            BoundaryTypeNode::Map {
                key: key_type,
                value: value_type,
            } => {
                self.expect_tag(TAG_MAP)?;
                let len = self.read_len()?;
                let mut map = RuntimeMap::new();
                for _ in 0..len {
                    let key = self.read_runtime_key(key_type)?;
                    let value = self.decode_typed(value_type)?;
                    map.insert(key, value);
                }
                Ok(RuntimeValue::Heap(self.heap.alloc_map(map)?))
            }
            BoundaryTypeNode::Record { fields } => {
                self.expect_tag(TAG_OBJECT)?;
                let len = self.read_len()?;
                let fields_by_name = fields
                    .iter()
                    .map(|field| (field.name.as_str(), field))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let mut object = RuntimeObjectFields::new();
                for _ in 0..len {
                    let name = self.read_string_raw()?;
                    reject_reserved_legacy_metadata_key(&name)?;
                    let Some(field) = fields_by_name.get(name.as_str()) else {
                        return Err(RuntimeError::Decode(format!(
                            "runtime payload record field {name} is not in descriptor"
                        )));
                    };
                    object.insert(field.name.clone(), self.decode_typed(&field.ty)?);
                }
                for field in fields {
                    if field.required && !object.contains_key(&field.name) {
                        return Err(RuntimeError::Decode(format!(
                            "record field {} is required",
                            field.name
                        )));
                    }
                }
                Ok(RuntimeValue::Heap(
                    self.heap.alloc_object(RuntimeObject::unshaped(object))?,
                ))
            }
            BoundaryTypeNode::Unknown => Err(RuntimeError::InvalidArtifact(format!(
                "unsupported runtime payload type descriptor {}",
                expected_type.label()
            ))),
        }
    }

    fn decode_any(&mut self, expected_type: &RuntimeTypePlan) -> Result<RuntimeValue> {
        match self.read_tag()? {
            TAG_NULL => Ok(RuntimeValue::Null),
            TAG_BOOL_FALSE => Ok(RuntimeValue::Bool(false)),
            TAG_BOOL_TRUE => Ok(RuntimeValue::Bool(true)),
            TAG_NUMBER => {
                let bytes = self.read_exact(8)?;
                Ok(RuntimeValue::Number(f64::from_le_bytes(
                    bytes.try_into().expect("slice length checked"),
                )))
            }
            TAG_STRING => Ok(RuntimeValue::String(self.read_string_raw()?)),
            TAG_DATE => self.decode_date_payload(),
            TAG_BYTES => {
                let bytes = self.read_bytes_raw()?.to_vec();
                Ok(RuntimeValue::Heap(self.heap.alloc_bytes(bytes)?))
            }
            TAG_ARRAY => {
                let len = self.read_len()?;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.decode_any(expected_type)?);
                }
                Ok(RuntimeValue::Heap(self.heap.alloc_array(items)?))
            }
            TAG_OBJECT => {
                let len = self.read_len()?;
                let mut fields = RuntimeObjectFields::new();
                for _ in 0..len {
                    let key = self.read_string_raw()?;
                    reject_reserved_legacy_metadata_key(&key)?;
                    let value = self.decode_any(expected_type)?;
                    fields.insert(key, value);
                }
                Ok(RuntimeValue::Heap(
                    self.heap.alloc_object(RuntimeObject::unshaped(fields))?,
                ))
            }
            TAG_MAP => {
                let len = self.read_len()?;
                let mut map = RuntimeMap::new();
                for _ in 0..len {
                    let key = self.read_runtime_key_any()?;
                    let value = self.decode_any(expected_type)?;
                    map.insert(key, value);
                }
                Ok(RuntimeValue::Heap(self.heap.alloc_map(map)?))
            }
            TAG_INTERFACE => Err(interface_recoverable_envelope_decode_error(
                expected_type,
                self.boundary,
            )),
            tag => Err(RuntimeError::Decode(format!(
                "unknown runtime payload tag {tag}"
            ))),
        }
    }

    fn decode_string_value(&mut self) -> Result<RuntimeValue> {
        self.expect_tag(TAG_STRING)?;
        Ok(RuntimeValue::String(self.read_string_raw()?))
    }

    fn decode_number_value(&mut self) -> Result<RuntimeValue> {
        self.expect_tag(TAG_NUMBER)?;
        let bytes = self.read_exact(8)?;
        let value = f64::from_le_bytes(bytes.try_into().expect("slice length checked"));
        if !value.is_finite() {
            return Err(RuntimeError::Decode(
                "runtime payload number must be finite".to_string(),
            ));
        }
        Ok(RuntimeValue::Number(value))
    }

    fn decode_date_value(&mut self) -> Result<RuntimeValue> {
        self.expect_tag(TAG_DATE)?;
        self.decode_date_payload()
    }

    fn decode_date_payload(&mut self) -> Result<RuntimeValue> {
        let bytes = self.read_exact(8)?;
        let value = i64::from_le_bytes(bytes.try_into().expect("slice length checked"));
        date_value::validate_epoch_millis(value, "runtime payload Date")?;
        Ok(RuntimeValue::Date(value))
    }

    fn read_runtime_key(&mut self, key_type: &RuntimeTypePlan) -> Result<RuntimeValueKey> {
        let value = self.read_string_raw()?;
        reject_reserved_legacy_metadata_key(&value)?;
        let key_shape = runtime_map_key_shape(key_type)?;
        Ok(key_shape.decode_runtime_key(value))
    }

    fn read_runtime_key_any(&mut self) -> Result<RuntimeValueKey> {
        match self.read_u8()? {
            0 => {
                let key = self.read_string_raw()?;
                reject_reserved_legacy_metadata_key(&key)?;
                Ok(RuntimeValueKey::string(key))
            }
            tag => Err(RuntimeError::Decode(format!(
                "unknown runtime payload map key tag {tag}"
            ))),
        }
    }

    fn expect_tag(&mut self, expected: u8) -> Result<()> {
        let actual = self.read_tag()?;
        if actual == expected {
            Ok(())
        } else {
            Err(RuntimeError::Decode(format!(
                "runtime payload expected tag {expected}, got {actual}"
            )))
        }
    }

    fn read_tag(&mut self) -> Result<u8> {
        self.read_u8()
    }

    fn read_u8(&mut self) -> Result<u8> {
        let byte = self
            .input
            .get(self.offset)
            .copied()
            .ok_or_else(|| RuntimeError::Decode("runtime payload ended early".to_string()))?;
        self.offset += 1;
        Ok(byte)
    }

    fn read_len(&mut self) -> Result<usize> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("slice length checked")) as usize)
    }

    fn read_string_raw(&mut self) -> Result<String> {
        let bytes = self.read_bytes_raw()?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|error| {
                RuntimeError::Decode(format!("runtime payload string is not UTF-8: {error}"))
            })
    }

    fn read_bytes_raw(&mut self) -> Result<&[u8]> {
        let len = self.read_len()?;
        self.read_exact(len)
    }

    fn read_exact(&mut self, len: usize) -> Result<&[u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| RuntimeError::Decode("runtime payload length overflow".to_string()))?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or_else(|| RuntimeError::Decode("runtime payload ended early".to_string()))?;
        self.offset = end;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests;
