use serde_json::json;
use skiff_runtime_model::addr::ExecutableAddr;
use skiff_runtime_model::recoverable::{
    InterfaceValueState, LocalConcreteOwner, NativeAdapterOwner, NativeHandleState,
    NominalObjectState, RecoverableCodeIdentity, RecoverableEnvelope, RecoverableField,
    RecoverableNode, RecoverableState, RecoverableValidationLimits, RecoverableValueKind,
    RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
    RuntimeRecoverableExpectedRecordFieldPlan, RuntimeRecoverableExpectedTypeNode,
    RuntimeRecoverableExpectedTypePlan, RuntimeRecoverableServiceRef,
    RuntimeRecoverableStorageLane, RuntimeRecoverableTrustBoundary,
};
use skiff_runtime_model::runtime_value::{
    CallbackCapabilityCarrier, InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable,
    InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue, RuntimeMap, RuntimeObject,
    RuntimeObjectFields, RuntimeValueKey,
};
use skiff_runtime_model::type_plan::{std_task_cancel_result_plan, std_task_status_plan};
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

fn json_object_expected() -> RuntimeRecoverableExpectedTypePlan {
    expected("JsonObject", RuntimeRecoverableExpectedTypeNode::JsonObject)
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

fn array_expected(item: RuntimeRecoverableExpectedTypePlan) -> RuntimeRecoverableExpectedTypePlan {
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
const READER_RUNTIME_IMPL: &str = "runtime:pkg.ReaderImpl";
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

fn test_method_table(interface_identity: &str, projection_identity: &str) -> InterfaceMethodTable {
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

fn local_interface_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    let interface = InterfaceValue::new(
        READER_INTERFACE.to_string(),
        InterfaceCarrier::Local {
            concrete_type: READER_RUNTIME_IMPL.to_string(),
            method_table: test_method_table(READER_INTERFACE, READER_PROJECTION),
            payload: RuntimeValue::String("Ada".to_string()),
        },
    );
    RuntimeValue::Heap(
        heap.alloc_interface(interface)
            .expect("local interface should allocate"),
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
        if request.concrete_type != READER_RUNTIME_IMPL {
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
            runtime_concrete_type_identity: READER_RUNTIME_IMPL.to_string(),
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
fn task_ref_roundtrips_through_recoverable_codec() {
    let context = recoverable_context();
    let task_ref_expected = expected("taskRef", RuntimeRecoverableExpectedTypeNode::TaskRef);
    let heap = RequestHeap::default();
    let canonical = "skiff-task-v1:b3duZXI.dGFzay0x";

    let bytes = RecoverableBoundaryCodec::encode(
        &RuntimeValue::String(canonical.to_string()),
        &task_ref_expected,
        &context,
        &heap,
    )
    .expect("canonical taskRef should encode through the recoverable boundary");
    let mut decode_heap = RequestHeap::default();
    let decoded =
        RecoverableBoundaryCodec::decode(&bytes, &task_ref_expected, &context, &mut decode_heap)
            .expect("canonical taskRef should decode through the recoverable boundary");
    assert_eq!(decoded, RuntimeValue::String(canonical.to_string()));
}

#[test]
fn task_status_and_cancel_result_roundtrip_through_recoverable_codec() {
    let context = recoverable_context();
    for (plan, kind) in [
        (std_task_status_plan(), "succeeded"),
        (std_task_status_plan(), "platformFailed"),
        (std_task_cancel_result_plan(), "canceled"),
        (std_task_cancel_result_plan(), "alreadyTerminal"),
    ] {
        let expected =
            RuntimeRecoverableExpectedTypePlan::from_runtime_type_plan_shape_only_for_diagnostics(
                &plan,
            );
        let mut heap = RequestHeap::default();
        let value = RuntimeValue::Heap(
            heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "kind".to_string(),
                RuntimeValue::String(kind.to_string()),
            )])))
            .expect("task control union record must allocate"),
        );
        let bytes = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
            .unwrap_or_else(|error| panic!("{kind}: {error}"));
        let mut decode_heap = RequestHeap::default();
        let decoded =
            RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut decode_heap)
                .unwrap_or_else(|error| panic!("{kind} decode: {error}"));
        let RuntimeValue::Heap(handle) = decoded else {
            panic!("{kind}: decoded task control union must be a heap record");
        };
        let HeapNode::Object(object) = decode_heap
            .get(handle)
            .expect("decoded task control union should resolve")
        else {
            panic!("{kind}: decoded task control union must be an object");
        };
        assert_eq!(
            object.fields().get("kind"),
            Some(&RuntimeValue::String(kind.to_string())),
            "{kind}: kind must roundtrip"
        );
    }
}

