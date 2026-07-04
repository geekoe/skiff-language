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
/// This is the non-DB integration helper for spawn/queue/runtime-wire/public/
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
        crate::payload::PayloadBoundaryKind::SpawnPayload => {
            RuntimeRecoverableBoundaryKind::SpawnPayload
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
mod tests {
    use std::cell::Cell;

    use serde_json::json;
    use skiff_runtime_model::addr::ExecutableAddr;
    use skiff_runtime_model::recoverable::{
        InterfaceValueState, LocalConcreteOwner, NativeAdapterOwner, NativeHandleState,
        NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope, RecoverableField,
        RecoverableNode, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
        RecoverableVariantIdentity, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    };

    use super::*;
    use crate::payload::PayloadBoundaryKind;
    use crate::recoverable::{
        FailClosedRecoverableBehaviorHooks, RecoverableBehaviorHooks,
        RecoverableEncodedLocalInterfaceSelf, RecoverableInterfaceConformanceRequest,
        RecoverableInterfaceMethodTableRequest, RecoverableLocalInterfaceEncodeRequest,
        RecoverableLocalInterfaceRestoreRequest, RecoverableRemoteInterfaceCarrierRequest,
        RecoverableRestoredLocalInterfaceSelf,
    };
    use crate::runtime_value::{
        InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
        InterfaceReceiverCallAbi, InterfaceValue, RemoteOperationSlot, RemoteOperationTable,
        RuntimeBytes,
    };
    use crate::type_descriptor::{RuntimeTypeNode, RuntimeTypePlanDescriptorExt};

    fn test_boundary() -> PayloadBoundary {
        PayloadBoundary::runtime_internal()
    }

    fn any_interface_plan() -> RuntimeTypePlan {
        RuntimeTypePlan {
            label: "anyInterface".to_string(),
            named_type_name: None,
            identity: Default::default(),
            node: RuntimeTypeNode::Unknown,
        }
    }

    const READER_INTERFACE: &str = "pkg.Reader";
    const READER_PROJECTION: &str = "projection:pkg.Reader:pkg.ReaderImpl";
    const READER_METHOD: &str = "method:pkg.Reader:read";
    const READER_IMPL: &str = "pkg.ReaderImpl";

    fn string_plan() -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "string",
            "args": []
        }))
        .expect("string plan should build")
    }

    fn any_reader_expected() -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan::any_interface(
            "any pkg.Reader",
            READER_INTERFACE,
            READER_PROJECTION,
        )
    }

    fn recoverable_unresolved_expected() -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan::unresolved("recoverable")
    }

    fn test_method_table(
        interface_identity: &str,
        projection_identity: &str,
    ) -> InterfaceMethodTable {
        InterfaceMethodTable::new(
            projection_identity.to_string(),
            interface_identity.to_string(),
            vec![InterfaceMethodSlot::new(
                0,
                READER_METHOD.to_string(),
                InterfaceMethodTarget::LocalExecutable {
                    executable: ExecutableAddr::service(0, 7),
                    receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                },
            )],
        )
    }

    fn test_remote_operation_table() -> RemoteOperationTable {
        RemoteOperationTable::new(
            "remote:reader".to_string(),
            READER_INTERFACE.to_string(),
            vec![RemoteOperationSlot::new(
                0,
                READER_METHOD.to_string(),
                "operation:reader.read".to_string(),
            )],
        )
    }

    fn local_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
        RuntimeValue::Heap(
            heap.alloc_interface(InterfaceValue::new(
                READER_INTERFACE.to_string(),
                InterfaceCarrier::Local {
                    concrete_type: READER_IMPL.to_string(),
                    method_table: test_method_table(READER_INTERFACE, READER_PROJECTION),
                    payload: RuntimeValue::String("Ada".to_string()),
                },
            ))
            .expect("local interface should allocate"),
        )
    }

    fn remote_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
        RuntimeValue::Heap(
            heap.alloc_interface(InterfaceValue::new(
                READER_INTERFACE.to_string(),
                InterfaceCarrier::Remote {
                    dependency_ref: "dep:reader".to_string(),
                    public_instance_key: "reader#1".to_string(),
                    operations: test_remote_operation_table(),
                },
            ))
            .expect("remote interface should allocate"),
        )
    }

    fn recoverable_string_node(value: &str) -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::String,
            RecoverableState::String(value.to_string()),
        )
    }

    fn local_concrete_self_node(value: &str) -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::NominalObject,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::LocalConcrete {
                owner: LocalConcreteOwner::Service,
                concrete_type_identity: READER_IMPL.to_string(),
            },
            state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
                fields: vec![RecoverableField {
                    field_identity: "value".to_string(),
                    value: recoverable_string_node(value),
                }],
            }),
        }
    }

    fn interface_node() -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_self_node("Ada")),
            }),
        )
    }

    fn native_handle_node() -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::NativeHandle,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: "std.FileHandleAdapter".to_string(),
                adapter_schema_version: "1".to_string(),
                owner: NativeAdapterOwner::Builtin,
                native_type_identity: "std.FileHandle".to_string(),
            },
            state: RecoverableState::NativeHandle(NativeHandleState {
                durable_state: Box::new(recoverable_string_node("durable")),
            }),
        }
    }

    fn native_adapter_plain_node() -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::String,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: "std.StringAdapter".to_string(),
                adapter_schema_version: "1".to_string(),
                owner: NativeAdapterOwner::Builtin,
                native_type_identity: "std.StringLike".to_string(),
            },
            state: RecoverableState::String("native-adapter".to_string()),
        }
    }

    fn record_node(field_identity: &str, value: RecoverableNode) -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::Record,
            RecoverableState::Record(vec![RecoverableField {
                field_identity: field_identity.to_string(),
                value,
            }]),
        )
    }

    fn canonical_envelope_bytes(node: RecoverableNode) -> Vec<u8> {
        RecoverableEnvelope::new(node)
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("recoverable envelope should canonical encode")
    }

    #[derive(Default)]
    struct TestBehaviorHooks {
        encode_calls: Cell<usize>,
        restore_calls: Cell<usize>,
        conformance_calls: Cell<usize>,
        table_calls: Cell<usize>,
        remote_table_calls: Cell<usize>,
    }

    impl RecoverableBehaviorHooks for TestBehaviorHooks {
        fn encode_local_interface_self(
            &self,
            request: RecoverableLocalInterfaceEncodeRequest<'_>,
            _heap: &RequestHeap,
        ) -> Result<Option<RecoverableEncodedLocalInterfaceSelf>> {
            self.encode_calls.set(self.encode_calls.get() + 1);
            let value = match request.payload {
                RuntimeValue::String(value) => value.as_str(),
                RuntimeValue::Null => "null",
                _ => "unsupported",
            };
            Ok(Some(RecoverableEncodedLocalInterfaceSelf {
                method_projection_identity: request.method_table.id().to_string(),
                self_node: local_concrete_self_node(value),
            }))
        }

        fn restore_local_interface_self(
            &self,
            request: RecoverableLocalInterfaceRestoreRequest<'_>,
            _heap: &mut RequestHeap,
        ) -> Result<Option<RecoverableRestoredLocalInterfaceSelf>> {
            self.restore_calls.set(self.restore_calls.get() + 1);
            let RecoverableCodeIdentity::LocalConcrete {
                concrete_type_identity,
                ..
            } = &request.self_node.code_identity
            else {
                return Ok(None);
            };
            let RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) =
                &request.self_node.state
            else {
                return Ok(None);
            };
            let value = fields
                .iter()
                .find(|field| field.field_identity == "value")
                .and_then(|field| match &field.value.state {
                    RecoverableState::String(value) => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            Ok(Some(RecoverableRestoredLocalInterfaceSelf {
                concrete_type_identity: concrete_type_identity.clone(),
                payload: RuntimeValue::String(value),
            }))
        }

        fn concrete_type_conforms_to_interface(
            &self,
            request: RecoverableInterfaceConformanceRequest<'_>,
        ) -> Result<bool> {
            self.conformance_calls.set(self.conformance_calls.get() + 1);
            Ok(request.concrete_type_identity == READER_IMPL
                && request.interface_identity == READER_INTERFACE
                && request.method_projection_identity == READER_PROJECTION)
        }

        fn rebuild_local_interface_method_table(
            &self,
            _request: RecoverableInterfaceMethodTableRequest<'_>,
        ) -> Result<Option<InterfaceMethodTable>> {
            self.table_calls.set(self.table_calls.get() + 1);
            Ok(Some(test_method_table(READER_INTERFACE, READER_PROJECTION)))
        }

        fn rebuild_remote_interface_operation_table(
            &self,
            request: RecoverableRemoteInterfaceCarrierRequest<'_>,
        ) -> Result<Option<RemoteOperationTable>> {
            self.remote_table_calls
                .set(self.remote_table_calls.get() + 1);
            if request.carrier.dependency_ref == "dep:reader"
                && request.carrier.public_instance_key == "reader#1"
                && request.interface_identity == READER_INTERFACE
                && request.method_projection_identity == READER_PROJECTION
            {
                Ok(Some(test_remote_operation_table()))
            } else {
                Ok(None)
            }
        }
    }

    fn assert_interface_recoverable_envelope_error(
        error: RuntimeError,
        code: RecoverableBoundaryErrorCode,
    ) {
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error, got {error}");
        };
        assert_eq!(error.code(), code);
        assert_eq!(
            error.context().kind,
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload
        );
        assert_eq!(
            error.context().storage_lane,
            RuntimeRecoverableStorageLane::RecoverableEnvelope
        );
        assert!(error.context().explicit_recoverable_slot);
        let message = error.message();
        assert!(
            message.contains("recoverable envelope")
                && message.contains("real envelope encoding is not implemented"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn payload_boundary_does_not_change_encoded_bytes() {
        let descriptor = json!({ "kind": "builtin", "name": "string", "args": [] });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
        let heap = RequestHeap::default();
        let value = RuntimeValue::String("Ada".to_string());
        let owner_internal = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let cross_service = PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            crate::payload::PayloadServiceRef::new("skiff.run/account").with_version("0.1.0"),
        );

        let owner_bytes = encode_payload_plan(&value, &plan, &owner_internal, &heap)
            .expect("owner-internal payload should encode");
        let cross_service_bytes = encode_payload_plan(&value, &plan, &cross_service, &heap)
            .expect("cross-service payload should encode");

        assert_eq!(owner_bytes, cross_service_bytes);
    }

    #[test]
    fn payload_codec_errors_include_boundary_context() {
        let descriptor = json!({ "kind": "builtin", "name": "string", "args": [] });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
        let heap = RequestHeap::default();
        let boundary = PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            crate::payload::PayloadServiceRef::new("skiff.run/registry").with_version("0.1.0"),
        );

        let error = encode_payload_plan(&RuntimeValue::Number(7.0), &plan, &boundary, &heap)
            .expect_err("number must not encode as string");
        let message = error.to_string();

        assert!(message.contains("kind=OutboundServiceCall"));
        assert!(message.contains("target=skiff.run/registry@0.1.0"));
    }

    #[test]
    fn spawn_and_queue_recoverable_payload_helpers_share_canonical_envelope() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "name": { "kind": "builtin", "name": "string", "args": [] },
                "score": { "kind": "builtin", "name": "number", "args": [] }
            }
        });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("plan should build");
        let mut heap = RequestHeap::default();
        let object = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("name".to_string(), RuntimeValue::String("Ada".to_string())),
                ("score".to_string(), RuntimeValue::Number(98.5)),
            ])))
            .expect("record should allocate");
        let value = RuntimeValue::Heap(object);
        let service =
            PayloadServiceRef::new("skiff.run/account").with_build_id("skiff-service-build-a");
        let spawn_boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload)
            .with_origin_service(service.clone());
        let queue_boundary =
            PayloadBoundary::owner_internal(PayloadBoundaryKind::QueueWorkItemPayload)
                .with_origin_service(service);

        let spawn_bytes = encode_recoverable_payload_plan(&value, &plan, &spawn_boundary, &heap)
            .expect("spawn recoverable payload should encode");
        let queue_bytes = encode_recoverable_payload_plan(&value, &plan, &queue_boundary, &heap)
            .expect("queue recoverable payload should encode");

        assert_eq!(spawn_bytes, queue_bytes);

        let mut decode_heap = RequestHeap::default();
        let decoded =
            decode_recoverable_payload_plan(&spawn_bytes, &plan, &spawn_boundary, &mut decode_heap)
                .expect("spawn recoverable payload should decode");
        let RuntimeValue::Heap(decoded_handle) = decoded else {
            panic!("decoded value should be a heap object");
        };
        let HeapNode::Object(decoded_object) = decode_heap
            .get(decoded_handle)
            .expect("decoded object resolves")
        else {
            panic!("decoded value should be an object");
        };
        assert_eq!(
            decoded_object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
        assert_eq!(
            decoded_object.fields().get("score"),
            Some(&RuntimeValue::Number(98.5))
        );
    }

    #[test]
    fn ordinary_payload_decode_rejects_recoverable_envelope_magic() {
        let plan = string_plan();
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let heap = RequestHeap::default();
        let bytes = encode_recoverable_payload_plan(
            &RuntimeValue::String("Ada".to_string()),
            &plan,
            &boundary,
            &heap,
        )
        .expect("recoverable payload should encode");

        let error = decode_payload_plan(&bytes, &plan, &boundary, &mut RequestHeap::default())
            .expect_err("ordinary runtime binary payload must not accept SKRE");

        assert!(error.to_string().contains("missing SKPV magic"));
    }

    #[test]
    fn public_cross_service_and_exported_materialization_plain_envelopes_roundtrip() {
        let plan = string_plan();
        let heap = RequestHeap::default();
        let value = RuntimeValue::String("plain".to_string());
        let boundaries = [
            PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
                .with_target_service(PayloadServiceRef::new("skiff.run/runtime-target")),
            PayloadBoundary::cross_service(
                PayloadBoundaryKind::OutboundServiceCall,
                PayloadServiceRef::new("skiff.run/registry"),
            ),
            PayloadBoundary::external_untrusted(PayloadBoundaryKind::PublicApiPayload),
            PayloadBoundary::external_untrusted(PayloadBoundaryKind::MaterializationPayload),
        ];

        for boundary in boundaries {
            let bytes = encode_recoverable_payload_plan(&value, &plan, &boundary, &heap)
                .expect("plain recoverable envelope should encode");
            let decoded = decode_recoverable_payload_plan(
                &bytes,
                &plan,
                &boundary,
                &mut RequestHeap::default(),
            )
            .expect("plain recoverable envelope should decode");
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn owner_internal_service_explicit_slot_roundtrips_local_interface_with_hooks() {
        let mut heap = RequestHeap::default();
        let value = local_interface_runtime_value(&mut heap);
        let expected = any_reader_expected();
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::InboundServiceCall)
            .with_origin_service(PayloadServiceRef::new("skiff.run/account"));
        let hooks = TestBehaviorHooks::default();

        let bytes =
            encode_recoverable_payload_with_behavior(&value, &expected, &boundary, &heap, &hooks)
                .expect("local interface should encode through explicit owner-internal slot");
        assert_eq!(hooks.encode_calls.get(), 1);

        let mut decode_heap = RequestHeap::default();
        let decoded = decode_recoverable_payload_with_behavior(
            &bytes,
            &expected,
            &boundary,
            &mut decode_heap,
            &hooks,
        )
        .expect("local interface should decode through explicit owner-internal slot");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded interface should be a heap value");
        };
        let HeapNode::Interface(interface) = decode_heap.get(handle).expect("interface resolves")
        else {
            panic!("decoded value should be an interface");
        };
        let InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } = interface.carrier()
        else {
            panic!("decoded interface should use local carrier");
        };
        assert_eq!(interface.interface(), READER_INTERFACE);
        assert_eq!(concrete_type, READER_IMPL);
        assert_eq!(method_table.id(), READER_PROJECTION);
        assert_eq!(method_table.interface_abi_id(), READER_INTERFACE);
        assert_eq!(method_table.slots()[0].method_abi_id(), READER_METHOD);
        assert_eq!(payload, &RuntimeValue::String("Ada".to_string()));
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 1);
        assert_eq!(hooks.table_calls.get(), 1);
    }

    #[test]
    fn owner_internal_service_explicit_slot_roundtrips_remote_interface_with_hooks() {
        let expected = any_reader_expected();
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::InboundServiceCall)
            .with_origin_service(PayloadServiceRef::new("skiff.run/account"));
        let mut heap = RequestHeap::default();
        let value = remote_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let bytes =
            encode_recoverable_payload_with_behavior(&value, &expected, &boundary, &heap, &hooks)
                .expect("remote interface should encode through explicit owner-internal slot");
        assert_eq!(hooks.encode_calls.get(), 0);

        let mut decode_heap = RequestHeap::default();
        let decoded = decode_recoverable_payload_with_behavior(
            &bytes,
            &expected,
            &boundary,
            &mut decode_heap,
            &hooks,
        )
        .expect("remote interface should decode through explicit owner-internal slot");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded interface should be a heap value");
        };
        let HeapNode::Interface(interface) = decode_heap.get(handle).expect("interface resolves")
        else {
            panic!("decoded value should be an interface");
        };
        let InterfaceCarrier::Remote {
            dependency_ref,
            public_instance_key,
            operations,
        } = interface.carrier()
        else {
            panic!("decoded interface should use remote carrier");
        };
        assert_eq!(interface.interface(), READER_INTERFACE);
        assert_eq!(dependency_ref, "dep:reader");
        assert_eq!(public_instance_key, "reader#1");
        assert_eq!(operations.id(), "remote:reader");
        assert_eq!(operations.interface_abi_id(), READER_INTERFACE);
        assert_eq!(operations.slots()[0].slot(), 0);
        assert_eq!(operations.slots()[0].method_abi_id(), READER_METHOD);
        assert_eq!(
            operations.slots()[0].operation_abi_id(),
            "operation:reader.read"
        );
        assert_eq!(hooks.restore_calls.get(), 0);
        assert_eq!(hooks.remote_table_calls.get(), 2);
    }

    #[test]
    fn behavior_helper_encode_failures_return_no_bytes_before_submission() {
        let expected = any_reader_expected();
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let mut heap = RequestHeap::default();
        let local_value = local_interface_runtime_value(&mut heap);

        let missing_hook = FailClosedRecoverableBehaviorHooks;
        let error = encode_recoverable_payload_with_behavior(
            &local_value,
            &expected,
            &boundary,
            &heap,
            &missing_hook,
        )
        .expect_err("missing production hook must fail before bytes are returned");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CodeIdentityMissing
        );
    }

    #[test]
    fn runtime_wire_target_service_rejects_behavior_and_maps_context() {
        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
            .with_target_service(PayloadServiceRef::new("skiff.run/registry"));
        let context = recoverable_payload_context(&boundary);
        assert_eq!(
            context.kind,
            RuntimeRecoverableBoundaryKind::RuntimeWirePayload
        );
        assert_eq!(
            context.trust_boundary,
            RuntimeRecoverableTrustBoundary::CrossService
        );
        assert_eq!(
            context.storage_lane,
            RuntimeRecoverableStorageLane::RecoverableEnvelope
        );
        assert!(context.explicit_recoverable_slot);

        let hooks = TestBehaviorHooks::default();
        let bytes = canonical_envelope_bytes(record_node("value", interface_node()));
        let error = decode_recoverable_payload_with_behavior(
            &bytes,
            &recoverable_unresolved_expected(),
            &boundary,
            &mut RequestHeap::default(),
            &hooks,
        )
        .expect_err("runtime wire cross-service behavior must fail before hooks");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
        );
        assert_eq!(hooks.restore_calls.get(), 0);
    }

    #[test]
    fn non_owner_explicit_slots_reject_nested_behavior_before_hooks() {
        let boundaries = [
            PayloadBoundary::cross_service(
                PayloadBoundaryKind::OutboundServiceCall,
                PayloadServiceRef::new("skiff.run/registry"),
            ),
            PayloadBoundary::external_untrusted(PayloadBoundaryKind::PublicApiPayload),
            PayloadBoundary::external_untrusted(PayloadBoundaryKind::MaterializationPayload),
            PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
                .with_target_service(PayloadServiceRef::new("skiff.run/registry")),
        ];
        let behavior_nodes = [
            interface_node(),
            local_concrete_self_node("Ada"),
            native_adapter_plain_node(),
            native_handle_node(),
        ];

        for boundary in boundaries {
            for node in behavior_nodes.clone() {
                let hooks = TestBehaviorHooks::default();
                let bytes = canonical_envelope_bytes(record_node("value", node));
                let error = decode_recoverable_payload_with_behavior(
                    &bytes,
                    &recoverable_unresolved_expected(),
                    &boundary,
                    &mut RequestHeap::default(),
                    &hooks,
                )
                .expect_err("non-owner behavior envelope must fail closed before hooks");
                let RuntimeError::Recoverable(error) = error else {
                    panic!("expected recoverable error");
                };
                assert_eq!(
                    error.code(),
                    RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
                );
                assert_eq!(hooks.restore_calls.get(), 0);
                assert_eq!(hooks.encode_calls.get(), 0);
            }
        }
    }

    #[test]
    fn cross_service_local_carrier_encode_uses_callback_unavailable_error() {
        let boundary = PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            PayloadServiceRef::new("skiff.run/registry"),
        );
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = local_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let error =
            encode_recoverable_payload_with_behavior(&value, &expected, &boundary, &heap, &hooks)
                .expect_err("cross-service local carrier must fail before hooks");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CrossServiceInterfaceCallbackUnavailable
        );
        assert_eq!(hooks.encode_calls.get(), 0);
    }

    #[test]
    fn payload_codec_round_trips_record_with_raw_bytes_without_base64_metadata() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "name": { "kind": "builtin", "name": "string", "args": [] },
                "body": { "kind": "builtin", "name": "bytes", "args": [] }
            }
        });
        let mut heap = RequestHeap::default();
        let bytes = vec![0, 1, 2, 250, 255];
        let bytes_handle = heap
            .alloc_bytes(RuntimeBytes::from(bytes.clone()))
            .expect("bytes should allocate");
        let object_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("name".to_string(), RuntimeValue::String("Ada".to_string())),
                ("body".to_string(), RuntimeValue::Heap(bytes_handle)),
            ])))
            .expect("record should allocate");
        let encoded = encode_payload(&RuntimeValue::Heap(object_handle), &descriptor, &heap)
            .expect("payload should encode");

        assert!(!String::from_utf8_lossy(&encoded).contains("__skiffBytesBase64"));

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("payload should decode");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded payload should be heap object");
        };
        let HeapNode::Object(object) = decoded_heap.get(handle).expect("object should exist")
        else {
            panic!("decoded payload should be object");
        };
        assert_eq!(
            object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
        let RuntimeValue::Heap(body_handle) = object.fields().get("body").unwrap() else {
            panic!("body should be heap bytes");
        };
        let HeapNode::Bytes(decoded_bytes) =
            decoded_heap.get(*body_handle).expect("bytes should exist")
        else {
            panic!("body should decode as bytes");
        };
        assert_eq!(decoded_bytes.as_slice(), bytes.as_slice());
    }

    #[test]
    fn payload_codec_round_trips_date_as_epoch_milliseconds_tag() {
        let descriptor = json!({ "kind": "builtin", "name": "Date", "args": [] });
        let heap = RequestHeap::default();
        let encoded =
            encode_payload(&RuntimeValue::Date(0), &descriptor, &heap).expect("Date should encode");

        assert!(
            !String::from_utf8_lossy(&encoded).contains("1970-01-01"),
            "payload Date should not materialize as an ISO string"
        );

        let mut decoded_heap = RequestHeap::default();
        let decoded =
            decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("Date should decode");

        assert_eq!(decoded, RuntimeValue::Date(0));
    }

    #[test]
    fn payload_codec_round_trips_duration_as_integer_milliseconds_payload() {
        let descriptor = json!({
            "kind": "representation",
            "name": "std.time.Duration",
            "representation": { "kind": "builtin", "name": "integer", "args": [] }
        });
        let heap = RequestHeap::default();
        let encoded = encode_payload(&RuntimeValue::Number(2_000.0), &descriptor, &heap)
            .expect("Duration should encode as integer payload");

        assert!(
            !String::from_utf8_lossy(&encoded).contains("Duration"),
            "payload Duration should not carry a nominal type envelope"
        );

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("Duration should decode as integer payload");

        assert_eq!(decoded, RuntimeValue::Number(2_000.0));
    }

    #[test]
    fn payload_codec_nullable_union_branch_zero_does_not_decode_as_null() {
        let descriptor = json!({
            "kind": "nullable",
            "inner": {
                "kind": "union",
                "items": [
                    { "kind": "builtin", "name": "string", "args": [] },
                    { "kind": "builtin", "name": "number", "args": [] }
                ]
            }
        });
        let heap = RequestHeap::default();
        let encoded = encode_payload(
            &RuntimeValue::String("branch-zero".to_string()),
            &descriptor,
            &heap,
        )
        .expect("nullable union should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("nullable union should decode");

        assert_eq!(decoded, RuntimeValue::String("branch-zero".to_string()));
    }

    #[test]
    fn payload_codec_encodes_map_literal_as_static_record_payload() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "tag": {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "accept" }
                },
                "context": {
                    "kind": "record",
                    "fields": {
                        "userId": { "kind": "builtin", "name": "string", "args": [] }
                    }
                }
            }
        });
        let mut heap = RequestHeap::default();
        let mut context = RuntimeMap::new();
        context.insert(
            RuntimeValueKey::string("userId"),
            RuntimeValue::String("user-1".to_string()),
        );
        let context_handle = heap
            .alloc_map(context)
            .expect("context map should allocate");
        let mut record = RuntimeMap::new();
        record.insert(
            RuntimeValueKey::string("tag"),
            RuntimeValue::String("accept".to_string()),
        );
        record.insert(
            RuntimeValueKey::string("context"),
            RuntimeValue::Heap(context_handle),
        );
        let record_handle = heap.alloc_map(record).expect("record map should allocate");

        let encoded = encode_payload(&RuntimeValue::Heap(record_handle), &descriptor, &heap)
            .expect("map literal should encode as static record payload");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("static record payload should decode");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded record should be heap value");
        };
        let HeapNode::Object(object) = decoded_heap.get(handle).expect("record should exist")
        else {
            panic!("decoded record should be object");
        };
        assert_eq!(
            object.fields().get("tag"),
            Some(&RuntimeValue::String("accept".to_string()))
        );
    }

    #[test]
    fn payload_codec_encodes_map_literal_against_union_record_payload() {
        let descriptor = json!({
            "kind": "union",
            "items": [
                {
                    "kind": "record",
                    "fields": {
                        "tag": {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "accept" }
                        },
                        "identity": { "kind": "builtin", "name": "string", "args": [] }
                    }
                },
                {
                    "kind": "record",
                    "fields": {
                        "tag": {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "reject" }
                        },
                        "reason": { "kind": "builtin", "name": "string", "args": [] }
                    }
                }
            ]
        });
        let mut heap = RequestHeap::default();
        let mut record = RuntimeMap::new();
        record.insert(
            RuntimeValueKey::string("identity"),
            RuntimeValue::String("user-1".to_string()),
        );
        record.insert(
            RuntimeValueKey::string("tag"),
            RuntimeValue::String("accept".to_string()),
        );
        let record_handle = heap.alloc_map(record).expect("record map should allocate");

        let encoded = encode_payload(&RuntimeValue::Heap(record_handle), &descriptor, &heap)
            .expect("map literal should encode against union record payload");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("union record payload should decode");
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded union branch should be heap value");
        };
        let HeapNode::Object(object) = decoded_heap
            .get(handle)
            .expect("decoded branch should exist")
        else {
            panic!("decoded union branch should be object");
        };
        assert_eq!(
            object.fields().get("identity"),
            Some(&RuntimeValue::String("user-1".to_string()))
        );
    }

    #[test]
    fn payload_codec_round_trips_map_with_representation_keys() {
        let descriptor = json!({
            "kind": "builtin",
            "name": "Map",
            "args": [
                {
                    "kind": "representation",
                    "name": "UserId",
                    "representation": { "kind": "builtin", "name": "string", "args": [] }
                },
                { "kind": "builtin", "name": "number", "args": [] }
            ]
        });
        let mut heap = RequestHeap::default();
        let mut map = RuntimeMap::new();
        map.insert(
            RuntimeValueKey::string("user-1"),
            RuntimeValue::Number(42.0),
        );
        let map_handle = heap.alloc_map(map).expect("map should allocate");

        let encoded = encode_payload(&RuntimeValue::Heap(map_handle), &descriptor, &heap)
            .expect("map should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded =
            decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("map should decode");
        let RuntimeValue::Heap(decoded_handle) = decoded else {
            panic!("decoded map should be heap value");
        };
        let HeapNode::Map(decoded_map) = decoded_heap
            .get(decoded_handle)
            .expect("decoded map should exist")
        else {
            panic!("decoded payload should be map");
        };
        assert_eq!(
            decoded_map.get(&RuntimeValueKey::string("user-1")),
            Some(&RuntimeValue::Number(42.0))
        );
    }

    #[test]
    fn payload_codec_round_trips_map_with_named_representation_keys() {
        let descriptor = json!({
            "kind": "builtin",
            "name": "Map",
            "args": [
                { "kind": "builtin", "name": "UserId", "args": [] },
                { "kind": "builtin", "name": "number", "args": [] }
            ]
        });
        let mut heap = RequestHeap::default();
        let mut map = RuntimeMap::new();
        map.insert(
            RuntimeValueKey::string("user-1"),
            RuntimeValue::Number(42.0),
        );
        let map_handle = heap.alloc_map(map).expect("map should allocate");

        let encoded = encode_payload(&RuntimeValue::Heap(map_handle), &descriptor, &heap)
            .expect("map should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded =
            decode_payload(&encoded, &descriptor, &mut decoded_heap).expect("map should decode");
        let RuntimeValue::Heap(decoded_handle) = decoded else {
            panic!("decoded map should be heap value");
        };
        let HeapNode::Map(decoded_map) = decoded_heap
            .get(decoded_handle)
            .expect("decoded map should exist")
        else {
            panic!("decoded payload should be map");
        };
        assert_eq!(
            decoded_map.get(&RuntimeValueKey::string("user-1")),
            Some(&RuntimeValue::Number(42.0))
        );
    }

    #[test]
    fn json_and_binary_boundaries_share_erased_plan_behavior() {
        let duration_descriptor = json!({
            "kind": "representation",
            "name": "std.time.Duration",
            "representation": { "kind": "builtin", "name": "integer", "args": [] }
        });
        let duration_plan = RuntimeTypePlan::from_descriptor(&duration_descriptor)
            .expect("duration plan should build");
        let mut json_heap = RequestHeap::default();
        let duration =
            RuntimeBoundaryCodec::new(&duration_plan, BoundaryUse::TypedJson, "json duration")
                .from_wire_json(&json!(250), &mut json_heap)
                .expect("JSON boundary should erase Duration representation");
        assert_eq!(duration, RuntimeValue::Number(250.0));

        let encoded = encode_payload_plan(&duration, &duration_plan, &test_boundary(), &json_heap)
            .expect("binary boundary should encode erased Duration payload");
        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload_plan(
            &encoded,
            &duration_plan,
            &test_boundary(),
            &mut decoded_heap,
        )
        .expect("binary boundary should decode erased Duration payload");
        assert_eq!(decoded, RuntimeValue::Number(250.0));

        let date_plan =
            RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": "Date" }))
                .expect("Date plan should build");
        let mut date_json_heap = RequestHeap::default();
        let date = RuntimeBoundaryCodec::new(&date_plan, BoundaryUse::TypedJson, "json date")
            .from_wire_json(&json!("1970-01-01T00:00:00.000Z"), &mut date_json_heap)
            .expect("JSON boundary should decode RFC3339 Date");
        assert_eq!(date, RuntimeValue::Date(0));
        let date_encoded =
            encode_payload_plan(&date, &date_plan, &test_boundary(), &date_json_heap)
                .expect("binary boundary should encode Date as epoch millis");
        let mut date_decoded_heap = RequestHeap::default();
        assert_eq!(
            decode_payload_plan(
                &date_encoded,
                &date_plan,
                &test_boundary(),
                &mut date_decoded_heap
            )
            .expect("binary boundary should decode Date as epoch millis"),
            RuntimeValue::Date(0)
        );
    }

    #[test]
    fn json_and_binary_boundaries_share_representation_map_key_behavior() {
        let descriptor = json!({
            "kind": "builtin",
            "name": "Map",
            "args": [
                {
                    "kind": "representation",
                    "name": "UserId",
                    "representation": { "kind": "builtin", "name": "string", "args": [] }
                },
                { "kind": "builtin", "name": "integer", "args": [] }
            ]
        });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("map plan should build");
        let mut json_heap = RequestHeap::default();
        let value = RuntimeBoundaryCodec::new(&plan, BoundaryUse::TypedJson, "json map")
            .from_wire_json(&json!({ "user-1": 7 }), &mut json_heap)
            .expect("JSON boundary should erase representation map keys");

        let encoded = encode_payload_plan(&value, &plan, &test_boundary(), &json_heap)
            .expect("binary boundary should encode representation map key as string");
        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload_plan(&encoded, &plan, &test_boundary(), &mut decoded_heap)
            .expect("binary boundary should decode representation map key as string");
        let RuntimeValue::Heap(decoded_handle) = decoded else {
            panic!("decoded map should be heap value");
        };
        let HeapNode::Map(decoded_map) = decoded_heap
            .get(decoded_handle)
            .expect("decoded map should exist")
        else {
            panic!("decoded payload should be map");
        };
        assert_eq!(
            decoded_map.get(&RuntimeValueKey::string("user-1")),
            Some(&RuntimeValue::Number(7.0))
        );
    }

    #[test]
    fn json_and_binary_boundaries_reject_legacy_skiff_type_metadata() {
        let descriptor = json!({
            "kind": "builtin",
            "name": "Map",
            "args": [
                { "kind": "builtin", "name": "string", "args": [] },
                { "kind": "builtin", "name": "string", "args": [] }
            ]
        });
        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("map plan should build");
        let mut json_heap = RequestHeap::default();
        let json_error = RuntimeBoundaryCodec::new(&plan, BoundaryUse::TypedJson, "json map")
            .from_wire_json(&json!({ "__skiffType": "Legacy" }), &mut json_heap)
            .expect_err("JSON boundary should reject reserved legacy metadata");
        assert!(json_error
            .to_string()
            .contains("reserved Skiff metadata field __skiffType"));

        let mut heap = RequestHeap::default();
        let mut map = RuntimeMap::new();
        map.insert(
            RuntimeValueKey::string("__skiffType"),
            RuntimeValue::String("Legacy".to_string()),
        );
        let handle = heap.alloc_map(map).expect("map should allocate");
        let binary_error =
            encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &test_boundary(), &heap)
                .expect_err("binary boundary should reject reserved legacy metadata");
        assert!(binary_error
            .to_string()
            .contains("reserved Skiff metadata field __skiffType"));
    }

    #[test]
    fn runtime_payload_codec_rejects_interface_wrapper() {
        let descriptor = json!({ "kind": "builtin", "name": "Json", "args": [] });
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_interface(InterfaceValue::new(
                "pkg.Reader".to_string(),
                InterfaceCarrier::Local {
                    concrete_type: "pkg.FileReader".to_string(),
                    method_table: InterfaceMethodTable::new(
                        "table:pkg.Reader:pkg.FileReader".to_string(),
                        "pkg.Reader".to_string(),
                        Vec::new(),
                    ),
                    payload: RuntimeValue::Null,
                },
            ))
            .expect("interface should allocate");

        let error = encode_payload(&RuntimeValue::Heap(handle), &descriptor, &heap)
            .expect_err("runtime binary payload should reject interface wrapper");

        assert_interface_recoverable_envelope_error(
            error,
            RecoverableBoundaryErrorCode::UnsupportedEncode,
        );
    }

    #[test]
    fn runtime_payload_codec_fails_closed_typed_any_interface_local_carrier() {
        let plan = any_interface_plan();
        let mut heap = RequestHeap::default();
        let payload = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "name".to_string(),
                RuntimeValue::String("Ada".to_string()),
            )])))
            .expect("payload object should allocate");
        let handle = heap
            .alloc_interface(InterfaceValue::new(
                "pkg.Reader".to_string(),
                InterfaceCarrier::Local {
                    concrete_type: "pkg.FileReader".to_string(),
                    method_table: InterfaceMethodTable::new(
                        "table:pkg.Reader:pkg.FileReader".to_string(),
                        "pkg.Reader".to_string(),
                        vec![InterfaceMethodSlot::new(
                            0,
                            "method:pkg.Reader.read".to_string(),
                            InterfaceMethodTarget::LocalExecutable {
                                executable: ExecutableAddr::service(2, 7),
                                receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                            },
                        )],
                    ),
                    payload: RuntimeValue::Heap(payload),
                },
            ))
            .expect("interface should allocate");

        let error =
            encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &test_boundary(), &heap)
                .expect_err("typed any interface payload must fail closed until recover P4");

        assert_interface_recoverable_envelope_error(
            error,
            RecoverableBoundaryErrorCode::UnsupportedEncode,
        );
    }

    #[test]
    fn runtime_payload_codec_fails_closed_typed_any_interface_remote_carrier() {
        let plan = any_interface_plan();
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_interface(InterfaceValue::new(
                "pkg.Reader".to_string(),
                InterfaceCarrier::Remote {
                    dependency_ref: "dep:pkg.reader".to_string(),
                    public_instance_key: "public:reader".to_string(),
                    operations: RemoteOperationTable::new(
                        "remote:pkg.Reader".to_string(),
                        "pkg.Reader".to_string(),
                        vec![RemoteOperationSlot::new(
                            0,
                            "method:pkg.Reader.read".to_string(),
                            "operation:pkg.Reader.read".to_string(),
                        )],
                    ),
                },
            ))
            .expect("interface should allocate");

        let boundary = PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload);
        let error = encode_payload_plan(&RuntimeValue::Heap(handle), &plan, &boundary, &heap)
            .expect_err("remote any interface payload must fail closed until recover P4");

        assert_interface_recoverable_envelope_error(
            error,
            RecoverableBoundaryErrorCode::UnsupportedEncode,
        );
    }

    #[test]
    fn runtime_payload_codec_rejects_reserved_interface_tag_without_reconstructing_interface() {
        let plan = RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": "Json" }))
            .expect("Json plan should build");
        let mut bytes = Vec::from(MAGIC.as_slice());
        bytes.push(VERSION);
        bytes.push(TAG_INTERFACE);

        let mut decoded_heap = RequestHeap::default();
        let error = decode_payload_plan(&bytes, &plan, &test_boundary(), &mut decoded_heap)
            .expect_err("reserved interface tag must not reconstruct InterfaceValue");

        assert_eq!(decoded_heap.len(), 0);
        assert_interface_recoverable_envelope_error(
            error,
            RecoverableBoundaryErrorCode::UnsupportedDecode,
        );
    }

    #[test]
    fn payload_codec_encodes_record_payload_for_representation_descriptor() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "name": { "kind": "builtin", "name": "string", "args": [] }
            }
        });
        let mut heap = RequestHeap::default();
        let object_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "name".to_string(),
                RuntimeValue::String("Ada".to_string()),
            )])))
            .expect("record should allocate");
        let encoded = encode_payload(&RuntimeValue::Heap(object_handle), &descriptor, &heap)
            .expect("erased representation payload record should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("erased representation payload record should decode");
        let RuntimeValue::Heap(decoded_handle) = decoded else {
            panic!("decoded record should be heap value");
        };
        let HeapNode::Object(decoded_object) = decoded_heap
            .get(decoded_handle)
            .expect("decoded record should exist")
        else {
            panic!("decoded payload should be object");
        };
        assert_eq!(
            decoded_object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
    }

    #[test]
    fn payload_codec_decodes_representation_descriptor_to_payload_value() {
        let descriptor = json!({
            "kind": "representation",
            "name": "Name",
            "representation": { "kind": "builtin", "name": "string", "args": [] }
        });
        let heap = RequestHeap::default();

        let encoded = encode_payload(&RuntimeValue::String("Ada".to_string()), &descriptor, &heap)
            .expect("erased representation payload should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("erased representation payload should decode");
        assert_eq!(decoded, RuntimeValue::String("Ada".to_string()));
    }

    #[test]
    fn payload_codec_representation_descriptor_does_not_preserve_nominal_identity() {
        let descriptor = json!({
            "kind": "representation",
            "name": "UserId",
            "representation": { "kind": "builtin", "name": "string", "args": [] }
        });
        let heap = RequestHeap::default();

        let encoded = encode_payload(
            &RuntimeValue::String("tenant-1".to_string()),
            &descriptor,
            &heap,
        )
        .expect("erased representation payload should encode");

        let mut decoded_heap = RequestHeap::default();
        let decoded = decode_payload(&encoded, &descriptor, &mut decoded_heap)
            .expect("erased representation payload should decode");
        assert_eq!(decoded, RuntimeValue::String("tenant-1".to_string()));
    }

    #[test]
    fn payload_codec_rejects_union_with_more_than_256_branches() {
        let descriptor = json!({
            "kind": "union",
            "items": (0..257)
                .map(|_| json!({ "kind": "builtin", "name": "string", "args": [] }))
                .collect::<Vec<_>>()
        });
        let heap = RequestHeap::default();

        let error = encode_payload(
            &RuntimeValue::String("too-many-branches".to_string()),
            &descriptor,
            &heap,
        )
        .expect_err("union with more than 256 branches should fail closed");

        assert!(error.to_string().contains("maximum is 256"));
    }
}
