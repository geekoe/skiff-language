use std::collections::{BTreeMap, BTreeSet, HashSet};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use skiff_runtime_model::{
    recoverable::{
        InterfaceValueState, NativeHandleState, NominalObjectState, RecoverableArtifactRef,
        RecoverableArtifactRetentionRoot, RecoverableCodeIdentity, RecoverableDate,
        RecoverableEnvelope, RecoverableField, RecoverableMapKey, RecoverableNode,
        RecoverableNumber, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
        RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableExpectedAnyInterfacePlan, RuntimeRecoverableExpectedTypeNode,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{
        HeapHandle, HeapNode, InterfaceCarrier, InterfaceMethodTable, InterfaceValue, RuntimeMap,
        RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
};

use crate::error::{RecoverableBoundaryError, RecoverableBoundaryErrorCode, Result, RuntimeError};

pub trait RecoverableArtifactStore {
    fn can_load_artifact(&self, artifact_identity: &str, build_id: &str) -> bool;
}

pub trait RecoverableArtifactRetentionRootStore {
    fn persist_roots(
        &mut self,
        roots: &[RecoverableArtifactRetentionRoot],
    ) -> std::result::Result<(), String>;
}

pub struct RecoverableLocalInterfaceEncodeRequest<'a> {
    pub interface_identity: &'a str,
    pub concrete_type: &'a str,
    pub method_table: &'a InterfaceMethodTable,
    pub payload: &'a RuntimeValue,
    pub path: &'a str,
    pub context: &'a RuntimeRecoverableBoundaryContext,
    pub expected: &'a RuntimeRecoverableExpectedTypePlan,
}

pub struct RecoverableEncodedLocalInterfaceSelf {
    pub method_projection_identity: String,
    pub self_node: RecoverableNode,
}

pub struct RecoverableLocalInterfaceRestoreRequest<'a> {
    pub interface_identity: &'a str,
    pub method_projection_identity: &'a str,
    pub expected_any_interface: &'a RuntimeRecoverableExpectedAnyInterfacePlan,
    pub self_node: &'a RecoverableNode,
    pub path: &'a str,
    pub context: &'a RuntimeRecoverableBoundaryContext,
    pub expected: &'a RuntimeRecoverableExpectedTypePlan,
    pub decode_policy: RecoverableDecodePolicy,
}

pub struct RecoverableRestoredLocalInterfaceSelf {
    pub concrete_type_identity: String,
    pub runtime_concrete_type_identity: String,
    pub payload: RuntimeValue,
}

pub struct RecoverableInterfaceConformanceRequest<'a> {
    pub concrete_type_identity: &'a str,
    pub interface_identity: &'a str,
    pub method_projection_identity: &'a str,
    pub expected_any_interface: &'a RuntimeRecoverableExpectedAnyInterfacePlan,
    pub path: &'a str,
    pub context: &'a RuntimeRecoverableBoundaryContext,
    pub expected: &'a RuntimeRecoverableExpectedTypePlan,
}

pub struct RecoverableInterfaceMethodTableRequest<'a> {
    pub concrete_type_identity: &'a str,
    pub interface_identity: &'a str,
    pub method_projection_identity: &'a str,
    pub expected_any_interface: &'a RuntimeRecoverableExpectedAnyInterfacePlan,
    pub path: &'a str,
    pub context: &'a RuntimeRecoverableBoundaryContext,
    pub expected: &'a RuntimeRecoverableExpectedTypePlan,
}

pub trait RecoverableBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        heap: &RequestHeap,
    ) -> Result<Option<RecoverableEncodedLocalInterfaceSelf>>;

    fn restore_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceRestoreRequest<'_>,
        heap: &mut RequestHeap,
    ) -> Result<Option<RecoverableRestoredLocalInterfaceSelf>>;

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> Result<bool>;

    fn rebuild_local_interface_method_table(
        &self,
        request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> Result<Option<InterfaceMethodTable>>;
}

pub struct FailClosedRecoverableBehaviorHooks;

