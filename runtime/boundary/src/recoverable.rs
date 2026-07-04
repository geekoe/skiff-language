use std::collections::{BTreeMap, BTreeSet, HashSet};

use skiff_runtime_model::{
    recoverable::{
        InterfaceValueState, NativeHandleState, NominalObjectState, RecoverableArtifactRef,
        RecoverableArtifactRetentionRoot, RecoverableCodeIdentity, RecoverableDate,
        RecoverableEnvelope, RecoverableField, RecoverableMapKey, RecoverableNode,
        RecoverableNumber, RecoverableRemoteInterfaceCarrier, RecoverableRemoteOperationSlot,
        RecoverableRemoteOperationTable, RecoverableState, RecoverableValidationLimits,
        RecoverableValueKind, RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableExpectedAnyInterfacePlan, RuntimeRecoverableExpectedTypeNode,
        RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{
        HeapHandle, HeapNode, InterfaceCarrier, InterfaceMethodTable, InterfaceValue,
        RemoteOperationSlot, RemoteOperationTable, RuntimeMap, RuntimeObject, RuntimeObjectFields,
        RuntimeValue, RuntimeValueKey,
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

pub struct RecoverableRemoteInterfaceCarrierRequest<'a> {
    pub interface_identity: &'a str,
    pub method_projection_identity: &'a str,
    pub expected_any_interface: &'a RuntimeRecoverableExpectedAnyInterfacePlan,
    pub carrier: &'a RecoverableRemoteInterfaceCarrier,
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

    fn rebuild_remote_interface_operation_table(
        &self,
        request: RecoverableRemoteInterfaceCarrierRequest<'_>,
    ) -> Result<Option<RemoteOperationTable>>;
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

    fn rebuild_remote_interface_operation_table(
        &self,
        _request: RecoverableRemoteInterfaceCarrierRequest<'_>,
    ) -> Result<Option<RemoteOperationTable>> {
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
            InterfaceCarrier::Remote {
                dependency_ref,
                public_instance_key,
                operations,
            } => {
                let carrier = recoverable_remote_interface_carrier_from_runtime(
                    dependency_ref,
                    public_instance_key,
                    operations,
                );
                let node = RecoverableNode {
                    value_kind: RecoverableValueKind::InterfaceValue,
                    variant_identity: RecoverableVariantIdentity::None,
                    code_identity: RecoverableCodeIdentity::None,
                    state: RecoverableState::InterfaceValue(InterfaceValueState::Remote {
                        carrier,
                    }),
                };
                let RecoverableState::InterfaceValue(InterfaceValueState::Remote { carrier }) =
                    &node.state
                else {
                    unreachable!("remote InterfaceValue node was constructed above");
                };
                validate_remote_interface_carrier_for_encode(
                    &node,
                    carrier,
                    path,
                    self.context,
                    self.expected,
                    behavior_hooks,
                )?;
                Ok(node)
            }
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
            let mut decoded = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                decoded.push(decode_node(
                    item,
                    child_expected.unwrap_or(selected_expected),
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
            let mut decoded = RuntimeMap::new();
            for (index, (key, value)) in entries.iter().enumerate() {
                let key =
                    runtime_key_from_recoverable_map_key(key, &format!("{path}.mapKey[{index}]"), context, root_expected)?;
                let value = decode_node(
                    value,
                    child_expected.unwrap_or(selected_expected),
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
                        field_expected.unwrap_or(selected_expected),
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
            let mut decoded = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                decoded.push(decode_node_with_behavior(
                    item,
                    child_expected.unwrap_or(selected_expected),
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
                    child_expected.unwrap_or(selected_expected),
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
            let mut decoded = RuntimeObjectFields::new();
            for field in fields {
                let field_expected =
                    expected_record_field_plan(selected_expected, &field.field_identity)
                        ;
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
                        field_expected.unwrap_or(selected_expected),
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
        InterfaceValueState::Remote { carrier } => decode_remote_interface_node_with_behavior(
            carrier,
            expected_any,
            path,
            context,
            root_expected,
            heap,
            behavior_hooks,
        ),
    }
}

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
                concrete_type: restored.concrete_type_identity,
                method_table,
                payload: restored.payload,
            },
        ),
    )?))
}

fn decode_remote_interface_node_with_behavior(
    carrier: &RecoverableRemoteInterfaceCarrier,
    expected_any: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<RuntimeValue> {
    validate_remote_interface_carrier_matches_expected(
        carrier,
        expected_any,
        path,
        context,
        root_expected,
        "decode",
    )?;
    let operations = behavior_hooks
        .rebuild_remote_interface_operation_table(RecoverableRemoteInterfaceCarrierRequest {
            interface_identity: &expected_any.interface_identity,
            method_projection_identity: &expected_any.method_projection_identity,
            expected_any_interface: expected_any,
            carrier,
            path,
            context,
            expected: root_expected,
        })?
        .ok_or_else(|| {
            remote_carrier_not_persistable_error(
                "current linked program does not provide a matching remote InterfaceValue carrier",
                carrier,
                path,
                context,
                root_expected,
            )
        })?;
    validate_restored_remote_operation_table(
        carrier,
        &operations,
        expected_any,
        path,
        context,
        root_expected,
    )?;
    Ok(RuntimeValue::Heap(heap.alloc_interface(
        InterfaceValue::new(
            expected_any.interface_identity.clone(),
            InterfaceCarrier::Remote {
                dependency_ref: carrier.dependency_ref.clone(),
                public_instance_key: carrier.public_instance_key.clone(),
                operations,
            },
        ),
    )?))
}

fn plain_node(value_kind: RecoverableValueKind, state: RecoverableState) -> RecoverableNode {
    RecoverableNode::plain(value_kind, state)
}

fn recoverable_remote_interface_carrier_from_runtime(
    dependency_ref: &str,
    public_instance_key: &str,
    operations: &RemoteOperationTable,
) -> RecoverableRemoteInterfaceCarrier {
    RecoverableRemoteInterfaceCarrier {
        dependency_ref: dependency_ref.to_string(),
        public_instance_key: public_instance_key.to_string(),
        operations: RecoverableRemoteOperationTable {
            id: operations.id().to_string(),
            interface_abi_id: operations.interface_abi_id().to_string(),
            slots: operations
                .slots()
                .iter()
                .map(|slot| RecoverableRemoteOperationSlot {
                    slot: slot.slot(),
                    method_abi_id: slot.method_abi_id().to_string(),
                    operation_abi_id: slot.operation_abi_id().to_string(),
                })
                .collect(),
        },
    }
}

fn remote_operation_table_from_recoverable(
    carrier: &RecoverableRemoteInterfaceCarrier,
) -> RemoteOperationTable {
    RemoteOperationTable::new(
        carrier.operations.id.clone(),
        carrier.operations.interface_abi_id.clone(),
        carrier
            .operations
            .slots
            .iter()
            .map(|slot| {
                RemoteOperationSlot::new(
                    slot.slot,
                    slot.method_abi_id.clone(),
                    slot.operation_abi_id.clone(),
                )
            })
            .collect(),
    )
}

fn remote_operation_tables_runtime_equivalent(
    left: &RemoteOperationTable,
    right: &RemoteOperationTable,
) -> bool {
    left.id() == right.id()
        && left.interface_abi_id() == right.interface_abi_id()
        && left.slots() == right.slots()
}

fn validate_remote_interface_carrier_for_encode(
    node: &RecoverableNode,
    carrier: &RecoverableRemoteInterfaceCarrier,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    root_expected: &RuntimeRecoverableExpectedTypePlan,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<()> {
    let selected_expected = select_expected_plan_for_node_with_behavior_policy(
        node,
        root_expected,
        path,
        context,
        root_expected,
        behavior_hooks,
        RecoverableDecodePolicy::strict(),
        "encode",
    )?;
    let expected_any = expected_any_interface_for_node(selected_expected, path)
        .map_err(|error| expected_type_mismatch_error(error, "encode", context, root_expected))?;
    validate_remote_interface_carrier_matches_expected(
        carrier,
        expected_any,
        path,
        context,
        root_expected,
        "encode",
    )?;
    let operations = behavior_hooks
        .rebuild_remote_interface_operation_table(RecoverableRemoteInterfaceCarrierRequest {
            interface_identity: &expected_any.interface_identity,
            method_projection_identity: &expected_any.method_projection_identity,
            expected_any_interface: expected_any,
            carrier,
            path,
            context,
            expected: root_expected,
        })?
        .ok_or_else(|| {
            remote_carrier_not_persistable_error(
                "current linked program does not provide a matching remote InterfaceValue carrier",
                carrier,
                path,
                context,
                root_expected,
            )
        })?;
    validate_restored_remote_operation_table(
        carrier,
        &operations,
        expected_any,
        path,
        context,
        root_expected,
    )
}

fn validate_remote_interface_carrier_matches_expected(
    carrier: &RecoverableRemoteInterfaceCarrier,
    expected_any: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
    operation: &'static str,
) -> Result<()> {
    remote_interface_carrier_precheck(carrier, expected_any, path)
        .map_err(|error| expected_type_mismatch_error(error, operation, context, expected))
}

fn remote_interface_carrier_precheck(
    carrier: &RecoverableRemoteInterfaceCarrier,
    expected_any: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
) -> std::result::Result<(), ExpectedTypePrecheckError> {
    if carrier.operations.interface_abi_id != expected_any.interface_identity {
        return Err(ExpectedTypePrecheckError::new(
            path,
            format!(
                "remote InterfaceValue operation table targets interface {}, expected {}",
                carrier.operations.interface_abi_id, expected_any.interface_identity
            ),
        ));
    }
    Ok(())
}

fn validate_restored_remote_operation_table(
    carrier: &RecoverableRemoteInterfaceCarrier,
    operations: &RemoteOperationTable,
    expected_any: &RuntimeRecoverableExpectedAnyInterfacePlan,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> Result<()> {
    if operations.interface_abi_id() != expected_any.interface_identity {
        return Err(remote_carrier_not_persistable_error(
            "rebuilt remote InterfaceValue operation table targets a different interface identity",
            carrier,
            path,
            context,
            expected,
        ));
    }
    let persisted = remote_operation_table_from_recoverable(carrier);
    if !remote_operation_tables_runtime_equivalent(&persisted, operations) {
        return Err(remote_carrier_not_persistable_error(
            "rebuilt remote InterfaceValue operation table no longer matches persisted carrier state",
            carrier,
            path,
            context,
            expected,
        ));
    }
    Ok(())
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
        RecoverableState::InterfaceValue(InterfaceValueState::Remote { .. }) => {}
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
        InterfaceValueState::Remote { carrier } => {
            if let Err(error) = remote_interface_carrier_precheck(carrier, expected_any, path) {
                return Ok(Err(error));
            }
            let operations = behavior_hooks.rebuild_remote_interface_operation_table(
                RecoverableRemoteInterfaceCarrierRequest {
                    interface_identity: &expected_any.interface_identity,
                    method_projection_identity: &expected_any.method_projection_identity,
                    expected_any_interface: expected_any,
                    carrier,
                    path,
                    context,
                    expected: root_expected,
                },
            )?;
            if operations.is_some() {
                Ok(Ok(()))
            } else {
                Ok(Err(ExpectedTypePrecheckError::new(
                    path,
                    format!(
                        "remote carrier {}/{} does not conform to any-interface {} projection {}",
                        carrier.dependency_ref,
                        carrier.public_instance_key,
                        expected_any.interface_identity,
                        expected_any.method_projection_identity
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
                &format!("{path}.field({})", field.field_identity),
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

fn remote_carrier_not_persistable_error(
    reason: &str,
    carrier: &RecoverableRemoteInterfaceCarrier,
    path: &str,
    context: &RuntimeRecoverableBoundaryContext,
    expected: &RuntimeRecoverableExpectedTypePlan,
) -> RuntimeError {
    RecoverableBoundaryError::new(
        RecoverableBoundaryErrorCode::RemoteCarrierNotPersistable,
        "InterfaceCarrier::Remote cannot be recovered from the current owner-internal linked program",
        context,
        expected,
    )
    .with_detail(serde_json::json!({
        "nodePath": path,
        "reason": reason,
        "carrier": "remote",
        "dependencyRef": carrier.dependency_ref,
        "publicInstanceKey": carrier.public_instance_key,
        "operationTableId": carrier.operations.id,
    }))
    .into()
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
            "recoverable {operation} expected type precheck failed for {}: {}",
            expected.diagnostic_label(),
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
mod tests {
    use serde_json::json;
    use skiff_runtime_model::addr::ExecutableAddr;
    use skiff_runtime_model::recoverable::{
        InterfaceValueState, LocalConcreteOwner, NativeAdapterOwner, NativeHandleState,
        NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope, RecoverableField,
        RecoverableNode, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
        RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableBoundaryKind, RuntimeRecoverableExpectedRecordFieldPlan,
        RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan,
        RuntimeRecoverableServiceRef, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    };
    use skiff_runtime_model::runtime_value::{
        InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable, InterfaceMethodTarget,
        InterfaceReceiverCallAbi, InterfaceValue, RemoteOperationSlot, RemoteOperationTable,
        RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValueKey,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    use super::*;
    use crate::{
        binary::{decode_payload_plan, encode_payload_plan},
        error::{RecoverableBoundaryErrorCode, RuntimeError},
        payload::PayloadBoundary,
        type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt},
    };

    fn runtime_string_plan() -> RuntimeTypePlan {
        RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "string",
            "args": []
        }))
        .expect("string plan should build")
    }

    fn recoverable_context() -> RuntimeRecoverableBoundaryContext {
        RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot()
    }

    fn recoverable_context_with_service() -> RuntimeRecoverableBoundaryContext {
        recoverable_context()
            .with_origin_service(RuntimeRecoverableServiceRef::new("skiff.run/account"))
    }

    fn external_recoverable_context() -> RuntimeRecoverableBoundaryContext {
        RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::ExternalUntrusted,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot()
    }

    fn expected_plan() -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan::unresolved("recoverable")
    }

    fn expected(
        label: &str,
        node: RuntimeRecoverableExpectedTypeNode,
    ) -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan {
            label: label.to_string(),
            identity: None,
            node,
        }
    }

    fn string_expected() -> RuntimeRecoverableExpectedTypePlan {
        expected("string", RuntimeRecoverableExpectedTypeNode::String)
    }

    fn bool_expected() -> RuntimeRecoverableExpectedTypePlan {
        expected("bool", RuntimeRecoverableExpectedTypeNode::Bool)
    }

    fn number_expected() -> RuntimeRecoverableExpectedTypePlan {
        expected("number", RuntimeRecoverableExpectedTypeNode::Number)
    }

    fn bytes_expected() -> RuntimeRecoverableExpectedTypePlan {
        expected("bytes", RuntimeRecoverableExpectedTypeNode::Bytes)
    }

    fn date_expected() -> RuntimeRecoverableExpectedTypePlan {
        expected("Date", RuntimeRecoverableExpectedTypeNode::Date)
    }

    fn map_expected(
        key: RuntimeRecoverableExpectedTypePlan,
        value: RuntimeRecoverableExpectedTypePlan,
    ) -> RuntimeRecoverableExpectedTypePlan {
        expected(
            "Map",
            RuntimeRecoverableExpectedTypeNode::Map {
                key: Box::new(key),
                value: Box::new(value),
            },
        )
    }

    fn array_expected(
        item: RuntimeRecoverableExpectedTypePlan,
    ) -> RuntimeRecoverableExpectedTypePlan {
        expected(
            "Array",
            RuntimeRecoverableExpectedTypeNode::Array {
                item: Box::new(item),
            },
        )
    }

    fn record_expected(
        fields: Vec<RuntimeRecoverableExpectedRecordFieldPlan>,
    ) -> RuntimeRecoverableExpectedTypePlan {
        expected(
            "record",
            RuntimeRecoverableExpectedTypeNode::Record {
                fields,
                boundary_record_kind: None,
            },
        )
    }

    fn field(
        name: &str,
        ty: RuntimeRecoverableExpectedTypePlan,
    ) -> RuntimeRecoverableExpectedRecordFieldPlan {
        RuntimeRecoverableExpectedRecordFieldPlan {
            name: name.to_string(),
            ty,
            required: true,
        }
    }

    fn optional_field(
        name: &str,
        ty: RuntimeRecoverableExpectedTypePlan,
    ) -> RuntimeRecoverableExpectedRecordFieldPlan {
        RuntimeRecoverableExpectedRecordFieldPlan {
            name: name.to_string(),
            ty,
            required: false,
        }
    }

    fn nullable_expected(
        inner: RuntimeRecoverableExpectedTypePlan,
    ) -> RuntimeRecoverableExpectedTypePlan {
        expected(
            "nullable",
            RuntimeRecoverableExpectedTypeNode::Nullable {
                inner: Box::new(inner),
            },
        )
    }

    fn union_expected(
        items: Vec<RuntimeRecoverableExpectedTypePlan>,
    ) -> RuntimeRecoverableExpectedTypePlan {
        expected("union", RuntimeRecoverableExpectedTypeNode::Union { items })
    }

    fn string_node(value: &str) -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::String,
            RecoverableState::String(value.to_string()),
        )
    }

    fn local_concrete_node() -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::NominalObject,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::LocalConcrete {
                owner: LocalConcreteOwner::Service,
                concrete_type_identity: "pkg.User".to_string(),
            },
            state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
                fields: vec![RecoverableField {
                    field_identity: "name".to_string(),
                    value: string_node("Ada"),
                }],
            }),
        }
    }

    fn custom_local_concrete_node() -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::NominalObject,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::LocalConcrete {
                owner: LocalConcreteOwner::Service,
                concrete_type_identity: "pkg.User".to_string(),
            },
            state: RecoverableState::NominalObject(NominalObjectState::Custom {
                durable_state: Box::new(string_node("durable")),
            }),
        }
    }

    fn interface_node() -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_node()),
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
                durable_state: Box::new(string_node("durable-handle")),
            }),
        }
    }

    fn native_adapter_artifact_plain_node(build_id: &str) -> RecoverableNode {
        RecoverableNode {
            value_kind: RecoverableValueKind::String,
            variant_identity: RecoverableVariantIdentity::None,
            code_identity: RecoverableCodeIdentity::NativeAdapter {
                adapter_identity: "std.StringAdapter".to_string(),
                adapter_schema_version: "1".to_string(),
                owner: NativeAdapterOwner::Artifact {
                    artifact_identity: SERVICE_ARTIFACT.to_string(),
                    build_id: build_id.to_string(),
                    package: None,
                },
                native_type_identity: "std.StringLike".to_string(),
            },
            state: RecoverableState::String("native-adapter".to_string()),
        }
    }

    const READER_INTERFACE: &str = "pkg.Reader";
    const READER_PROJECTION: &str = "projection:pkg.Reader:pkg.ReaderImpl";
    const READER_METHOD: &str = "method:pkg.Reader:read";
    const WRITER_INTERFACE: &str = "pkg.Writer";
    const WRITER_PROJECTION: &str = "projection:pkg.Writer:pkg.ReaderImpl";
    const READER_IMPL: &str = "pkg.ReaderImpl";
    const SERVICE_ARTIFACT: &str = "svc/account";

    fn any_reader_expected() -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan::any_interface(
            "any pkg.Reader",
            READER_INTERFACE,
            READER_PROJECTION,
        )
    }

    fn any_writer_expected() -> RuntimeRecoverableExpectedTypePlan {
        RuntimeRecoverableExpectedTypePlan::any_interface(
            "any pkg.Writer",
            WRITER_INTERFACE,
            WRITER_PROJECTION,
        )
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
                "operation:reader:read".to_string(),
            )],
        )
    }

    fn local_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
        let interface = InterfaceValue::new(
            READER_INTERFACE.to_string(),
            InterfaceCarrier::Local {
                concrete_type: READER_IMPL.to_string(),
                method_table: test_method_table(READER_INTERFACE, READER_PROJECTION),
                payload: RuntimeValue::String("Ada".to_string()),
            },
        );
        RuntimeValue::Heap(
            heap.alloc_interface(interface)
                .expect("local interface should allocate"),
        )
    }

    fn remote_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
        let interface = InterfaceValue::new(
            READER_INTERFACE.to_string(),
            InterfaceCarrier::Remote {
                dependency_ref: "svc.reader".to_string(),
                public_instance_key: "reader#42".to_string(),
                operations: test_remote_operation_table(),
            },
        );
        RuntimeValue::Heap(
            heap.alloc_interface(interface)
                .expect("remote interface should allocate"),
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
                    value: string_node(value),
                }],
            }),
        }
    }

    fn reader_interface_node(value: &str) -> RecoverableNode {
        RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_self_node(value)),
            }),
        )
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

    struct TestBehaviorHooks {
        encode_available: bool,
        restore_available: bool,
        conformance_available: bool,
        table_available: bool,
        self_node_has_local_concrete: bool,
        table_interface_identity: RefCell<String>,
        table_projection_identity: RefCell<String>,
        additional_conformances: Vec<(String, String)>,
        last_restore_decode_policy: RefCell<Option<RecoverableDecodePolicy>>,
        encode_calls: Cell<usize>,
        restore_calls: Cell<usize>,
        conformance_calls: Cell<usize>,
        table_calls: Cell<usize>,
        remote_table_calls: Cell<usize>,
    }

    impl Default for TestBehaviorHooks {
        fn default() -> Self {
            Self {
                encode_available: true,
                restore_available: true,
                conformance_available: true,
                table_available: true,
                self_node_has_local_concrete: true,
                table_interface_identity: RefCell::new(READER_INTERFACE.to_string()),
                table_projection_identity: RefCell::new(READER_PROJECTION.to_string()),
                additional_conformances: Vec::new(),
                last_restore_decode_policy: RefCell::new(None),
                encode_calls: Cell::new(0),
                restore_calls: Cell::new(0),
                conformance_calls: Cell::new(0),
                table_calls: Cell::new(0),
                remote_table_calls: Cell::new(0),
            }
        }
    }

    impl TestBehaviorHooks {
        fn without_local_concrete_identity() -> Self {
            Self {
                self_node_has_local_concrete: false,
                ..Self::default()
            }
        }

        fn without_restore_hook() -> Self {
            Self {
                restore_available: false,
                ..Self::default()
            }
        }

        fn without_conformance() -> Self {
            Self {
                conformance_available: false,
                ..Self::default()
            }
        }

        fn with_additional_conformance(interface: &str, projection: &str) -> Self {
            Self {
                additional_conformances: vec![(interface.to_string(), projection.to_string())],
                ..Self::default()
            }
        }

        fn with_wrong_method_table_interface() -> Self {
            Self {
                table_interface_identity: RefCell::new("pkg.Other".to_string()),
                ..Self::default()
            }
        }

        fn with_wrong_method_table_projection() -> Self {
            Self {
                table_projection_identity: RefCell::new("projection:pkg.Reader:Other".to_string()),
                ..Self::default()
            }
        }
    }

    impl RecoverableBehaviorHooks for TestBehaviorHooks {
        fn encode_local_interface_self(
            &self,
            request: RecoverableLocalInterfaceEncodeRequest<'_>,
            _heap: &RequestHeap,
        ) -> Result<Option<RecoverableEncodedLocalInterfaceSelf>> {
            self.encode_calls.set(self.encode_calls.get() + 1);
            if !self.encode_available {
                return Ok(None);
            }
            let value = match request.payload {
                RuntimeValue::String(value) => value.as_str(),
                RuntimeValue::Null => "null",
                _ => "unsupported",
            };
            let mut self_node = local_concrete_self_node(value);
            if !self.self_node_has_local_concrete {
                self_node.code_identity = RecoverableCodeIdentity::None;
            }
            Ok(Some(RecoverableEncodedLocalInterfaceSelf {
                method_projection_identity: request.method_table.id().to_string(),
                self_node,
            }))
        }

        fn restore_local_interface_self(
            &self,
            request: RecoverableLocalInterfaceRestoreRequest<'_>,
            _heap: &mut RequestHeap,
        ) -> Result<Option<RecoverableRestoredLocalInterfaceSelf>> {
            self.restore_calls.set(self.restore_calls.get() + 1);
            *self.last_restore_decode_policy.borrow_mut() = Some(request.decode_policy);
            if !self.restore_available {
                return Ok(None);
            }
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
            let primary = request.concrete_type_identity == READER_IMPL
                && request.interface_identity == READER_INTERFACE
                && request.method_projection_identity == READER_PROJECTION;
            let additional = self.additional_conformances.iter().any(
                |(interface_identity, method_projection_identity)| {
                    request.concrete_type_identity == READER_IMPL
                        && request.interface_identity == interface_identity
                        && request.method_projection_identity == method_projection_identity
                },
            );
            Ok(self.conformance_available
                && request.concrete_type_identity == READER_IMPL
                && (primary || additional))
        }

        fn rebuild_local_interface_method_table(
            &self,
            _request: RecoverableInterfaceMethodTableRequest<'_>,
        ) -> Result<Option<InterfaceMethodTable>> {
            self.table_calls.set(self.table_calls.get() + 1);
            if !self.table_available {
                return Ok(None);
            }
            Ok(Some(test_method_table(
                &self.table_interface_identity.borrow(),
                &self.table_projection_identity.borrow(),
            )))
        }

        fn rebuild_remote_interface_operation_table(
            &self,
            request: RecoverableRemoteInterfaceCarrierRequest<'_>,
        ) -> Result<Option<RemoteOperationTable>> {
            self.remote_table_calls
                .set(self.remote_table_calls.get() + 1);
            if request.carrier.dependency_ref == "svc.reader"
                && request.carrier.public_instance_key == "reader#42"
                && request.interface_identity == READER_INTERFACE
                && request.method_projection_identity == READER_PROJECTION
            {
                Ok(Some(test_remote_operation_table()))
            } else {
                Ok(None)
            }
        }
    }

    #[derive(Default)]
    struct TestArtifactStore {
        available: HashSet<(String, String)>,
    }

    impl TestArtifactStore {
        fn with_available(mut self, artifact_identity: &str, build_id: &str) -> Self {
            self.available
                .insert((artifact_identity.to_string(), build_id.to_string()));
            self
        }
    }

    impl RecoverableArtifactStore for TestArtifactStore {
        fn can_load_artifact(&self, artifact_identity: &str, build_id: &str) -> bool {
            self.available
                .contains(&(artifact_identity.to_string(), build_id.to_string()))
        }
    }

    #[derive(Default)]
    struct TestRootStore {
        fail: bool,
        roots: Vec<skiff_runtime_model::recoverable::RecoverableArtifactRetentionRoot>,
    }

    impl RecoverableArtifactRetentionRootStore for TestRootStore {
        fn persist_roots(
            &mut self,
            roots: &[skiff_runtime_model::recoverable::RecoverableArtifactRetentionRoot],
        ) -> std::result::Result<(), String> {
            if self.fail {
                return Err("root store unavailable".to_string());
            }
            self.roots.extend_from_slice(roots);
            Ok(())
        }
    }

    #[test]
    fn plain_record_array_map_roundtrips_through_recoverable_codec() {
        let context = recoverable_context();
        let expected = record_expected(vec![
            field("name", string_expected()),
            field("tags", array_expected(string_expected())),
            field("scores", map_expected(string_expected(), number_expected())),
        ]);
        let mut heap = RequestHeap::default();
        let tags = heap
            .alloc_array(vec![
                RuntimeValue::String("runtime".to_string()),
                RuntimeValue::String("codec".to_string()),
            ])
            .expect("tags allocate");
        let scores = heap
            .alloc_map(RuntimeMap::from([(
                RuntimeValueKey::string("math"),
                RuntimeValue::Number(98.5),
            )]))
            .expect("scores allocate");
        let value = RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("name".to_string(), RuntimeValue::String("Ada".to_string())),
                ("tags".to_string(), RuntimeValue::Heap(tags)),
                ("scores".to_string(), RuntimeValue::Heap(scores)),
            ])))
            .expect("record allocate"),
        );

        let bytes = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
            .expect("plain record should encode");
        let mut decode_heap = RequestHeap::default();
        let decoded =
            RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut decode_heap)
                .expect("plain record should decode");

        let RuntimeValue::Heap(record_handle) = decoded else {
            panic!("expected decoded record handle");
        };
        let HeapNode::Object(record) = decode_heap
            .get(record_handle)
            .expect("decoded record should resolve")
        else {
            panic!("expected decoded object");
        };
        assert_eq!(
            record.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
        let RuntimeValue::Heap(tags_handle) = record.fields().get("tags").expect("tags field")
        else {
            panic!("tags should be a heap array");
        };
        let HeapNode::Array(tags) = decode_heap.get(*tags_handle).expect("tags should resolve")
        else {
            panic!("expected tags array");
        };
        assert_eq!(tags.len(), 2);
        let RuntimeValue::Heap(scores_handle) =
            record.fields().get("scores").expect("scores field")
        else {
            panic!("scores should be a heap map");
        };
        let HeapNode::Map(scores) = decode_heap
            .get(*scores_handle)
            .expect("scores should resolve")
        else {
            panic!("expected scores map");
        };
        assert_eq!(
            scores.get(&RuntimeValueKey::string("math")),
            Some(&RuntimeValue::Number(98.5))
        );
        let reencoded = RecoverableBoundaryCodec::encode(
            &RuntimeValue::Heap(record_handle),
            &expected,
            &context,
            &decode_heap,
        )
        .expect("decoded record should re-encode");
        assert_eq!(bytes, reencoded);
    }

    #[test]
    fn bytes_date_and_number_edges_follow_canonical_dto_rules() {
        let context = recoverable_context();

        let mut heap = RequestHeap::default();
        let bytes_value = RuntimeValue::Heap(
            heap.alloc_bytes(vec![0, 1, 2, 255])
                .expect("bytes should allocate"),
        );
        let encoded =
            RecoverableBoundaryCodec::encode(&bytes_value, &bytes_expected(), &context, &heap)
                .expect("bytes should encode");
        let mut decoded_heap = RequestHeap::default();
        let decoded = RecoverableBoundaryCodec::decode(
            &encoded,
            &bytes_expected(),
            &context,
            &mut decoded_heap,
        )
        .expect("bytes should decode");
        let RuntimeValue::Heap(bytes_handle) = decoded else {
            panic!("expected bytes handle");
        };
        let HeapNode::Bytes(decoded_bytes) = decoded_heap.get(bytes_handle).expect("bytes resolve")
        else {
            panic!("expected bytes");
        };
        assert_eq!(decoded_bytes.as_slice(), &[0, 1, 2, 255]);

        let encoded = RecoverableBoundaryCodec::encode(
            &RuntimeValue::Number(-0.0),
            &number_expected(),
            &context,
            &RequestHeap::default(),
        )
        .expect("negative zero should encode");
        let decoded = RecoverableBoundaryCodec::decode(
            &encoded,
            &number_expected(),
            &context,
            &mut RequestHeap::default(),
        )
        .expect("negative zero should decode");
        let RuntimeValue::Number(decoded_number) = decoded else {
            panic!("expected number");
        };
        assert_eq!(decoded_number.to_bits(), (-0.0f64).to_bits());

        let date = RuntimeValue::Date(1_609_459_200_000);
        let encoded = RecoverableBoundaryCodec::encode(
            &date,
            &date_expected(),
            &context,
            &RequestHeap::default(),
        )
        .expect("valid Date should encode");
        let decoded = RecoverableBoundaryCodec::decode(
            &encoded,
            &date_expected(),
            &context,
            &mut RequestHeap::default(),
        )
        .expect("valid Date should decode");
        assert_eq!(decoded, date);

        let error = RecoverableBoundaryCodec::encode(
            &RuntimeValue::Number(f64::INFINITY),
            &number_expected(),
            &context,
            &RequestHeap::default(),
        )
        .expect_err("non-finite numbers must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(error.code(), RecoverableBoundaryErrorCode::StateInvalid);

        let error = RecoverableBoundaryCodec::encode(
            &RuntimeValue::Date(253_402_300_800_000),
            &date_expected(),
            &context,
            &RequestHeap::default(),
        )
        .expect_err("out-of-range Date must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(error.code(), RecoverableBoundaryErrorCode::StateInvalid);
    }

    #[test]
    fn expected_type_mismatch_fails_before_heap_decode() {
        let context = recoverable_context();
        let heap = RequestHeap::default();
        let bytes = RecoverableBoundaryCodec::encode(
            &RuntimeValue::String("Ada".to_string()),
            &string_expected(),
            &context,
            &heap,
        )
        .expect("string should encode");

        let mut decode_heap = RequestHeap::default();
        let error =
            RecoverableBoundaryCodec::decode(&bytes, &bool_expected(), &context, &mut decode_heap)
                .expect_err("decode precheck must reject expected type mismatch");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert_eq!(decode_heap.len(), 0);
        assert_eq!(
            error
                .detail()
                .and_then(|detail| detail.get("nodePath"))
                .and_then(|path| path.as_str()),
            Some("$.root")
        );
    }

    #[test]
    fn durable_db_policy_ignores_unknown_record_fields_and_materializes_missing_nullable_fields() {
        let context = recoverable_context();
        let expected = record_expected(vec![
            field("name", string_expected()),
            optional_field("nickname", nullable_expected(string_expected())),
        ]);
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Record,
            RecoverableState::Record(vec![
                RecoverableField {
                    field_identity: "name".to_string(),
                    value: string_node("Ada"),
                },
                RecoverableField {
                    field_identity: "historical".to_string(),
                    value: string_node("ignored"),
                },
            ]),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("record envelope should encode");

        let strict_error = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect_err("strict decode must reject unknown record fields");
        let RuntimeError::Recoverable(strict_error) = strict_error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            strict_error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );

        let mut heap = RequestHeap::default();
        let decoded = RecoverableBoundaryCodec::decode_with_policy(
            &bytes,
            &expected,
            &context,
            &mut heap,
            RecoverableDecodePolicy::durable_db(),
        )
        .expect("durable DB policy should ignore unknown fields and materialize nullable fields");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("expected object handle");
        };
        let HeapNode::Object(object) = heap.get(handle).expect("decoded object should resolve")
        else {
            panic!("expected decoded object");
        };
        assert_eq!(
            object.fields().get("name"),
            Some(&RuntimeValue::String("Ada".to_string()))
        );
        assert_eq!(object.fields().get("nickname"), Some(&RuntimeValue::Null));
        assert!(!object.fields().contains_key("historical"));
    }

    #[test]
    fn durable_db_policy_still_rejects_missing_required_record_fields() {
        let context = recoverable_context();
        let expected = record_expected(vec![field("name", string_expected())]);
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Record,
            RecoverableState::Record(Vec::new()),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("record envelope should encode");

        let mut heap = RequestHeap::default();
        let error = RecoverableBoundaryCodec::decode_with_policy(
            &bytes,
            &expected,
            &context,
            &mut heap,
            RecoverableDecodePolicy::durable_db(),
        )
        .expect_err("missing required fields must fail under durable DB policy");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert_eq!(heap.len(), 0);
    }

    #[test]
    fn union_expected_multi_match_fails_closed() {
        let context = recoverable_context();
        let expected = union_expected(vec![
            string_expected(),
            expected("json", RuntimeRecoverableExpectedTypeNode::Json),
        ]);
        let envelope = RecoverableEnvelope::new(string_node("Ada"));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("string envelope should encode");

        let error = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect_err("union multi-match must fail closed");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert!(error.to_string().contains("multiple union branches"));
    }

    #[test]
    fn union_expected_any_interface_single_conformance_selects_matching_branch() {
        let context = recoverable_context();
        let expected = union_expected(vec![any_writer_expected(), any_reader_expected()]);
        let bytes = RecoverableEnvelope::new(reader_interface_node("Ada"))
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode");
        let hooks = TestBehaviorHooks::default();
        let mut heap = RequestHeap::default();

        let decoded = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes, &expected, &context, &mut heap, &hooks,
        )
        .expect("single conforming any-interface branch should decode");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded interface should be a heap value");
        };
        let HeapNode::Interface(interface) = heap.get(handle).expect("interface resolves") else {
            panic!("expected decoded InterfaceValue");
        };
        assert_eq!(interface.interface(), READER_INTERFACE);
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.table_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 5);
    }

    #[test]
    fn union_expected_any_interface_multi_conformance_fails_closed_before_restore() {
        let context = recoverable_context();
        let expected = union_expected(vec![any_writer_expected(), any_reader_expected()]);
        let bytes = RecoverableEnvelope::new(reader_interface_node("Ada"))
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode");
        let hooks =
            TestBehaviorHooks::with_additional_conformance(WRITER_INTERFACE, WRITER_PROJECTION);

        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
            &hooks,
        )
        .expect_err("multiple conforming any-interface branches must fail closed");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert!(error.to_string().contains("multiple union branches"));
        assert_eq!(hooks.restore_calls.get(), 0);
        assert_eq!(hooks.table_calls.get(), 0);
    }

    #[test]
    fn unresolved_expected_does_not_decode_behavior_bearing_interface_value() {
        let context = recoverable_context();
        let hooks = TestBehaviorHooks::default();
        let bytes = RecoverableEnvelope::new(interface_node())
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode");

        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected_plan(),
            &context,
            &mut RequestHeap::default(),
            &hooks,
        )
        .expect_err("unresolved expected must not decode behavior-bearing InterfaceValue");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert_eq!(hooks.restore_calls.get(), 0);
    }

    #[test]
    fn untrusted_context_rejects_behavior_envelope_before_restore() {
        let envelope = RecoverableEnvelope::new(local_concrete_node());
        let context = external_recoverable_context();
        let expected = expected_plan();
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("behavior envelope should encode canonically");

        let error = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect_err("external decode must reject behavior before restore");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
        );
        assert_eq!(
            error
                .detail()
                .and_then(|detail| detail.get("nodePath"))
                .and_then(|path| path.as_str()),
            Some("$.root")
        );
    }

    #[test]
    fn decode_rejects_legacy_runtime_binary_payload_without_fallback() {
        let runtime_plan = runtime_string_plan();
        let expected =
            RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                &runtime_plan,
            );
        let context = recoverable_context();
        let heap = RequestHeap::default();
        let bytes = encode_payload_plan(
            &RuntimeValue::String("Ada".to_string()),
            &runtime_plan,
            &PayloadBoundary::runtime_internal(),
            &heap,
        )
        .expect("legacy runtime binary payload should still encode");

        let mut decode_heap = RequestHeap::default();
        let error = RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut decode_heap)
            .expect_err("recoverable decode must not accept legacy runtime binary bytes");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(error.code(), RecoverableBoundaryErrorCode::StateInvalid);
        assert_eq!(decode_heap.len(), 0);
    }

    #[test]
    fn legacy_runtime_binary_codec_remains_available() {
        let runtime_plan = runtime_string_plan();
        let heap = RequestHeap::default();
        let bytes = encode_payload_plan(
            &RuntimeValue::String("Ada".to_string()),
            &runtime_plan,
            &PayloadBoundary::runtime_internal(),
            &heap,
        )
        .expect("legacy runtime binary payload should encode");

        let mut decode_heap = RequestHeap::default();
        let value = decode_payload_plan(
            &bytes,
            &runtime_plan,
            &PayloadBoundary::runtime_internal(),
            &mut decode_heap,
        )
        .expect("legacy runtime binary payload should decode");

        assert_eq!(value, RuntimeValue::String("Ada".to_string()));
    }

    #[test]
    fn interface_value_encode_and_decode_remain_p4_fail_closed() {
        let context = recoverable_context();
        let expected = expected_plan();
        let mut heap = RequestHeap::default();
        let interface = InterfaceValue::new(
            "pkg.Reader".to_string(),
            InterfaceCarrier::Local {
                concrete_type: "pkg.ReaderImpl".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:reader".to_string(),
                    "pkg.Reader".to_string(),
                    Vec::new(),
                ),
                payload: RuntimeValue::Null,
            },
        );
        let value = RuntimeValue::Heap(
            heap.alloc_interface(interface)
                .expect("interface should allocate"),
        );

        let error = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
            .expect_err("P3 must not encode any-I wrappers");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UnsupportedEncode
        );

        let error = RecoverableBoundaryCodec::encode(
            &value,
            &expected,
            &external_recoverable_context(),
            &heap,
        )
        .expect_err("untrusted interface encode must fail before P4");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UntrustedBehaviorPayload
        );

        let bytes = RecoverableEnvelope::new(interface_node())
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode canonically");
        let error = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect_err("P3 must not decode any-I wrappers");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UnsupportedDecode
        );
    }

    #[test]
    fn behavior_api_roundtrips_owner_internal_local_interface_value() {
        let context = recoverable_context();
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = local_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect("local interface should encode through behavior hook");
        assert_eq!(hooks.encode_calls.get(), 1);
        assert!(matches!(
            envelope.root.code_identity,
            RecoverableCodeIdentity::None
        ));
        let RecoverableState::InterfaceValue(state) = &envelope.root.state else {
            panic!("expected InterfaceValue root");
        };
        let InterfaceValueState::Local { self_node } = state else {
            panic!("encoded local interface should use Local state");
        };
        let RecoverableCodeIdentity::LocalConcrete {
            owner,
            concrete_type_identity,
        } = &self_node.code_identity
        else {
            panic!("self_node should carry LocalConcrete");
        };
        assert_eq!(owner, &LocalConcreteOwner::Service);
        assert_eq!(concrete_type_identity, READER_IMPL);

        let bytes = RecoverableBoundaryCodec::encode_envelope_canonical(
            &envelope,
            &RecoverableValidationLimits::default(),
            &expected,
            &context,
        )
        .expect("behavior envelope should canonical encode");
        let mut decode_heap = RequestHeap::default();
        let decoded = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &hooks,
        )
        .expect("local interface should decode through behavior hook");
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 1);
        assert_eq!(hooks.table_calls.get(), 1);
        assert_eq!(
            *hooks.last_restore_decode_policy.borrow(),
            Some(RecoverableDecodePolicy::strict())
        );

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded interface should be a heap value");
        };
        let HeapNode::Interface(interface) = decode_heap.get(handle).expect("interface resolves")
        else {
            panic!("expected decoded InterfaceValue");
        };
        assert_eq!(interface.interface(), READER_INTERFACE);
        let InterfaceCarrier::Local {
            concrete_type,
            method_table,
            payload,
        } = interface.carrier()
        else {
            panic!("decoded interface should use local carrier");
        };
        assert_eq!(concrete_type, READER_IMPL);
        assert_eq!(method_table.id(), READER_PROJECTION);
        assert_eq!(method_table.interface_abi_id(), READER_INTERFACE);
        assert_eq!(method_table.slots()[0].method_abi_id(), READER_METHOD);
        assert_eq!(payload, &RuntimeValue::String("Ada".to_string()));

        let durable_policy_hooks = TestBehaviorHooks::default();
        let mut decode_heap = RequestHeap::default();
        RecoverableBoundaryCodec::decode_with_behavior_and_policy(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &durable_policy_hooks,
            RecoverableDecodePolicy::durable_db(),
        )
        .expect("policy-aware behavior decode should succeed");
        assert_eq!(
            *durable_policy_hooks.last_restore_decode_policy.borrow(),
            Some(RecoverableDecodePolicy::durable_db())
        );
    }

    #[test]
    fn behavior_api_roundtrips_owner_internal_remote_interface_value() {
        let context = recoverable_context();
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = remote_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect("remote interface should encode through owner-internal behavior API");
        assert_eq!(hooks.encode_calls.get(), 0);
        assert!(matches!(
            envelope.root.code_identity,
            RecoverableCodeIdentity::None
        ));
        let RecoverableState::InterfaceValue(InterfaceValueState::Remote { carrier }) =
            &envelope.root.state
        else {
            panic!("encoded remote interface should use Remote state");
        };
        assert_eq!(carrier.dependency_ref, "svc.reader");
        assert_eq!(carrier.public_instance_key, "reader#42");
        assert_eq!(carrier.operations.id, "remote:reader");
        assert_eq!(carrier.operations.interface_abi_id, READER_INTERFACE);
        assert_eq!(carrier.operations.slots[0].slot, 0);
        assert_eq!(carrier.operations.slots[0].method_abi_id, READER_METHOD);
        assert_eq!(
            carrier.operations.slots[0].operation_abi_id,
            "operation:reader:read"
        );

        let bytes = RecoverableBoundaryCodec::encode_envelope_canonical(
            &envelope,
            &RecoverableValidationLimits::default(),
            &expected,
            &context,
        )
        .expect("remote behavior envelope should canonical encode");
        let mut decode_heap = RequestHeap::default();
        let decoded = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &hooks,
        )
        .expect("remote interface should decode through behavior hook");

        let RuntimeValue::Heap(handle) = decoded else {
            panic!("decoded interface should be a heap value");
        };
        let HeapNode::Interface(interface) = decode_heap.get(handle).expect("interface resolves")
        else {
            panic!("expected decoded InterfaceValue");
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
        assert_eq!(dependency_ref, "svc.reader");
        assert_eq!(public_instance_key, "reader#42");
        assert_eq!(operations.id(), "remote:reader");
        assert_eq!(operations.interface_abi_id(), READER_INTERFACE);
        assert_eq!(operations.slots()[0].slot(), 0);
        assert_eq!(operations.slots()[0].method_abi_id(), READER_METHOD);
        assert_eq!(
            operations.slots()[0].operation_abi_id(),
            "operation:reader:read"
        );
        assert_eq!(hooks.restore_calls.get(), 0);
        assert_eq!(hooks.remote_table_calls.get(), 2);
    }

    #[test]
    fn behavior_api_remote_encode_rejects_unresolved_expected_before_hook() {
        let context = recoverable_context();
        let expected = expected_plan();
        let mut heap = RequestHeap::default();
        let value = remote_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect_err("remote encode must reject unresolved expected type");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ExpectedTypeMismatch
        );
        assert!(error.message().contains("recoverable encode"));
        assert_eq!(hooks.remote_table_calls.get(), 0);
    }

    #[test]
    fn behavior_api_remote_encode_selects_unique_union_any_interface() {
        let context = recoverable_context();
        let expected = union_expected(vec![any_writer_expected(), any_reader_expected()]);
        let mut heap = RequestHeap::default();
        let value = remote_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let envelope = RecoverableBoundaryCodec::encode_envelope_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect("single matching union any-interface branch should encode");

        let RecoverableState::InterfaceValue(InterfaceValueState::Remote { carrier }) =
            &envelope.root.state
        else {
            panic!("encoded remote interface should use Remote state");
        };
        assert_eq!(carrier.operations.interface_abi_id, READER_INTERFACE);
        assert_eq!(hooks.remote_table_calls.get(), 3);
    }

    #[test]
    fn behavior_api_remote_encode_requires_rebuild_hook() {
        let context = recoverable_context();
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = remote_interface_runtime_value(&mut heap);
        let missing_hook = FailClosedRecoverableBehaviorHooks;

        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value,
            &expected,
            &context,
            &heap,
            &missing_hook,
        )
        .expect_err("remote encode must fail when linked program cannot rebuild the carrier");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::RemoteCarrierNotPersistable
        );
    }

    #[test]
    fn behavior_api_missing_hook_or_local_concrete_identity_fails_closed() {
        let context = recoverable_context();
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = local_interface_runtime_value(&mut heap);

        let missing_hook = FailClosedRecoverableBehaviorHooks;
        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value,
            &expected,
            &context,
            &heap,
            &missing_hook,
        )
        .expect_err("missing encode hook must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CodeIdentityMissing
        );

        let missing_identity = TestBehaviorHooks::without_local_concrete_identity();
        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value,
            &expected,
            &context,
            &heap,
            &missing_identity,
        )
        .expect_err("missing LocalConcrete identity must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CodeIdentityMissing
        );

        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_self_node("Ada")),
            }),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode canonically");
        let missing_restore = TestBehaviorHooks::without_restore_hook();
        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &any_reader_expected(),
            &context,
            &mut RequestHeap::default(),
            &missing_restore,
        )
        .expect_err("missing restore hook must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::UnsupportedDecode
        );
    }

    #[test]
    fn behavior_api_expected_interface_or_projection_comes_from_expected_plan() {
        let context = recoverable_context();
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_self_node("Ada")),
            }),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode canonically");

        let wrong_interface = RuntimeRecoverableExpectedTypePlan::any_interface(
            "any pkg.Other",
            "pkg.Other",
            READER_PROJECTION,
        );
        let hooks = TestBehaviorHooks::default();
        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &wrong_interface,
            &context,
            &mut RequestHeap::default(),
            &hooks,
        )
        .expect_err("wrong expected interface identity must fail closed");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::InterfaceConformanceMissing
        );
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 1);

        let wrong_projection = RuntimeRecoverableExpectedTypePlan::any_interface(
            "any pkg.Reader",
            READER_INTERFACE,
            "projection:pkg.Reader:Other",
        );
        let hooks = TestBehaviorHooks::default();
        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &wrong_projection,
            &context,
            &mut RequestHeap::default(),
            &hooks,
        )
        .expect_err("wrong expected method projection must fail closed");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::InterfaceConformanceMissing
        );
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 1);
    }

    #[test]
    fn behavior_api_conformance_or_method_table_mismatch_fails_before_returning_value() {
        let context = recoverable_context();
        let expected = any_reader_expected();
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::InterfaceValue,
            RecoverableState::InterfaceValue(InterfaceValueState::Local {
                self_node: Box::new(local_concrete_self_node("Ada")),
            }),
        ));
        let bytes = envelope
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("interface envelope should encode canonically");

        let hooks = TestBehaviorHooks::without_conformance();
        let mut decode_heap = RequestHeap::default();
        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &hooks,
        )
        .expect_err("missing conformance must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::InterfaceConformanceMissing
        );
        assert_eq!(hooks.restore_calls.get(), 1);
        assert_eq!(hooks.conformance_calls.get(), 1);
        assert_eq!(hooks.table_calls.get(), 0);
        assert_eq!(decode_heap.len(), 0);

        let hooks = TestBehaviorHooks::with_wrong_method_table_interface();
        let mut decode_heap = RequestHeap::default();
        let error = RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &hooks,
        )
        .expect_err("wrong rebuilt method table must fail");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::InterfaceConformanceMissing
        );
        assert_eq!(hooks.table_calls.get(), 1);
        assert_eq!(decode_heap.len(), 0);

        let hooks = TestBehaviorHooks::with_wrong_method_table_projection();
        let mut decode_heap = RequestHeap::default();
        RecoverableBoundaryCodec::decode_with_behavior(
            &bytes,
            &expected,
            &context,
            &mut decode_heap,
            &hooks,
        )
        .expect("runtime method table id is not the durable projection identity");
        assert_eq!(hooks.table_calls.get(), 1);
        assert_ne!(decode_heap.len(), 0);
    }

    #[test]
    fn behavior_api_untrusted_nested_behavior_rejects_before_hook() {
        let expected = record_expected(vec![field("value", any_reader_expected())]);
        let contexts = [
            RuntimeRecoverableBoundaryContext::new(
                RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
                RuntimeRecoverableTrustBoundary::CrossService,
                RuntimeRecoverableStorageLane::RecoverableEnvelope,
            )
            .with_explicit_recoverable_slot(),
            RuntimeRecoverableBoundaryContext::new(
                RuntimeRecoverableBoundaryKind::PublicApiPayload,
                RuntimeRecoverableTrustBoundary::ExternalUntrusted,
                RuntimeRecoverableStorageLane::RecoverableEnvelope,
            )
            .with_explicit_recoverable_slot(),
        ];

        for context in contexts {
            for node in [
                record_node("value", interface_node()),
                record_node("value", local_concrete_node()),
            ] {
                let hooks = TestBehaviorHooks::default();
                let bytes = RecoverableEnvelope::new(node)
                    .to_canonical_bytes(&RecoverableValidationLimits::default())
                    .expect("nested behavior envelope should encode canonically");
                let error = RecoverableBoundaryCodec::decode_with_behavior(
                    &bytes,
                    &expected,
                    &context,
                    &mut RequestHeap::default(),
                    &hooks,
                )
                .expect_err("untrusted behavior must reject before hook");
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
    fn behavior_api_cross_service_local_interface_encode_rejects_before_hook() {
        let context = RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
            RuntimeRecoverableTrustBoundary::CrossService,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot();
        let expected = any_reader_expected();
        let mut heap = RequestHeap::default();
        let value = local_interface_runtime_value(&mut heap);
        let hooks = TestBehaviorHooks::default();

        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect_err("cross-service local interface cannot be encoded");

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
    fn nominal_custom_and_native_restore_fail_closed_without_hooks() {
        let context = recoverable_context();
        let expected = expected_plan();

        for node in [local_concrete_node(), custom_local_concrete_node()] {
            let bytes = RecoverableEnvelope::new(node)
                .to_canonical_bytes(&RecoverableValidationLimits::default())
                .expect("nominal envelope should encode canonically");
            let error = RecoverableBoundaryCodec::decode(
                &bytes,
                &expected,
                &context,
                &mut RequestHeap::default(),
            )
            .expect_err("nominal restore without hook must fail closed");
            let RuntimeError::Recoverable(error) = error else {
                panic!("expected recoverable error");
            };
            assert_eq!(
                error.code(),
                RecoverableBoundaryErrorCode::UnsupportedDecode
            );
        }

        let bytes = RecoverableEnvelope::new(native_handle_node())
            .to_canonical_bytes(&RecoverableValidationLimits::default())
            .expect("native envelope should encode canonically");
        let error = RecoverableBoundaryCodec::decode(
            &bytes,
            &expected,
            &context,
            &mut RequestHeap::default(),
        )
        .expect_err("native restore without adapter must fail closed");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::NativeMissingAdapter
        );
    }

    #[test]
    fn canonical_envelope_helpers_roundtrip_behavior_dtos() {
        let envelope = RecoverableEnvelope::new(interface_node());
        let context = recoverable_context_with_service();
        let expected = expected_plan();
        let limits = RecoverableValidationLimits::default();

        let bytes = RecoverableBoundaryCodec::encode_envelope_canonical(
            &envelope, &limits, &expected, &context,
        )
        .expect("envelope canonical encode should succeed");
        let decoded = RecoverableBoundaryCodec::decode_envelope_canonical(
            &bytes, &limits, &expected, &context,
        )
        .expect("envelope canonical decode should succeed");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn unavailable_artifact_fails_closed_with_required_diagnostics() {
        let envelope =
            RecoverableEnvelope::new(native_adapter_artifact_plain_node("missing-build"));
        let store = TestArtifactStore::default();
        let context = recoverable_context_with_service();
        let expected = expected_plan();

        let error = RecoverableBoundaryCodec::verify_artifact_availability(
            &envelope, &store, &expected, &context,
        )
        .expect_err("missing artifact must fail");

        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ArtifactUnavailable
        );
        let detail = error.detail().expect("artifact detail");
        assert_eq!(detail.get("serviceId"), Some(&json!("skiff.run/account")));
        assert_eq!(detail.get("artifactIdentity"), Some(&json!("svc/account")));
        assert_eq!(detail.get("buildId"), Some(&json!("missing-build")));
        assert_eq!(detail.get("nodePath"), Some(&json!("$.root")));
        assert_eq!(
            detail.get("boundaryKind"),
            Some(&json!("runtimeBinaryPayload"))
        );
    }

    #[test]
    fn available_artifacts_produce_retention_roots_and_root_write_failure_fails_closed() {
        let envelope = RecoverableEnvelope::new(RecoverableNode::plain(
            RecoverableValueKind::Array,
            RecoverableState::Array(vec![
                native_adapter_artifact_plain_node("build-a"),
                native_adapter_artifact_plain_node("build-b"),
            ]),
        ));
        let store = TestArtifactStore::default()
            .with_available("svc/account", "build-a")
            .with_available("svc/account", "build-b");
        let context = recoverable_context_with_service();
        let expected = expected_plan();
        let refs = RecoverableBoundaryCodec::verify_artifact_availability(
            &envelope, &store, &expected, &context,
        )
        .expect("artifacts should be available");
        assert_eq!(refs.len(), 2);

        let mut root_store = TestRootStore::default();
        let roots = RecoverableBoundaryCodec::persist_artifact_retention_roots(
            &refs,
            &mut root_store,
            &expected,
            &context,
            Some(1_609_459_200_000),
        )
        .expect("root write should succeed");
        assert_eq!(roots.len(), 2);
        assert_eq!(root_store.roots, roots);
        assert_eq!(roots[0].service_id, "skiff.run/account");
        assert_eq!(
            roots[0].boundary_kind,
            RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload
        );

        let mut failing_root_store = TestRootStore {
            fail: true,
            roots: Vec::new(),
        };
        let error = RecoverableBoundaryCodec::persist_artifact_retention_roots(
            &refs,
            &mut failing_root_store,
            &expected,
            &context,
            None,
        )
        .expect_err("root write failure must fail closed");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::ArtifactUnavailable
        );
        assert_eq!(
            error
                .detail()
                .and_then(|detail| detail.get("reason"))
                .and_then(|reason| reason.as_str()),
            Some("root store unavailable")
        );
    }
}