#[test]
fn task_ref_rejects_plain_strings_and_malformed_refs() {
    let context = recoverable_context();
    let task_ref_expected = expected("taskRef", RuntimeRecoverableExpectedTypeNode::TaskRef);
    let heap = RequestHeap::default();

    for malformed in [
        "not-a-task-ref",
        "skiff-task-v1:",
        "skiff-task-v1:b3duZXI.",
        "skiff-task-v1:.dGFzay0x",
        "skiff-task-v1:!!!!.dGFzay0x",
        "skiff-task-v1:b3duZXI.bm90IGJhc2U2NA==",
    ] {
        let error = RecoverableBoundaryCodec::encode(
            &RuntimeValue::String(malformed.to_string()),
            &task_ref_expected,
            &context,
            &heap,
        )
        .expect_err("non-canonical taskRef must fail closed on encode");
        assert!(
            error.to_string().contains("taskRef"),
            "{malformed}: {error}"
        );
    }

    // A plain string encoded against a String plan must also fail closed when
    // decoded against the TaskRef plan (decode-side precheck).
    let plain_bytes = RecoverableBoundaryCodec::encode(
        &RuntimeValue::String("plain".to_string()),
        &string_expected(),
        &context,
        &heap,
    )
    .expect("plain string should encode against a string plan");
    let mut decode_heap = RequestHeap::default();
    let error = RecoverableBoundaryCodec::decode(
        &plain_bytes,
        &task_ref_expected,
        &context,
        &mut decode_heap,
    )
    .expect_err("plain string must fail closed on TaskRef decode");
    assert!(error.to_string().contains("taskRef"));
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
    let decoded = RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut decode_heap)
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
    let RuntimeValue::Heap(tags_handle) = record.fields().get("tags").expect("tags field") else {
        panic!("tags should be a heap array");
    };
    let HeapNode::Array(tags) = decode_heap.get(*tags_handle).expect("tags should resolve") else {
        panic!("expected tags array");
    };
    assert_eq!(tags.len(), 2);
    let RuntimeValue::Heap(scores_handle) = record.fields().get("scores").expect("scores field")
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
fn json_object_decodes_json_child_values() {
    let context = recoverable_context();
    let expected = json_object_expected();
    let mut heap = RequestHeap::default();
    let nested = heap
        .alloc_map(RuntimeMap::from([
            (RuntimeValueKey::string("enabled"), RuntimeValue::Bool(true)),
            (
                RuntimeValueKey::string("name"),
                RuntimeValue::String("inner".to_string()),
            ),
        ]))
        .expect("nested JsonObject map should allocate");
    let value = RuntimeValue::Heap(
        heap.alloc_map(RuntimeMap::from([
            (
                RuntimeValueKey::string("providerKey"),
                RuntimeValue::String("test".to_string()),
            ),
            (RuntimeValueKey::string("route"), RuntimeValue::Heap(nested)),
        ]))
        .expect("JsonObject map should allocate"),
    );
    let bytes = RecoverableBoundaryCodec::encode(&value, &expected, &context, &heap)
        .expect("JsonObject should encode");
    let mut decoded_heap = RequestHeap::default();
    let decoded = RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut decoded_heap)
        .expect("JsonObject should decode scalar child values as Json");

    let RuntimeValue::Heap(handle) = decoded else {
        panic!("JsonObject should decode to a heap map");
    };
    let HeapNode::Map(decoded) = decoded_heap
        .get(handle)
        .expect("decoded map should resolve")
    else {
        panic!("expected decoded JsonObject map");
    };
    assert_eq!(
        decoded.get(&RuntimeValueKey::string("providerKey")),
        Some(&RuntimeValue::String("test".to_string()))
    );
    let RuntimeValue::Heap(route_handle) = decoded
        .get(&RuntimeValueKey::string("route"))
        .expect("route")
    else {
        panic!("route should be a nested map");
    };
    let HeapNode::Map(route) = decoded_heap
        .get(*route_handle)
        .expect("nested map should resolve")
    else {
        panic!("expected nested JsonObject map");
    };
    assert_eq!(
        route.get(&RuntimeValueKey::string("enabled")),
        Some(&RuntimeValue::Bool(true))
    );

    let record_envelope = RecoverableEnvelope::new(RecoverableNode::plain(
        RecoverableValueKind::Record,
        RecoverableState::Record(vec![RecoverableField {
            field_identity: "providerKey".to_string(),
            value: string_node("test"),
        }]),
    ));
    let record_bytes = record_envelope
        .to_canonical_bytes(&RecoverableValidationLimits::default())
        .expect("record JsonObject envelope should encode");
    RecoverableBoundaryCodec::decode(
        &record_bytes,
        &expected,
        &context,
        &mut RequestHeap::default(),
    )
    .expect("record-shaped JsonObject should decode scalar child values as Json");
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
    let decoded =
        RecoverableBoundaryCodec::decode(&encoded, &bytes_expected(), &context, &mut decoded_heap)
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

    let strict_error =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
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
    let HeapNode::Object(object) = heap.get(handle).expect("decoded object should resolve") else {
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

    let error =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
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
    let hooks = TestBehaviorHooks::with_additional_conformance(WRITER_INTERFACE, WRITER_PROJECTION);

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

    let error =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
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

    let error =
        RecoverableBoundaryCodec::encode(&value, &expected, &external_recoverable_context(), &heap)
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
    let error =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
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
    let InterfaceValueState::Local { self_node } = state;
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
    assert_eq!(concrete_type, READER_RUNTIME_IMPL);
    assert_eq!(method_table.id(), READER_PROJECTION);
    assert_eq!(method_table.interface_abi_id(), READER_INTERFACE);
    assert_eq!(method_table.slots()[0].method_abi_id(), READER_METHOD);
    assert_eq!(payload, &RuntimeValue::String("Ada".to_string()));

    let reencoded = RecoverableBoundaryCodec::encode_envelope_with_behavior(
        &decoded,
        &expected,
        &context,
        &decode_heap,
        &hooks,
    )
    .expect("decoded local interface should re-encode through behavior hook");
    assert_eq!(hooks.encode_calls.get(), 2);
    assert!(matches!(
        reencoded.root.state,
        RecoverableState::InterfaceValue(InterfaceValueState::Local { .. })
    ));

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

    let error =
        RecoverableBoundaryCodec::encode_with_behavior(&value, &expected, &context, &heap, &hooks)
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
fn callback_capability_recoverable_encode_rejects_before_every_behavior_hook() {
    let expected = any_reader_expected();
    let mut heap = RequestHeap::default();
    let value = RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            READER_INTERFACE.to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                READER_INTERFACE,
                "capability-1",
            )),
        ))
        .expect("callback capability should allocate"),
    );
    for kind in [
        RuntimeRecoverableBoundaryKind::RuntimeBinaryPayload,
        RuntimeRecoverableBoundaryKind::TaskDispatchPayload,
        RuntimeRecoverableBoundaryKind::QueueWorkItemPayload,
    ] {
        let context = RuntimeRecoverableBoundaryContext::new(
            kind,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_explicit_recoverable_slot();
        let hooks = TestBehaviorHooks::default();
        let error = RecoverableBoundaryCodec::encode_with_behavior(
            &value, &expected, &context, &heap, &hooks,
        )
        .expect_err("callback capability must never enter recoverable encoding");
        let RuntimeError::Recoverable(error) = error else {
            panic!("expected structured recoverable error");
        };
        assert_eq!(
            error.code(),
            RecoverableBoundaryErrorCode::CallbackCapabilityNotRecoverable
        );
        assert_eq!(hooks.encode_calls.get(), 0);
        assert_eq!(hooks.restore_calls.get(), 0);
        assert_eq!(hooks.conformance_calls.get(), 0);
        assert_eq!(hooks.table_calls.get(), 0);
    }
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
    let error =
        RecoverableBoundaryCodec::decode(&bytes, &expected, &context, &mut RequestHeap::default())
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
    let decoded =
        RecoverableBoundaryCodec::decode_envelope_canonical(&bytes, &limits, &expected, &context)
            .expect("envelope canonical decode should succeed");

    assert_eq!(decoded, envelope);
}

#[test]
fn unavailable_artifact_fails_closed_with_required_diagnostics() {
    let envelope = RecoverableEnvelope::new(native_adapter_artifact_plain_node("missing-build"));
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