impl RecoverableBehaviorHooks for FailClosedRecoverableBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        _request: RecoverableLocalInterfaceEncodeRequest<'_>,
        _heap: &RequestHeap,
    ) -> Result<Option<RecoverableEncodedLocalInterfaceSelf>> {
        Ok(None)
    }

    fn restore_local_interface_self(
        &self,
        _request: RecoverableLocalInterfaceRestoreRequest<'_>,
        _heap: &mut RequestHeap,
    ) -> Result<Option<RecoverableRestoredLocalInterfaceSelf>> {
        Ok(None)
    }

    fn concrete_type_conforms_to_interface(
        &self,
        _request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> Result<bool> {
        Ok(false)
    }

    fn rebuild_local_interface_method_table(
        &self,
        _request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> Result<Option<InterfaceMethodTable>> {
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoverableDecodePolicy {
    ignore_unknown_record_fields: bool,
    materialize_missing_nullable_record_fields: bool,
}

impl RecoverableDecodePolicy {
    pub const fn strict() -> Self {
        Self {
            ignore_unknown_record_fields: false,
            materialize_missing_nullable_record_fields: false,
        }
    }

    pub const fn durable_db() -> Self {
        Self {
            ignore_unknown_record_fields: true,
            materialize_missing_nullable_record_fields: true,
        }
    }

    pub const fn ignores_unknown_record_fields(self) -> bool {
        self.ignore_unknown_record_fields
    }

    pub const fn materializes_missing_nullable_record_fields(self) -> bool {
        self.materialize_missing_nullable_record_fields
    }
}

impl Default for RecoverableDecodePolicy {
    fn default() -> Self {
        Self::strict()
    }
}

pub struct RecoverableBoundaryCodec;

impl RecoverableBoundaryCodec {
    pub fn encode_envelope_canonical(
        envelope: &RecoverableEnvelope,
        limits: &RecoverableValidationLimits,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
    ) -> Result<Vec<u8>> {
        envelope.to_canonical_bytes(limits).map_err(|error| {
            RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::StateInvalid,
                error.to_string(),
                context,
                expected,
            )
            .with_detail(serde_json::json!({
                "nodePath": error.path(),
                "reason": error.message(),
            }))
            .into()
        })
    }

    pub fn decode_envelope_canonical(
        bytes: &[u8],
        limits: &RecoverableValidationLimits,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
    ) -> Result<RecoverableEnvelope> {
        RecoverableEnvelope::from_canonical_bytes(bytes, limits).map_err(|error| {
            RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::StateInvalid,
                error.to_string(),
                context,
                expected,
            )
            .with_detail(serde_json::json!({
                "nodePath": error.path(),
                "reason": error.message(),
            }))
            .into()
        })
    }

    pub fn verify_artifact_availability(
        envelope: &RecoverableEnvelope,
        store: &dyn RecoverableArtifactStore,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
    ) -> Result<Vec<RecoverableArtifactRef>> {
        let refs = envelope.collect_artifact_refs();
        for artifact_ref in &refs {
            if !store.can_load_artifact(&artifact_ref.artifact_identity, &artifact_ref.build_id) {
                return Err(artifact_unavailable_error(
                    artifact_ref,
                    "artifact is not loadable by build id",
                    context,
                    expected,
                )
                .into());
            }
        }
        Ok(refs)
    }

    pub fn persist_artifact_retention_roots(
        refs: &[RecoverableArtifactRef],
        store: &mut dyn RecoverableArtifactRetentionRootStore,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        expires_at_epoch_millis: Option<i64>,
    ) -> Result<Vec<RecoverableArtifactRetentionRoot>> {
        let roots = retention_roots_for_refs(refs, context, expires_at_epoch_millis);
        if let Err(message) = store.persist_roots(&roots) {
            let detail = serde_json::json!({
                "serviceId": recoverable_service_id(context),
                "boundaryKind": context.kind,
                "reason": message,
                "rootCount": roots.len(),
            });
            return Err(RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::ArtifactUnavailable,
                "recoverable artifact retention root write failed",
                context,
                expected,
            )
            .with_detail(detail)
            .into());
        }
        Ok(roots)
    }

    pub fn encode(
        value: &RuntimeValue,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &RequestHeap,
    ) -> Result<Vec<u8>> {
        let mut encoder = RecoverableValueEncoder {
            heap,
            context,
            expected,
            seen_handles: HashSet::new(),
            behavior_hooks: None,
        };
        let root = encoder.encode_value(value, "$.root")?;
        let envelope = RecoverableEnvelope::new(root);
        precheck_expected_type(&envelope.root, expected, "$.root")
            .map_err(|error| expected_type_mismatch_error(error, "encode", context, expected))?;
        Self::encode_envelope_canonical(
            &envelope,
            &RecoverableValidationLimits::default(),
            expected,
            context,
        )
    }

    pub fn encode_envelope_with_behavior(
        value: &RuntimeValue,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &RequestHeap,
        behavior_hooks: &dyn RecoverableBehaviorHooks,
    ) -> Result<RecoverableEnvelope> {
        let mut encoder = RecoverableValueEncoder {
            heap,
            context,
            expected,
            seen_handles: HashSet::new(),
            behavior_hooks: Some(behavior_hooks),
        };
        let root = encoder.encode_value(value, "$.root")?;
        let envelope = RecoverableEnvelope::new(root);
        select_expected_plan_for_node_with_behavior_policy(
            &envelope.root,
            expected,
            "$.root",
            context,
            expected,
            behavior_hooks,
            RecoverableDecodePolicy::strict(),
            "encode",
        )?;
        Ok(envelope)
    }

    pub fn encode_with_behavior(
        value: &RuntimeValue,
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &RequestHeap,
        behavior_hooks: &dyn RecoverableBehaviorHooks,
    ) -> Result<Vec<u8>> {
        let envelope =
            Self::encode_envelope_with_behavior(value, expected, context, heap, behavior_hooks)?;
        Self::encode_envelope_canonical(
            &envelope,
            &RecoverableValidationLimits::default(),
            expected,
            context,
        )
    }

    pub fn decode(
        bytes: &[u8],
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        Self::decode_with_policy(
            bytes,
            expected,
            context,
            heap,
            RecoverableDecodePolicy::strict(),
        )
    }

    pub fn decode_with_policy(
        bytes: &[u8],
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &mut RequestHeap,
        decode_policy: RecoverableDecodePolicy,
    ) -> Result<RuntimeValue> {
        let envelope = Self::decode_envelope_canonical(
            bytes,
            &RecoverableValidationLimits::default(),
            expected,
            context,
        )?;
        reject_untrusted_behavior_payload(&envelope, context, expected)?;
        precheck_expected_type_with_policy(&envelope.root, expected, "$.root", decode_policy)
            .map_err(|error| expected_type_mismatch_error(error, "decode", context, expected))?;

        let checkpoint = heap.checkpoint();
        match decode_node(
            &envelope.root,
            expected,
            "$.root",
            context,
            expected,
            heap,
            decode_policy,
        ) {
            Ok(value) => Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub fn decode_with_behavior(
        bytes: &[u8],
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &mut RequestHeap,
        behavior_hooks: &dyn RecoverableBehaviorHooks,
    ) -> Result<RuntimeValue> {
        Self::decode_with_behavior_and_policy(
            bytes,
            expected,
            context,
            heap,
            behavior_hooks,
            RecoverableDecodePolicy::strict(),
        )
    }

    pub fn decode_with_behavior_and_policy(
        bytes: &[u8],
        expected: &RuntimeRecoverableExpectedTypePlan,
        context: &RuntimeRecoverableBoundaryContext,
        heap: &mut RequestHeap,
        behavior_hooks: &dyn RecoverableBehaviorHooks,
        decode_policy: RecoverableDecodePolicy,
    ) -> Result<RuntimeValue> {
        let envelope = Self::decode_envelope_canonical(
            bytes,
            &RecoverableValidationLimits::default(),
            expected,
            context,
        )?;
        reject_untrusted_behavior_payload(&envelope, context, expected)?;
        select_expected_plan_for_node_with_behavior_policy(
            &envelope.root,
            expected,
            "$.root",
            context,
            expected,
            behavior_hooks,
            decode_policy,
            "decode",
        )?;

        let checkpoint = heap.checkpoint();
        match decode_node_with_behavior(
            &envelope.root,
            expected,
            "$.root",
            context,
            expected,
            heap,
            behavior_hooks,
            decode_policy,
        ) {
            Ok(value) => Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                Err(error)
            }
        }
    }
}

struct RecoverableValueEncoder<'a> {
    heap: &'a RequestHeap,
    context: &'a RuntimeRecoverableBoundaryContext,
    expected: &'a RuntimeRecoverableExpectedTypePlan,
    seen_handles: HashSet<HeapHandle>,
    behavior_hooks: Option<&'a dyn RecoverableBehaviorHooks>,
}

impl RecoverableValueEncoder<'_> {
    fn encode_value(&mut self, value: &RuntimeValue, path: &str) -> Result<RecoverableNode> {
        match value {
            RuntimeValue::Null => Ok(plain_node(
                RecoverableValueKind::Null,
                RecoverableState::Null,
            )),
            RuntimeValue::Bool(value) => Ok(plain_node(
                RecoverableValueKind::Bool,
                RecoverableState::Bool(*value),
            )),
            RuntimeValue::Number(value) => Ok(plain_node(
                RecoverableValueKind::Number,
                RecoverableState::Number(
                    RecoverableNumber::try_from_f64(*value)
                        .map_err(|error| state_invalid_error(error, self.context, self.expected))?,
                ),
            )),
            RuntimeValue::Date(epoch_millis) => Ok(plain_node(
                RecoverableValueKind::Date,
                RecoverableState::Date(
                    RecoverableDate::new(*epoch_millis)
                        .map_err(|error| state_invalid_error(error, self.context, self.expected))?,
                ),
            )),
            RuntimeValue::String(value) => Ok(plain_node(
                RecoverableValueKind::String,
                RecoverableState::String(value.clone()),
            )),
            RuntimeValue::ActorRef(actor_ref) => Err(unsupported_encode_error(
                format!(
                    "actor ref {} is request-local and has no recoverable envelope codec",
                    actor_ref.actor_type_identity()
                ),
                path,
                self.context,
                self.expected,
            )),
            RuntimeValue::Heap(handle) => self.encode_heap_node(*handle, path),
        }
    }

    fn encode_heap_node(&mut self, handle: HeapHandle, path: &str) -> Result<RecoverableNode> {
        if !self.seen_handles.insert(handle) {
            return Err(RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::StateInvalid,
                format!(
                    "recoverable encode does not preserve shared heap identity; heap handle {handle} is referenced more than once"
                ),
                self.context,
                self.expected,
            )
            .with_detail(serde_json::json!({
                "nodePath": path,
                "reason": "shared or cyclic heap handle is not supported by recoverable envelope v1",
            }))
            .into());
        }

        match self.heap.get(handle)? {
            HeapNode::Bytes(bytes) => Ok(plain_node(
                RecoverableValueKind::Bytes,
                RecoverableState::Bytes(bytes.as_slice().to_vec()),
            )),
            HeapNode::Array(items) => {
                let mut encoded = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    encoded.push(self.encode_value(item, &format!("{path}[{index}]"))?);
                }
                Ok(plain_node(
                    RecoverableValueKind::Array,
                    RecoverableState::Array(encoded),
                ))
            }
            HeapNode::Object(object) => {
                let fields = object
                    .fields()
                    .iter()
                    .map(|(field_identity, value)| {
                        Ok(RecoverableField {
                            field_identity: field_identity.clone(),
                            value: self
                                .encode_value(value, &format!("{path}.field({field_identity})"))?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(plain_node(
                    RecoverableValueKind::Record,
                    RecoverableState::Record(fields),
                ))
            }
            HeapNode::Map(map) => {
                let entries = map
                    .iter()
                    .map(|(key, value)| {
                        let key = recoverable_map_key_from_runtime_key(key);
                        let key_label = key_label(&key).to_string();
                        Ok((
                            key,
                            self.encode_value(value, &format!("{path}.map({key_label})"))?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(plain_node(
                    RecoverableValueKind::Map,
                    RecoverableState::Map(entries),
                ))
            }
            HeapNode::Interface(value) => match self.behavior_hooks {
                Some(behavior_hooks) => self.encode_interface_value(value, path, behavior_hooks),
                None => Err(interface_encode_error(
                    value,
                    path,
                    self.context,
                    self.expected,
                )),
            },
            HeapNode::Exception(_) => Err(RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::StateInvalid,
                "request-local exception cannot enter a recoverable boundary",
                self.context,
                self.expected,
            )
            .with_detail(serde_json::json!({ "nodePath": path }))
            .into()),
        }
    }

    fn encode_interface_value(
        &self,
        value: &InterfaceValue,
        path: &str,
        behavior_hooks: &dyn RecoverableBehaviorHooks,
    ) -> Result<RecoverableNode> {
        if self.context.trust_boundary != RuntimeRecoverableTrustBoundary::OwnerInternal {
            return Err(interface_encode_error(
                value,
                path,
                self.context,
                self.expected,
            ));
        }
        match value.carrier() {
            InterfaceCarrier::CallbackCapability(carrier) => Err(
                crate::persistent::callback_capability_not_recoverable_error(
                    carrier,
                    path,
                    self.context,
                    self.expected,
                ),
            ),
            InterfaceCarrier::Local {
                concrete_type,
                method_table,
                payload,
            } => {
                let encoded = behavior_hooks
                    .encode_local_interface_self(
                        RecoverableLocalInterfaceEncodeRequest {
                            interface_identity: value.interface(),
                            concrete_type,
                            method_table,
                            payload,
                            path,
                            context: self.context,
                            expected: self.expected,
                        },
                        self.heap,
                    )?
                    .ok_or_else(|| {
                        code_identity_missing_error(
                            "local InterfaceValue encode requires a registered behavior hook that supplies a LocalConcrete self node",
                            path,
                            self.context,
                            self.expected,
                        )
                    })?;
                validate_local_interface_self_node(
                    &encoded.self_node,
                    &format!("{path}.selfNode"),
                    self.context,
                    self.expected,
                )?;
                if encoded.method_projection_identity.is_empty() {
                    return Err(code_identity_missing_error(
                        "local InterfaceValue encode hook returned an empty method projection identity",
                        path,
                        self.context,
                        self.expected,
                    ));
                }
                Ok(RecoverableNode {
                    value_kind: RecoverableValueKind::InterfaceValue,
                    variant_identity: RecoverableVariantIdentity::None,
                    code_identity: RecoverableCodeIdentity::None,
                    state: RecoverableState::InterfaceValue(InterfaceValueState::Local {
                        self_node: Box::new(encoded.self_node),
                    }),
                })
            }
        }
    }
}

fn decode_node(
    node: &RecoverableNode,
    expected_for_node: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    heap: &mut RequestHeap,
    decode_policy: RecoverableDecodePolicy,
) -> Result<RuntimeValue> {
    let selected_expected =
        select_expected_plan_for_node_with_policy(node, expected_for_node, path, decode_policy)
            .map_err(|error| {
                expected_type_mismatch_error(error, "decode", context, root_expected)
            })?;
    reject_behavior_node_for_plain_decode(node, path, context, root_expected)?;
    match &node.state {
        RecoverableState::Null => Ok(RuntimeValue::Null),
        RecoverableState::Bool(value) => Ok(RuntimeValue::Bool(*value)),
        RecoverableState::Number(value) => Ok(RuntimeValue::Number(value.to_f64())),
        RecoverableState::String(value) => Ok(RuntimeValue::String(value.clone())),
        RecoverableState::Bytes(value) => Ok(RuntimeValue::Heap(heap.alloc_bytes(value.clone())?)),
        RecoverableState::Date(value) => Ok(RuntimeValue::Date(value.epoch_millis)),
        RecoverableState::Array(items) => {
            let child_expected = expected_array_item_plan(selected_expected);
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                decoded.push(decode_node(
                    item,
                    child_expected.unwrap_or(fallback_expected),
                    &format!("{path}[{index}]"),
                    context,
                    root_expected,
                    heap,
                    decode_policy,
                )?);
            }
            Ok(RuntimeValue::Heap(heap.alloc_array(decoded)?))
        }
        RecoverableState::Map(entries) => {
            let child_expected = expected_map_value_plan(selected_expected);
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = RuntimeMap::new();
            for (index, (key, value)) in entries.iter().enumerate() {
                let key =
                    runtime_key_from_recoverable_map_key(key, &format!("{path}.mapKey[{index}]"), context, root_expected)?;
                let value = decode_node(
                    value,
                    child_expected.unwrap_or(fallback_expected),
                    &format!("{path}.map[{index}]"),
                    context,
                    root_expected,
                    heap,
                    decode_policy,
                )?;
                decoded.insert(key, value);
            }
            Ok(RuntimeValue::Heap(heap.alloc_map(decoded)?))
        }
        RecoverableState::Record(fields) => {
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = RuntimeObjectFields::new();
            for field in fields {
                let field_expected =
                    expected_record_field_plan(selected_expected, &field.field_identity);
                if field_expected.is_none()
                    && decode_policy.ignores_unknown_record_fields()
                    && expected_record_fields(selected_expected).is_some()
                {
                    continue;
                }
                decoded.insert(
                    field.field_identity.clone(),
                    decode_node(
                        &field.value,
                        field_expected.unwrap_or(fallback_expected),
                        &format!("{path}.field({})", field.field_identity),
                        context,
                        root_expected,
                        heap,
                        decode_policy,
                    )?,
                );
            }
            materialize_missing_nullable_record_fields(&mut decoded, selected_expected, decode_policy);
            Ok(RuntimeValue::Heap(
                heap.alloc_object(RuntimeObject::unshaped(decoded))?,
            ))
        }
        RecoverableState::NominalObject(_) => Err(unsupported_decode_error(
            "nominal object restore requires an explicit concrete restore plan, which is not available in the current runtime architecture",
            path,
            context,
            root_expected,
        )),
        RecoverableState::InterfaceValue(_) => Err(unsupported_decode_error(
            "InterfaceValue recoverable wrapper recovery is reserved for P4 and is not decoded by the P3 plain codec",
            path,
            context,
            root_expected,
        )),
        RecoverableState::NativeHandle(_) => Err(RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::NativeMissingAdapter,
            "native handle restore requires an explicit native adapter hook, which is not available in the current runtime architecture",
            context,
            root_expected,
        )
        .with_detail(serde_json::json!({
            "nodePath": path,
            "reason": "native adapter decode hook is not registered",
        }))
        .into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_node_with_behavior(
    node: &RecoverableNode,
    expected_for_node: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
    decode_policy: RecoverableDecodePolicy,
) -> Result<RuntimeValue> {
    let selected_expected = select_expected_plan_for_node_with_behavior_policy(
        node,
        expected_for_node,
        path,
        context,
        root_expected,
        behavior_hooks,
        decode_policy,
        "decode",
    )?;
    if !matches!(node.state, RecoverableState::InterfaceValue(_)) {
        reject_behavior_node_for_plain_decode(node, path, context, root_expected)?;
    }
    match &node.state {
        RecoverableState::Null => Ok(RuntimeValue::Null),
        RecoverableState::Bool(value) => Ok(RuntimeValue::Bool(*value)),
        RecoverableState::Number(value) => Ok(RuntimeValue::Number(value.to_f64())),
        RecoverableState::String(value) => Ok(RuntimeValue::String(value.clone())),
        RecoverableState::Bytes(value) => Ok(RuntimeValue::Heap(heap.alloc_bytes(value.clone())?)),
        RecoverableState::Date(value) => Ok(RuntimeValue::Date(value.epoch_millis)),
        RecoverableState::Array(items) => {
            let child_expected = expected_array_item_plan(selected_expected);
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                decoded.push(decode_node_with_behavior(
                    item,
                    child_expected.unwrap_or(fallback_expected),
                    &format!("{path}[{index}]"),
                    context,
                    root_expected,
                    heap,
                    behavior_hooks,
                    decode_policy,
                )?);
            }
            Ok(RuntimeValue::Heap(heap.alloc_array(decoded)?))
        }
        RecoverableState::Map(entries) => {
            let child_expected = expected_map_value_plan(selected_expected);
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = RuntimeMap::new();
            for (index, (key, value)) in entries.iter().enumerate() {
                let key = runtime_key_from_recoverable_map_key(
                    key,
                    &format!("{path}.mapKey[{index}]"),
                    context,
                    root_expected,
                )?;
                let value = decode_node_with_behavior(
                    value,
                    child_expected.unwrap_or(fallback_expected),
                    &format!("{path}.map[{index}]"),
                    context,
                    root_expected,
                    heap,
                    behavior_hooks,
                    decode_policy,
                )?;
                decoded.insert(key, value);
            }
            Ok(RuntimeValue::Heap(heap.alloc_map(decoded)?))
        }
        RecoverableState::Record(fields) => {
            let json_child_expected = json_value_child_expected_plan();
            let fallback_expected = recoverable_child_fallback_expected(
                selected_expected,
                &json_child_expected,
            );
            let mut decoded = RuntimeObjectFields::new();
            for field in fields {
                let field_expected =
                    expected_record_field_plan(selected_expected, &field.field_identity);
                if field_expected.is_none()
                    && decode_policy.ignores_unknown_record_fields()
                    && expected_record_fields(selected_expected).is_some()
                {
                    continue;
                }
                decoded.insert(
                    field.field_identity.clone(),
                    decode_node_with_behavior(
                        &field.value,
                        field_expected.unwrap_or(fallback_expected),
                        &format!("{path}.field({})", field.field_identity),
                        context,
                        root_expected,
                        heap,
                        behavior_hooks,
                        decode_policy,
                    )?,
                );
            }
            materialize_missing_nullable_record_fields(&mut decoded, selected_expected, decode_policy);
            Ok(RuntimeValue::Heap(
                heap.alloc_object(RuntimeObject::unshaped(decoded))?,
            ))
        }
        RecoverableState::InterfaceValue(state) => decode_interface_node_with_behavior(
            state,
            selected_expected,
            path,
            context,
            root_expected,
            heap,
            behavior_hooks,
            decode_policy,
        ),
        RecoverableState::NominalObject(_) => Err(unsupported_decode_error(
            "nominal object restore outside an any-I self node requires an explicit concrete restore plan, which is not available in the P4 behavior API",
            path,
            context,
            root_expected,
        )),
        RecoverableState::NativeHandle(_) => Err(RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::NativeMissingAdapter,
            "native handle restore requires an explicit native adapter hook, which is not available in the P4 any-I behavior API",
            context,
            root_expected,
        )
        .with_detail(serde_json::json!({
            "nodePath": path,
            "reason": "native adapter decode hook is not registered",
        }))
        .into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_interface_node_with_behavior(
    state: &InterfaceValueState,
    expected_for_node: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
    decode_policy: RecoverableDecodePolicy,
) -> Result<RuntimeValue> {
    let expected_any = expected_any_interface_for_node(expected_for_node, path)
        .map_err(|error| expected_type_mismatch_error(error, "decode", context, root_expected))?;
    match state {
        InterfaceValueState::Local { self_node } => decode_local_interface_node_with_behavior(
            self_node,
            expected_any,
            path,
            context,
            root_expected,
            heap,
            behavior_hooks,
            decode_policy,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_local_interface_node_with_behavior(
    self_node: &RecoverableNode,
    expected_any: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
    decode_policy: RecoverableDecodePolicy,
) -> Result<RuntimeValue> {
    validate_local_interface_self_node(
        self_node,
        &format!("{path}.selfNode"),
        context,
        root_expected,
    )?;

    let restored = behavior_hooks
        .restore_local_interface_self(
            RecoverableLocalInterfaceRestoreRequest {
                interface_identity: &expected_any.interface_identity,
                method_projection_identity: &expected_any.method_projection_identity,
                expected_any_interface: expected_any,
                self_node,
                path,
                context,
                expected: root_expected,
                decode_policy,
            },
            heap,
        )?
        .ok_or_else(|| {
            unsupported_decode_error(
                "local InterfaceValue restore requires a registered behavior hook for the self LocalConcrete identity",
                path,
                context,
                root_expected,
            )
        })?;
    if restored.concrete_type_identity.is_empty() {
        return Err(code_identity_missing_error(
            "local InterfaceValue restore hook returned an empty concrete type identity",
            path,
            context,
            root_expected,
        ));
    }
    if restored.runtime_concrete_type_identity.is_empty() {
        return Err(code_identity_missing_error(
            "local InterfaceValue restore hook returned an empty runtime concrete type identity",
            path,
            context,
            root_expected,
        ));
    }

    let conforms = behavior_hooks.concrete_type_conforms_to_interface(
        RecoverableInterfaceConformanceRequest {
            concrete_type_identity: &restored.concrete_type_identity,
            interface_identity: &expected_any.interface_identity,
            method_projection_identity: &expected_any.method_projection_identity,
            expected_any_interface: expected_any,
            path,
            context,
            expected: root_expected,
        },
    )?;
    if !conforms {
        return Err(interface_conformance_missing_error(
            &restored.concrete_type_identity,
            &expected_any.interface_identity,
            &expected_any.method_projection_identity,
            "concrete type no longer conforms to expected any-interface projection",
            path,
            context,
            root_expected,
        ));
    }

    let method_table = behavior_hooks
        .rebuild_local_interface_method_table(RecoverableInterfaceMethodTableRequest {
            concrete_type_identity: &restored.concrete_type_identity,
            interface_identity: &expected_any.interface_identity,
            method_projection_identity: &expected_any.method_projection_identity,
            expected_any_interface: expected_any,
            path,
            context,
            expected: root_expected,
        })?
        .ok_or_else(|| {
            interface_conformance_missing_error(
                &restored.concrete_type_identity,
                &expected_any.interface_identity,
                &expected_any.method_projection_identity,
                "method table rebuild hook did not find a compatible interface projection",
                path,
                context,
                root_expected,
            )
        })?;
    if method_table.interface_abi_id() != expected_any.interface_identity {
        return Err(interface_conformance_missing_error(
            &restored.concrete_type_identity,
            &expected_any.interface_identity,
            &expected_any.method_projection_identity,
            "rebuilt method table targets a different interface identity",
            path,
            context,
            root_expected,
        ));
    }

    Ok(RuntimeValue::Heap(heap.alloc_interface(
        InterfaceValue::new(
            expected_any.interface_identity.clone(),
            InterfaceCarrier::Local {
                concrete_type: restored.runtime_concrete_type_identity,
                method_table,
                payload: restored.payload,
            },
        ),
    )?))
}

fn plain_node(value_kind: RecoverableValueKind, state: RecoverableState) -> RecoverableNode {
    RecoverableNode::plain(value_kind, state)
}

fn recoverable_map_key_from_runtime_key(key: &RuntimeValueKey) -> RecoverableMapKey {
    match key {
        RuntimeValueKey::String(value) => RecoverableMapKey::String(value.clone()),
    }
}

fn runtime_key_from_recoverable_map_key(
    key: &RecoverableMapKey,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<RuntimeValueKey> {
    match key {
        RecoverableMapKey::String(value) => Ok(RuntimeValueKey::string(value)),
        RecoverableMapKey::NominalRepresentation { .. } => Err(unsupported_decode_error(
            "nominal representation map keys require an explicit representation restore hook",
            path,
            context,
            expected,
        )),
    }
}

fn key_label(key: &RecoverableMapKey) -> &str {
    match key {
        RecoverableMapKey::String(value) => value.as_str(),
        RecoverableMapKey::NominalRepresentation {
            representation_identity,
            ..
        } => representation_identity.as_str(),
    }
}

fn reject_behavior_node_for_plain_decode(
    node: &RecoverableNode,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    match &node.code_identity {
        RecoverableCodeIdentity::None => {}
        RecoverableCodeIdentity::LocalConcrete { .. } => {
            return Err(unsupported_decode_error(
                "LocalConcrete recoverable nodes require concrete restore hooks; P3 does not fake nominal/custom recovery",
                path,
                context,
                expected,
            ));
        }
        RecoverableCodeIdentity::NativeAdapter { .. } => {
            return Err(RecoverableBoundaryError::new(
                RecoverableBoundaryErrorCode::NativeMissingAdapter,
                "NativeAdapter recoverable nodes require a registered adapter decode hook",
                context,
                expected,
            )
            .with_detail(serde_json::json!({
                "nodePath": path,
                "reason": "native adapter decode hook is not registered",
            }))
            .into());
        }
    }
    Ok(())
}

fn reject_untrusted_behavior_payload(
    envelope: &RecoverableEnvelope,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    if context.trust_boundary == RuntimeRecoverableTrustBoundary::OwnerInternal {
        return Ok(());
    }
    scan_untrusted_behavior_node(&envelope.root, "$.root", context, expected)
}

fn scan_untrusted_behavior_node(
    node: &RecoverableNode,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    if let Some(reason) = untrusted_behavior_reason(node) {
        return Err(RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::UntrustedBehaviorPayload,
            format!(
                "recoverable behavior payload is not allowed across {} trust boundary",
                context.trust_boundary
            ),
            context,
            expected,
        )
        .with_detail(serde_json::json!({
            "nodePath": path,
            "reason": reason,
            "trustBoundary": context.trust_boundary,
            "boundaryKind": context.kind,
        }))
        .into());
    }

    match &node.state {
        RecoverableState::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                scan_untrusted_behavior_node(item, &format!("{path}[{index}]"), context, expected)?;
            }
        }
        RecoverableState::Map(entries) => {
            for (index, (_key, value)) in entries.iter().enumerate() {
                scan_untrusted_behavior_node(
                    value,
                    &format!("{path}.map[{index}]"),
                    context,
                    expected,
                )?;
            }
        }
        RecoverableState::Record(fields)
        | RecoverableState::NominalObject(NominalObjectState::DefaultFields { fields }) => {
            for field in fields {
                scan_untrusted_behavior_node(
                    &field.value,
                    &format!("{path}.field({})", field.field_identity),
                    context,
                    expected,
                )?;
            }
        }
        RecoverableState::NominalObject(NominalObjectState::Custom { durable_state, .. })
        | RecoverableState::NativeHandle(NativeHandleState { durable_state }) => {
            scan_untrusted_behavior_node(
                durable_state,
                &format!("{path}.durableState"),
                context,
                expected,
            )?;
        }
        RecoverableState::InterfaceValue(InterfaceValueState::Local { self_node }) => {
            scan_untrusted_behavior_node(
                self_node,
                &format!("{path}.selfNode"),
                context,
                expected,
            )?;
        }
        RecoverableState::Null
        | RecoverableState::Bool(_)
        | RecoverableState::Number(_)
        | RecoverableState::String(_)
        | RecoverableState::Bytes(_)
        | RecoverableState::Date(_) => {}
    }
    Ok(())
}

fn untrusted_behavior_reason(node: &RecoverableNode) -> Option<&'static str> {
    match node.value_kind {
        RecoverableValueKind::NominalObject => {
            Some("NominalObject envelope node is behavior-bearing")
        }
        RecoverableValueKind::InterfaceValue => {
            Some("InterfaceValue envelope node is behavior-bearing")
        }
        RecoverableValueKind::NativeHandle => {
            Some("NativeHandle envelope node is behavior-bearing")
        }
        RecoverableValueKind::Null
        | RecoverableValueKind::Bool
        | RecoverableValueKind::Number
        | RecoverableValueKind::String
        | RecoverableValueKind::Bytes
        | RecoverableValueKind::Date
        | RecoverableValueKind::Array
        | RecoverableValueKind::Map
        | RecoverableValueKind::Record => match node.code_identity {
            RecoverableCodeIdentity::None => None,
            RecoverableCodeIdentity::LocalConcrete { .. } => {
                Some("LocalConcrete identity is behavior-bearing")
            }
            RecoverableCodeIdentity::NativeAdapter { .. } => {
                Some("NativeAdapter identity is behavior-bearing")
            }
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpectedTypePrecheckError {
    path: String,
    reason: String,
}

impl ExpectedTypePrecheckError {
    fn new(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

fn precheck_expected_type(
    node: &RecoverableNode,
    expected: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    precheck_expected_type_with_policy(node, expected, path, RecoverableDecodePolicy::strict())
}

fn precheck_expected_type_with_policy(
    node: &RecoverableNode,
    expected: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    decode_policy: RecoverableDecodePolicy,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target } => {
            precheck_expected_type_with_policy(node, target, path, decode_policy)
        }
        RuntimeRecoverableExpectedTypeNode::Nullable { inner } => {
            if matches!(node.state, RecoverableState::Null) {
                Ok(())
            } else {
                precheck_expected_type_with_policy(node, inner, path, decode_policy)
            }
        }
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            let mut errors = Vec::new();
            let mut matches = Vec::new();
            for item in items {
                match precheck_expected_type_with_policy(node, item, path, decode_policy) {
                    Ok(()) => matches.push(item.label.as_str()),
                    Err(error) => errors.push(format!("{}: {}", item.label, error.reason)),
                }
            }
            if matches.len() == 1 {
                return Ok(());
            }
            if matches.len() > 1 {
                return Err(ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "recoverable value matched multiple union branches for {}: {}",
                        expected.diagnostic_label(),
                        matches.join(", ")
                    ),
                ));
            }
            Err(ExpectedTypePrecheckError::new(
                path,
                format!(
                    "recoverable value did not match any union branch for {}: {}",
                    expected.diagnostic_label(),
                    errors.join("; ")
                ),
            ))
        }
        RuntimeRecoverableExpectedTypeNode::LiteralString { value } => match &node.state {
            RecoverableState::String(actual) if actual == value => Ok(()),
            RecoverableState::String(_) => Err(ExpectedTypePrecheckError::new(
                path,
                format!("expected literal string {value:?}"),
            )),
            _ => kind_mismatch(path, "literal string", node.value_kind),
        },
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            precheck_expected_type_with_policy(node, payload, path, decode_policy)
        }
        RuntimeRecoverableExpectedTypeNode::Json => precheck_json_value(node, path),
        RuntimeRecoverableExpectedTypeNode::JsonObject => precheck_json_object(node, path),
        RuntimeRecoverableExpectedTypeNode::Bytes => {
            require_kind(node, path, RecoverableValueKind::Bytes, "bytes")
        }
        RuntimeRecoverableExpectedTypeNode::Date => {
            require_kind(node, path, RecoverableValueKind::Date, "Date")
        }
        RuntimeRecoverableExpectedTypeNode::String => {
            require_kind(node, path, RecoverableValueKind::String, "string")
        }
        RuntimeRecoverableExpectedTypeNode::TaskRef => match &node.state {
            RecoverableState::String(value) if is_canonical_task_ref_string(value) => Ok(()),
            RecoverableState::String(_) => Err(ExpectedTypePrecheckError::new(
                path,
                "expected canonical taskRef string (skiff-task-v1:<owner>.<taskId>)",
            )),
            _ => kind_mismatch(path, "taskRef", node.value_kind),
        },
        RuntimeRecoverableExpectedTypeNode::Bool => {
            require_kind(node, path, RecoverableValueKind::Bool, "bool")
        }
        RuntimeRecoverableExpectedTypeNode::Number => {
            require_kind(node, path, RecoverableValueKind::Number, "number")
        }
        RuntimeRecoverableExpectedTypeNode::Integer => match &node.state {
            RecoverableState::Number(value) if value.to_f64().fract() == 0.0 => Ok(()),
            RecoverableState::Number(_) => Err(ExpectedTypePrecheckError::new(
                path,
                "expected integer number",
            )),
            _ => kind_mismatch(path, "integer", node.value_kind),
        },
        RuntimeRecoverableExpectedTypeNode::Null => {
            require_kind(node, path, RecoverableValueKind::Null, "null")
        }
        RuntimeRecoverableExpectedTypeNode::Stream { .. } => Err(ExpectedTypePrecheckError::new(
            path,
            "Stream handles are request-local and cannot be recovered",
        )),
        RuntimeRecoverableExpectedTypeNode::Array { item } => {
            let RecoverableState::Array(items) = &node.state else {
                return kind_mismatch(path, "array", node.value_kind);
            };
            for (index, item_node) in items.iter().enumerate() {
                precheck_expected_type_with_policy(
                    item_node,
                    item,
                    &format!("{path}[{index}]"),
                    decode_policy,
                )?;
            }
            Ok(())
        }
        RuntimeRecoverableExpectedTypeNode::Map { key, value } => {
            let RecoverableState::Map(entries) = &node.state else {
                return kind_mismatch(path, "map", node.value_kind);
            };
            for (index, (entry_key, entry_value)) in entries.iter().enumerate() {
                precheck_map_key(entry_key, key, &format!("{path}.mapKey[{index}]"))?;
                precheck_expected_type_with_policy(
                    entry_value,
                    value,
                    &format!("{path}.map[{index}]"),
                    decode_policy,
                )?;
            }
            Ok(())
        }
        RuntimeRecoverableExpectedTypeNode::Record { fields, .. } => {
            precheck_record_fields(node, fields, path, decode_policy)
        }
        RuntimeRecoverableExpectedTypeNode::AnyInterface { expected } => {
            precheck_any_interface(node, expected, path)
        }
        RuntimeRecoverableExpectedTypeNode::Unresolved { .. } => Ok(()),
    }
}

fn precheck_any_interface(
    node: &RecoverableNode,
    _expected: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    let RecoverableState::InterfaceValue(_) = &node.state else {
        return kind_mismatch(path, "interface value", node.value_kind);
    };
    Ok(())
}

fn precheck_json_value(
    node: &RecoverableNode,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    match &node.state {
        RecoverableState::Null
        | RecoverableState::Bool(_)
        | RecoverableState::Number(_)
        | RecoverableState::String(_)
        | RecoverableState::Bytes(_)
        | RecoverableState::Date(_) => Ok(()),
        RecoverableState::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                precheck_json_value(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        RecoverableState::Map(entries) => {
            for (index, (key, value)) in entries.iter().enumerate() {
                if !matches!(key, RecoverableMapKey::String(_)) {
                    return Err(ExpectedTypePrecheckError::new(
                        format!("{path}.mapKey[{index}]"),
                        "Json map keys must be plain strings",
                    ));
                }
                precheck_json_value(value, &format!("{path}.map[{index}]"))?;
            }
            Ok(())
        }
        RecoverableState::Record(fields) => {
            for field in fields {
                precheck_json_value(
                    &field.value,
                    &format!("{path}.field({})", field.field_identity),
                )?;
            }
            Ok(())
        }
        RecoverableState::NominalObject(_)
        | RecoverableState::InterfaceValue(_)
        | RecoverableState::NativeHandle(_) => Err(ExpectedTypePrecheckError::new(
            path,
            "Json expected type does not accept behavior-bearing recoverable nodes",
        )),
    }
}

fn precheck_json_object(
    node: &RecoverableNode,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    match &node.state {
        RecoverableState::Record(fields) => {
            for field in fields {
                precheck_json_value(
                    &field.value,
                    &format!("{path}.field({})", field.field_identity),
                )?;
            }
            Ok(())
        }
        RecoverableState::Map(entries) => {
            for (index, (key, value)) in entries.iter().enumerate() {
                if !matches!(key, RecoverableMapKey::String(_)) {
                    return Err(ExpectedTypePrecheckError::new(
                        format!("{path}.mapKey[{index}]"),
                        "JsonObject map keys must be plain strings",
                    ));
                }
                precheck_json_value(value, &format!("{path}.map[{index}]"))?;
            }
            Ok(())
        }
        _ => kind_mismatch(path, "JsonObject", node.value_kind),
    }
}

fn select_expected_plan_for_node_with_policy<'a>(
    node: &RecoverableNode,
    expected: &'a RuntimeRecoverableExpectedTypePlan,
    path: &str,
    decode_policy: RecoverableDecodePolicy,
) -> std::result::Result<&'a RuntimeRecoverableExpectedTypePlan, ExpectedTypePrecheckError> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target } => {
            select_expected_plan_for_node_with_policy(node, target, path, decode_policy)
        }
        RuntimeRecoverableExpectedTypeNode::Nullable { inner } => {
            if matches!(node.state, RecoverableState::Null) {
                Ok(expected)
            } else {
                select_expected_plan_for_node_with_policy(node, inner, path, decode_policy)
            }
        }
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            let mut matches = Vec::new();
            for item in items {
                if precheck_expected_type_with_policy(node, item, path, decode_policy).is_ok() {
                    matches.push(item);
                }
            }
            if matches.len() == 1 {
                return select_expected_plan_for_node_with_policy(
                    node,
                    matches[0],
                    path,
                    decode_policy,
                );
            }
            if matches.len() > 1 {
                return Err(ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "recoverable value matched multiple union branches for {}: {}",
                        expected.diagnostic_label(),
                        matches
                            .iter()
                            .map(|item| item.label.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            Err(ExpectedTypePrecheckError::new(
                path,
                format!(
                    "recoverable value did not match any union branch for {}",
                    expected.diagnostic_label()
                ),
            ))
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            select_expected_plan_for_node_with_policy(node, payload, path, decode_policy)
        }
        _ => {
            precheck_expected_type_with_policy(node, expected, path, decode_policy)?;
            Ok(expected)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn select_expected_plan_for_node_with_behavior_policy<'a>(
    node: &RecoverableNode,
    expected: &'a RuntimeRecoverableExpectedTypePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
    decode_policy: RecoverableDecodePolicy,
    operation: &'static str,
) -> Result<&'a RuntimeRecoverableExpectedTypePlan> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target } => {
            select_expected_plan_for_node_with_behavior_policy(
                node,
                target,
                path,
                context,
                root_expected,
                behavior_hooks,
                decode_policy,
                operation,
            )
        }
        RuntimeRecoverableExpectedTypeNode::Nullable { inner } => {
            if matches!(node.state, RecoverableState::Null) {
                Ok(expected)
            } else {
                select_expected_plan_for_node_with_behavior_policy(
                    node,
                    inner,
                    path,
                    context,
                    root_expected,
                    behavior_hooks,
                    decode_policy,
                    operation,
                )
            }
        }
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            let mut matches = Vec::new();
            let mut errors = Vec::new();
            for item in items {
                match behavior_union_branch_matches(
                    node,
                    item,
                    path,
                    context,
                    root_expected,
                    behavior_hooks,
                    decode_policy,
                )? {
                    Ok(()) => matches.push(item),
                    Err(error) => errors.push(format!("{}: {}", item.label, error.reason)),
                }
            }
            if matches.len() == 1 {
                return select_expected_plan_for_node_with_behavior_policy(
                    node,
                    matches[0],
                    path,
                    context,
                    root_expected,
                    behavior_hooks,
                    decode_policy,
                    operation,
                );
            }
            if matches.len() > 1 {
                return Err(expected_type_mismatch_error(
                    ExpectedTypePrecheckError::new(
                        path,
                        format!(
                            "recoverable value matched multiple union branches for {}: {}",
                            expected.diagnostic_label(),
                            matches
                                .iter()
                                .map(|item| item.label.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ),
                    operation,
                    context,
                    root_expected,
                ));
            }
            Err(expected_type_mismatch_error(
                ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "recoverable value did not match any union branch for {}: {}",
                        expected.diagnostic_label(),
                        errors.join("; ")
                    ),
                ),
                operation,
                context,
                root_expected,
            ))
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            select_expected_plan_for_node_with_behavior_policy(
                node,
                payload,
                path,
                context,
                root_expected,
                behavior_hooks,
                decode_policy,
                operation,
            )
        }
        _ => select_expected_plan_for_node_with_policy(node, expected, path, decode_policy)
            .map_err(|error| {
                expected_type_mismatch_error(error, operation, context, root_expected)
            }),
    }
}

fn behavior_union_branch_matches(
    node: &RecoverableNode,
    expected: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
    decode_policy: RecoverableDecodePolicy,
) -> Result<std::result::Result<(), ExpectedTypePrecheckError>> {
    if matches!(node.state, RecoverableState::Null) {
        return Ok(precheck_expected_type_with_policy(
            node,
            expected,
            path,
            decode_policy,
        ));
    }
    let Some(expected_any) = expected_any_interface_candidate(expected) else {
        return Ok(precheck_expected_type_with_policy(
            node,
            expected,
            path,
            decode_policy,
        ));
    };
    let RecoverableState::InterfaceValue(state) = &node.state else {
        return Ok(Err(ExpectedTypePrecheckError::new(
            path,
            format!(
                "expected interface value but recoverable node kind was {:?}",
                node.value_kind
            ),
        )));
    };
    match state {
        InterfaceValueState::Local { .. } => {
            let concrete_type_identity =
                match local_concrete_identity_for_interface_precheck(node, path) {
                    Ok(identity) => identity,
                    Err(error) => return Ok(Err(error)),
                };
            let conforms = behavior_hooks.concrete_type_conforms_to_interface(
                RecoverableInterfaceConformanceRequest {
                    concrete_type_identity,
                    interface_identity: &expected_any.interface_identity,
                    method_projection_identity: &expected_any.method_projection_identity,
                    expected_any_interface: expected_any,
                    path,
                    context,
                    expected: root_expected,
                },
            )?;
            if conforms {
                Ok(Ok(()))
            } else {
                Ok(Err(ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "local concrete {concrete_type_identity} does not conform to any-interface {} projection {}",
                        expected_any.interface_identity, expected_any.method_projection_identity
                    ),
                )))
            }
        }
    }
}

fn expected_any_interface_candidate(
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Option<&RuntimeRecoverableExpectedAnyInterfacePlan> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target }
        | RuntimeRecoverableExpectedTypeNode::Nullable { inner: target } => {
            expected_any_interface_candidate(target)
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            expected_any_interface_candidate(payload)
        }
        RuntimeRecoverableExpectedTypeNode::AnyInterface { expected } => Some(expected),
        _ => None,
    }
}

fn local_concrete_identity_for_interface_precheck<'a>(
    node: &'a RecoverableNode,
    path: &str,
) -> std::result::Result<&'a str, ExpectedTypePrecheckError> {
    let RecoverableState::InterfaceValue(InterfaceValueState::Local { self_node }) = &node.state
    else {
        return Err(ExpectedTypePrecheckError::new(
            path,
            "InterfaceValue union branch selection requires LocalConcrete self identity",
        ));
    };
    let RecoverableCodeIdentity::LocalConcrete {
        concrete_type_identity,
        ..
    } = &self_node.code_identity
    else {
        return Err(ExpectedTypePrecheckError::new(
            path,
            "InterfaceValue union branch selection requires LocalConcrete self identity",
        ));
    };
    if concrete_type_identity.is_empty() {
        return Err(ExpectedTypePrecheckError::new(
            path,
            "InterfaceValue union branch selection requires non-empty LocalConcrete identity",
        ));
    }
    Ok(concrete_type_identity)
}

fn expected_array_item_plan(
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Option<&RuntimeRecoverableExpectedTypePlan> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Array { item } => Some(item),
        _ => None,
    }
}

fn expected_map_value_plan(
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Option<&RuntimeRecoverableExpectedTypePlan> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Map { value, .. } => Some(value),
        _ => None,
    }
}

fn expected_record_field_plan<'a>(
    expected: &'a RuntimeRecoverableExpectedTypePlan,
    field_identity: &str,
) -> Option<&'a RuntimeRecoverableExpectedTypePlan> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Record { fields, .. } => fields
            .iter()
            .find(|field| field.name == field_identity)
            .map(|field| &field.ty),
        _ => None,
    }
}

fn expected_record_fields(
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Option<&[skiff_runtime_model::recoverable::RuntimeRecoverableExpectedRecordFieldPlan]> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Record { fields, .. } => Some(fields.as_slice()),
        _ => None,
    }
}

fn recoverable_child_fallback_expected<'a>(
    selected_expected: &'a RuntimeRecoverableExpectedTypePlan,
    json_child_expected: &'a RuntimeRecoverableExpectedTypePlan,
) -> &'a RuntimeRecoverableExpectedTypePlan {
    if expected_decodes_children_as_json(selected_expected) {
        json_child_expected
    } else {
        selected_expected
    }
}

fn expected_decodes_children_as_json(expected: &RuntimeRecoverableExpectedTypePlan) -> bool {
    matches!(
        expected.node,
        RuntimeRecoverableExpectedTypeNode::Json | RuntimeRecoverableExpectedTypeNode::JsonObject
    )
}

fn json_value_child_expected_plan() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "Json".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Json,
    }
}

fn materialize_missing_nullable_record_fields(
    decoded: &mut RuntimeObjectFields,
    expected: &RuntimeRecoverableExpectedTypePlan,
    decode_policy: RecoverableDecodePolicy,
) {
    if !decode_policy.materializes_missing_nullable_record_fields() {
        return;
    }
    let Some(fields) = expected_record_fields(expected) else {
        return;
    };
    for field in fields {
        if !field.required
            && !decoded.contains_key(&field.name)
            && expected_type_accepts_null(&field.ty)
        {
            decoded.insert(field.name.clone(), RuntimeValue::Null);
        }
    }
}

fn expected_type_accepts_null(expected: &RuntimeRecoverableExpectedTypePlan) -> bool {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target } => expected_type_accepts_null(target),
        RuntimeRecoverableExpectedTypeNode::Nullable { .. }
        | RuntimeRecoverableExpectedTypeNode::Null => true,
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            items.iter().any(expected_type_accepts_null)
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            expected_type_accepts_null(payload)
        }
        _ => false,
    }
}

fn expected_any_interface_for_node<'a>(
    expected: &'a RuntimeRecoverableExpectedTypePlan,
    path: &str,
) -> std::result::Result<&'a RuntimeRecoverableExpectedAnyInterfacePlan, ExpectedTypePrecheckError>
{
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::AnyInterface { expected } => Ok(expected),
        _ => Err(ExpectedTypePrecheckError::new(
            path,
            "InterfaceValue decode requires an expected any-interface identity and method projection",
        )),
    }
}

fn precheck_map_key(
    key: &RecoverableMapKey,
    expected: &RuntimeRecoverableExpectedTypePlan,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    match &expected.node {
        RuntimeRecoverableExpectedTypeNode::Alias { target } => precheck_map_key(key, target, path),
        RuntimeRecoverableExpectedTypeNode::Union { items } => {
            let mut errors = Vec::new();
            let mut matches = Vec::new();
            for item in items {
                match precheck_map_key(key, item, path) {
                    Ok(()) => matches.push(item.label.as_str()),
                    Err(error) => errors.push(format!("{}: {}", item.label, error.reason)),
                }
            }
            if matches.len() == 1 {
                return Ok(());
            }
            if matches.len() > 1 {
                return Err(ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "recoverable map key matched multiple union branches: {}",
                        matches.join(", ")
                    ),
                ));
            }
            Err(ExpectedTypePrecheckError::new(
                path,
                format!(
                    "recoverable map key did not match any union branch: {}",
                    errors.join("; ")
                ),
            ))
        }
        RuntimeRecoverableExpectedTypeNode::Representation { payload, .. } => {
            precheck_map_key(key, payload, path)
        }
        RuntimeRecoverableExpectedTypeNode::LiteralString { value } => match key {
            RecoverableMapKey::String(actual) if actual == value => Ok(()),
            RecoverableMapKey::String(_) => Err(ExpectedTypePrecheckError::new(
                path,
                format!("expected literal map key {value:?}"),
            )),
            RecoverableMapKey::NominalRepresentation { .. } => Err(ExpectedTypePrecheckError::new(
                path,
                "nominal representation map keys are not supported by the P3 runtime codec",
            )),
        },
        RuntimeRecoverableExpectedTypeNode::String
        | RuntimeRecoverableExpectedTypeNode::Json
        | RuntimeRecoverableExpectedTypeNode::Unresolved { .. } => match key {
            RecoverableMapKey::String(_) => Ok(()),
            RecoverableMapKey::NominalRepresentation { .. } => Err(ExpectedTypePrecheckError::new(
                path,
                "nominal representation map keys are not supported by the P3 runtime codec",
            )),
        },
        _ => Err(ExpectedTypePrecheckError::new(
            path,
            format!(
                "recoverable map key expected type {} is not supported",
                expected.diagnostic_label()
            ),
        )),
    }
}

fn precheck_record_fields(
    node: &RecoverableNode,
    fields: &[skiff_runtime_model::recoverable::RuntimeRecoverableExpectedRecordFieldPlan],
    path: &str,
    decode_policy: RecoverableDecodePolicy,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    let RecoverableState::Record(actual_fields) = &node.state else {
        return kind_mismatch(path, "record", node.value_kind);
    };
    let actual_by_name = actual_fields
        .iter()
        .map(|field| (field.field_identity.as_str(), &field.value))
        .collect::<BTreeMap<_, _>>();
    let allowed = fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();

    for field in fields {
        match actual_by_name.get(field.name.as_str()) {
            Some(value) => {
                precheck_expected_type_with_policy(
                    value,
                    &field.ty,
                    &format!("{path}.field({})", field.name),
                    decode_policy,
                )?;
            }
            None if field.required => {
                return Err(ExpectedTypePrecheckError::new(
                    path,
                    format!("record field {} is required", field.name),
                ));
            }
            None => {}
        }
    }

    for field in actual_fields {
        if !allowed.contains(field.field_identity.as_str()) {
            if decode_policy.ignores_unknown_record_fields() {
                continue;
            }
            return Err(ExpectedTypePrecheckError::new(
                format!("{path}.field({})", field.field_identity),
                format!(
                    "record field {} is not declared by expected type {}",
                    field.field_identity, "record"
                ),
            ));
        }
    }
    Ok(())
}

fn require_kind(
    node: &RecoverableNode,
    path: &str,
    expected_kind: RecoverableValueKind,
    expected_label: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    if node.value_kind == expected_kind {
        Ok(())
    } else {
        kind_mismatch(path, expected_label, node.value_kind)
    }
}

fn kind_mismatch(
    path: &str,
    expected_label: &str,
    actual: RecoverableValueKind,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    Err(ExpectedTypePrecheckError::new(
        path,
        format!(
            "expected recoverable {expected_label}, got {}",
            recoverable_value_kind_label(actual)
        ),
    ))
}

/// Canonical `taskRef` string check mirroring the wire `TaskRef` format
/// (`skiff-task-v1:<base64url-nopad(owner)>.<base64url-nopad(taskId)>`).
/// The recoverable boundary owns this check so opaque task references can be
/// restored from durable payloads without a transport dependency.
pub fn is_canonical_task_ref_string(raw: &str) -> bool {
    let Some(rest) = raw.strip_prefix("skiff-task-v1:") else {
        return false;
    };
    let Some((owner_encoded, task_encoded)) = rest.split_once('.') else {
        return false;
    };
    if owner_encoded.is_empty() || task_encoded.is_empty() {
        return false;
    }
    decode_task_ref_segment(owner_encoded).is_some()
        && decode_task_ref_segment(task_encoded).is_some()
}

fn decode_task_ref_segment(encoded: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    (!decoded.trim().is_empty()).then_some(decoded)
}

fn recoverable_value_kind_label(kind: RecoverableValueKind) -> &'static str {
    match kind {
        RecoverableValueKind::Null => "null",
        RecoverableValueKind::Bool => "bool",
        RecoverableValueKind::Number => "number",
        RecoverableValueKind::String => "string",
        RecoverableValueKind::Bytes => "bytes",
        RecoverableValueKind::Date => "Date",
        RecoverableValueKind::Array => "array",
        RecoverableValueKind::Map => "map",
        RecoverableValueKind::Record => "record",
        RecoverableValueKind::NominalObject => "nominal object",
        RecoverableValueKind::InterfaceValue => "interface value",
        RecoverableValueKind::NativeHandle => "native handle",
    }
}

fn validate_local_interface_self_node(
    node: &RecoverableNode,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    if node.value_kind != RecoverableValueKind::NominalObject
        || !matches!(node.state, RecoverableState::NominalObject(_))
    {
        return Err(state_invalid_message_error(
            "InterfaceValue self_node must be a NominalObject recoverable node",
            path,
            context,
            expected,
        ));
    }
    match &node.code_identity {
        RecoverableCodeIdentity::LocalConcrete {
            concrete_type_identity,
            ..
        } if !concrete_type_identity.is_empty() => Ok(()),
        RecoverableCodeIdentity::LocalConcrete { .. } => Err(code_identity_missing_error(
            "InterfaceValue self_node LocalConcrete identity must include concrete type identity",
            path,
            context,
            expected,
        )),
        RecoverableCodeIdentity::None | RecoverableCodeIdentity::NativeAdapter { .. } => {
            Err(code_identity_missing_error(
                "InterfaceValue self_node must carry LocalConcrete identity",
                path,
                context,
                expected,
            ))
        }
    }
}

fn interface_encode_error(
    value: &InterfaceValue,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    if context.trust_boundary == RuntimeRecoverableTrustBoundary::CrossService {
        return RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::CrossServiceInterfaceCallbackUnavailable,
            "InterfaceValue cannot be encoded across crossService recoverable boundary because callback transport is unavailable",
            context,
            expected,
        )
        .with_detail(serde_json::json!({
            "nodePath": path,
            "reason": value.diagnostic_label(),
            "trustBoundary": context.trust_boundary,
        }))
        .into();
    }
    if context.trust_boundary != RuntimeRecoverableTrustBoundary::OwnerInternal {
        return RecoverableBoundaryError::new(
            RecoverableBoundaryErrorCode::UntrustedBehaviorPayload,
            format!(
                "InterfaceValue cannot be encoded across {} trust boundary",
                context.trust_boundary
            ),
            context,
            expected,
        )
        .with_detail(serde_json::json!({
            "nodePath": path,
            "reason": value.diagnostic_label(),
            "trustBoundary": context.trust_boundary,
        }))
        .into();
    }
    unsupported_encode_error(
        format!(
            "{} requires P4 any-I wrapper recovery and is not encoded by the P3 plain codec",
            value.diagnostic_label()
        ),
        path,
        context,
        expected,
    )
}

fn code_identity_missing_error(
    reason: impl Into<String>,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::CodeIdentityMissing,
        reason,
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
    }))
    .into()
}

fn interface_conformance_missing_error(
    concrete_type_identity: &str,
    interface_identity: &str,
    method_projection_identity: &str,
    reason: &str,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::InterfaceConformanceMissing,
        "recoverable InterfaceValue concrete self no longer conforms to expected any-interface projection",
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
        "concreteTypeIdentity": concrete_type_identity,
        "interfaceIdentity": interface_identity,
        "methodProjectionIdentity": method_projection_identity,
        "reason": reason,
    }))
    .into()
}

fn state_invalid_message_error(
    reason: impl Into<String>,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    let reason = reason.into();
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::StateInvalid,
        reason.clone(),
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
        "reason": reason,
    }))
    .into()
}

fn unsupported_encode_error(
    reason: impl Into<String>,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::UnsupportedEncode,
        reason,
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
    }))
    .into()
}

fn unsupported_decode_error(
    reason: impl Into<String>,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::UnsupportedDecode,
        reason,
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
    }))
    .into()
}

fn expected_type_mismatch_error(
    error: ExpectedTypePrecheckError,
    operation: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::ExpectedTypeMismatch,
        format!(
            "recoverable {operation} expected type precheck failed for {} at {}: {}",
            expected.diagnostic_label(),
            error.path,
            error.reason
        ),
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": error.path,
        "reason": error.reason,
    }))
    .into()
}

fn state_invalid_error(
    error: skiff_runtime_model::recoverable::RecoverableStateInvalid,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::StateInvalid,
        error.to_string(),
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": error.path(),
        "reason": error.message(),
    }))
    .into()
}

pub fn retention_roots_for_refs(
    refs: &[RecoverableArtifactRef],
    context: &RuntimeRecoverableBoundaryContext,
    expires_at_epoch_millis: Option<i64>,
) -> Vec<RecoverableArtifactRetentionRoot> {
    let service_id = recoverable_service_id(context);
    refs.iter()
        .map(|artifact_ref| RecoverableArtifactRetentionRoot {
            service_id: service_id.clone(),
            artifact_identity: artifact_ref.artifact_identity.clone(),
            build_id: artifact_ref.build_id.clone(),
            boundary_kind: context.kind,
            expires_at_epoch_millis,
        })
        .collect()
}

fn artifact_unavailable_error(
    artifact_ref: &RecoverableArtifactRef,
    reason: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RecoverableBoundaryError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::ArtifactUnavailable,
        format!(
            "recoverable artifact {} build {} is unavailable for {} boundary",
            artifact_ref.artifact_identity, artifact_ref.build_id, context.kind
        ),
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "serviceId": recoverable_service_id(context),
        "artifactIdentity": artifact_ref.artifact_identity,
        "buildId": artifact_ref.build_id,
        "boundaryKind": context.kind,
        "nodePath": artifact_ref.node_path,
        "reason": reason,
    }))
}

fn recoverable_service_id(context: &RuntimeRecoverableBoundaryContext) -> String {
    context
        .origin_service
        .as_ref()
        .or(context.target_service.as_ref())
        .map(|service| service.service_id.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests;
