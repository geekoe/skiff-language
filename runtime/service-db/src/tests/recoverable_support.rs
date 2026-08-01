use super::{super::*, support::*};

pub(super) fn recoverable_envelope_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_envelope_metadata_value(json!([]))[0], 0)
        .expect("recoverable-envelope metadata should parse")
}

pub(super) fn recoverable_nullable_envelope_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_nullable_envelope_metadata_value()[0], 0)
        .expect("nullable recoverable-envelope metadata should parse")
}

pub(super) fn recoverable_envelope_metadata_value(indexes: Value) -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } },
                { "name": "settings", "type": { "kind": "localType", "typeIndex": 0 } }
            ],
            "indexes": indexes
        }
    ]))
}

pub(super) fn recoverable_nullable_envelope_metadata_value() -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } },
                {
                    "name": "settings",
                    "type": {
                        "kind": "nullable",
                        "inner": { "kind": "localType", "typeIndex": 0 }
                    }
                }
            ],
            "indexes": []
        }
    ]))
}

pub(super) fn recoverable_provider_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_provider_metadata_value()[0], 0)
        .expect("recoverable provider metadata should parse")
}

pub(super) fn recoverable_provider_metadata_value() -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "ProviderBinding",
            "collectionName": "ProviderBinding",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                {
                    "name": "provider",
                    "type": {
                        "kind": "anyInterface",
                        "interface": {
                            "interfaceAbiId": TEST_PROVIDER_INTERFACE,
                            "canonicalTypeArgs": []
                        }
                    }
                }
            ],
            "indexes": []
        }
    ]))
}

pub(super) const TEST_PROVIDER_INTERFACE: &str = "pkg.ToolProvider";
pub(super) const TEST_PROVIDER_PROJECTION: &str = "projection:pkg.ToolProvider:pkg.StaticProvider";
pub(super) const TEST_PROVIDER_METHOD: &str = "method:pkg.ToolProvider:complete";
pub(super) const TEST_PROVIDER_IMPL: &str = "pkg.StaticProvider";
pub(super) const TEST_PROVIDER_RUNTIME_IMPL: &str = "runtime:pkg.StaticProvider";
pub(super) const TEST_SERVICE_ARTIFACT: &str = "svc/llm";
pub(super) const TEST_SERVICE_BUILD: &str = "build-provider-a";

pub(super) fn test_provider_expected_plan() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan::any_interface(
        "any ToolProvider",
        TEST_PROVIDER_INTERFACE,
        TEST_PROVIDER_PROJECTION,
    )
}

pub(super) fn recoverable_settings_expected(
    fields: &[(&str, RuntimeRecoverableExpectedTypePlan, bool)],
) -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "Settings".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Record {
            fields: fields
                .iter()
                .map(
                    |(name, ty, required)| RuntimeRecoverableExpectedRecordFieldPlan {
                        name: (*name).to_string(),
                        ty: ty.clone(),
                        required: *required,
                    },
                )
                .collect(),
            boundary_record_kind: None,
        },
    }
}

pub(super) fn string_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "string".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::String,
    }
}

pub(super) fn nullable_string_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "string?".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Nullable {
            inner: Box::new(string_expected()),
        },
    }
}

pub(super) fn runtime_settings_object<const N: usize>(
    fields: [(&str, RuntimeValue); N],
) -> [(&str, RuntimeValue); N] {
    fields
}

pub(super) fn recoverable_settings_document_with_expected<const N: usize>(
    binding: &DbCollectionMetadata,
    expected: RuntimeRecoverableExpectedTypePlan,
    settings_fields: [(&str, RuntimeValue); N],
) -> mongodb::bson::Document {
    let hooks = TestDbBehaviorHooks::default();
    let mut heap = RequestHeap::default();
    let settings = runtime_object(&mut heap, settings_fields);
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("thread-1".to_string())),
            ("title", RuntimeValue::String("Hello".to_string())),
            ("settings", settings),
        ],
    );
    let mut write_context = DbRecoverableRuntimeWriteContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: Some(&expected),
        recoverable_expected_overrides: None,
        artifact_store: None,
        retention_root_store: None,
        retention_expires_at_epoch_millis: None,
    };
    binding
        .document_from_runtime_business_value(&value, &heap, Some(&mut write_context))
        .expect("recoverable settings fixture should encode")
}

pub(super) fn recoverable_settings_runtime_read_with_expected(
    binding: &DbCollectionMetadata,
    document: mongodb::bson::Document,
    expected: RuntimeRecoverableExpectedTypePlan,
) -> Result<RuntimeObjectFields> {
    let hooks = TestDbBehaviorHooks::default();
    let mut heap = RequestHeap::default();
    let read_context = DbRecoverableRuntimeReadContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: Some(&expected),
        recoverable_expected_overrides: None,
    };
    let decoded =
        binding.runtime_business_value_from_document(document, &mut heap, Some(&read_context))?;
    let RuntimeValue::Heap(row_handle) = decoded else {
        panic!("decoded DB row should be an object");
    };
    let HeapNode::Object(row) = heap.get(row_handle).expect("decoded row handle") else {
        panic!("decoded DB row should be an object");
    };
    let RuntimeValue::Heap(settings_handle) = row.fields().get("settings").unwrap() else {
        panic!("settings should be a heap object");
    };
    let HeapNode::Object(settings) = heap.get(*settings_handle).expect("settings handle") else {
        panic!("settings should decode as an object");
    };
    Ok(settings.fields().clone())
}

pub(super) fn local_provider_runtime_value(
    heap: &mut RequestHeap,
    provider_name: &str,
) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            TEST_PROVIDER_INTERFACE.to_string(),
            InterfaceCarrier::Local {
                concrete_type: TEST_PROVIDER_RUNTIME_IMPL.to_string(),
                method_table: test_provider_method_table(),
                payload: RuntimeValue::String(provider_name.to_string()),
            },
        ))
        .expect("local provider interface should allocate"),
    )
}

pub(super) fn test_provider_method_table() -> InterfaceMethodTable {
    InterfaceMethodTable::new(
        TEST_PROVIDER_PROJECTION.to_string(),
        TEST_PROVIDER_INTERFACE.to_string(),
        vec![InterfaceMethodSlot::new(
            0,
            TEST_PROVIDER_METHOD.to_string(),
            InterfaceMethodTarget::LocalExecutable {
                executable: skiff_runtime_model::addr::ExecutableAddr::service(0, 7),
                receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
            },
        )],
    )
}

pub(super) fn provider_self_node(provider_name: &str) -> RecoverableNode {
    RecoverableNode {
        value_kind: RecoverableValueKind::NominalObject,
        variant_identity: RecoverableVariantIdentity::None,
        code_identity: RecoverableCodeIdentity::LocalConcrete {
            owner: LocalConcreteOwner::Service,
            concrete_type_identity: TEST_PROVIDER_IMPL.to_string(),
        },
        state: RecoverableState::NominalObject(NominalObjectState::DefaultFields {
            fields: vec![RecoverableField {
                field_identity: "name".to_string(),
                value: RecoverableNode::plain(
                    RecoverableValueKind::String,
                    RecoverableState::String(provider_name.to_string()),
                ),
            }],
        }),
    }
}

pub(super) fn assert_decoded_provider_runtime_value(
    value: &RuntimeValue,
    heap: &RequestHeap,
    expected_id: &str,
    expected_provider_name: &str,
) {
    let RuntimeValue::Heap(object_handle) = value else {
        panic!("decoded DB value should be an object");
    };
    let HeapNode::Object(object) = heap.get(*object_handle).expect("object handle") else {
        panic!("decoded DB value should be an object");
    };
    assert_eq!(
        object.fields().get("id"),
        Some(&RuntimeValue::String(expected_id.to_string()))
    );
    let RuntimeValue::Heap(provider_handle) = object.fields().get("provider").unwrap() else {
        panic!("provider should be an interface heap value");
    };
    let HeapNode::Interface(provider) = heap.get(*provider_handle).expect("provider handle") else {
        panic!("provider should decode as InterfaceValue");
    };
    assert_eq!(provider.interface(), TEST_PROVIDER_INTERFACE);
    let InterfaceCarrier::Local {
        concrete_type,
        method_table,
        payload,
    } = provider.carrier()
    else {
        panic!("provider should decode as a local carrier");
    };
    assert_eq!(concrete_type, TEST_PROVIDER_RUNTIME_IMPL);
    assert_eq!(
        payload,
        &RuntimeValue::String(expected_provider_name.to_string())
    );
    assert_eq!(method_table.id(), TEST_PROVIDER_PROJECTION);
    assert_eq!(method_table.interface_abi_id(), TEST_PROVIDER_INTERFACE);
    assert_eq!(
        method_table.slots()[0].method_abi_id(),
        TEST_PROVIDER_METHOD
    );
    assert!(matches!(
        method_table.slots()[0].target(),
        InterfaceMethodTarget::LocalExecutable {
            executable,
            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
        } if *executable == skiff_runtime_model::addr::ExecutableAddr::service(0, 7)
    ));
}

pub(super) struct TestDbBehaviorHooks {
    state: Mutex<TestDbBehaviorHookState>,
}

struct TestDbBehaviorHookState {
    encode_calls: usize,
    restore_calls: usize,
    conformance_calls: usize,
    table_calls: usize,
    table_projection_identity: String,
}

impl Default for TestDbBehaviorHooks {
    fn default() -> Self {
        Self {
            state: Mutex::new(TestDbBehaviorHookState {
                encode_calls: 0,
                restore_calls: 0,
                conformance_calls: 0,
                table_calls: 0,
                table_projection_identity: TEST_PROVIDER_PROJECTION.to_string(),
            }),
        }
    }
}

impl TestDbBehaviorHooks {
    pub(super) fn encode_calls(&self) -> usize {
        self.state.lock().expect("test hook mutex").encode_calls
    }

    pub(super) fn restore_calls(&self) -> usize {
        self.state.lock().expect("test hook mutex").restore_calls
    }

    pub(super) fn conformance_calls(&self) -> usize {
        self.state
            .lock()
            .expect("test hook mutex")
            .conformance_calls
    }

    pub(super) fn table_calls(&self) -> usize {
        self.state.lock().expect("test hook mutex").table_calls
    }
}

impl RecoverableBehaviorHooks for TestDbBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        _heap: &RequestHeap,
    ) -> BoundaryResult<Option<RecoverableEncodedLocalInterfaceSelf>> {
        self.state.lock().expect("test hook mutex").encode_calls += 1;
        if request.concrete_type != TEST_PROVIDER_RUNTIME_IMPL {
            return Ok(None);
        }
        let provider_name = match request.payload {
            RuntimeValue::String(value) => value.as_str(),
            _ => "unsupported",
        };
        Ok(Some(RecoverableEncodedLocalInterfaceSelf {
            method_projection_identity: request.method_table.id().to_string(),
            self_node: provider_self_node(provider_name),
        }))
    }

    fn restore_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceRestoreRequest<'_>,
        _heap: &mut RequestHeap,
    ) -> BoundaryResult<Option<RecoverableRestoredLocalInterfaceSelf>> {
        self.state.lock().expect("test hook mutex").restore_calls += 1;
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
        let provider_name = fields
            .iter()
            .find(|field| field.field_identity == "name")
            .and_then(|field| match &field.value.state {
                RecoverableState::String(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_default();
        Ok(Some(RecoverableRestoredLocalInterfaceSelf {
            concrete_type_identity: concrete_type_identity.clone(),
            runtime_concrete_type_identity: TEST_PROVIDER_RUNTIME_IMPL.to_string(),
            payload: RuntimeValue::String(provider_name),
        }))
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> BoundaryResult<bool> {
        self.state
            .lock()
            .expect("test hook mutex")
            .conformance_calls += 1;
        Ok(request.concrete_type_identity == TEST_PROVIDER_IMPL
            && request.interface_identity == TEST_PROVIDER_INTERFACE)
    }

    fn rebuild_local_interface_method_table(
        &self,
        request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> BoundaryResult<Option<InterfaceMethodTable>> {
        let mut state = self.state.lock().expect("test hook mutex");
        state.table_calls += 1;
        if request.method_projection_identity != state.table_projection_identity {
            return Ok(None);
        }
        Ok(Some(test_provider_method_table()))
    }
}

pub(super) fn production_runtime_context(
    hooks: Arc<TestDbBehaviorHooks>,
) -> DbRecoverableRuntimeContext {
    let mut expected_plans = DbRecoverableRuntimeExpectedPlans::default();
    expected_plans.insert_field("provider".to_string(), test_provider_expected_plan());
    DbRecoverableRuntimeContext {
        behavior_hooks: hooks,
        expected_plans,
        artifact_identity: TEST_SERVICE_ARTIFACT.to_string(),
        build_id: TEST_SERVICE_BUILD.to_string(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_origin_service(RuntimeRecoverableServiceRef {
            service_id: "skiff.run/p5dbprodtest".to_string(),
            version: Some("0.1.0".to_string()),
            build_id: Some(TEST_SERVICE_BUILD.to_string()),
        })
        .with_explicit_recoverable_slot(),
        retention_expires_at_epoch_millis: Some(1_609_459_200_000),
    }
}

#[derive(Default)]
pub(super) struct TestDbArtifactStore {
    available: HashSet<(String, String)>,
}

impl TestDbArtifactStore {
    pub(super) fn with_available(mut self, artifact_identity: &str, build_id: &str) -> Self {
        self.available
            .insert((artifact_identity.to_string(), build_id.to_string()));
        self
    }
}

impl RecoverableArtifactStore for TestDbArtifactStore {
    fn can_load_artifact(&self, artifact_identity: &str, build_id: &str) -> bool {
        self.available
            .contains(&(artifact_identity.to_string(), build_id.to_string()))
    }
}

#[derive(Default)]
pub(super) struct TestDbRootStore {
    roots: Vec<RecoverableArtifactRetentionRoot>,
}

impl TestDbRootStore {
    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

impl RecoverableArtifactRetentionRootStore for TestDbRootStore {
    fn persist_roots(
        &mut self,
        roots: &[RecoverableArtifactRetentionRoot],
    ) -> std::result::Result<(), String> {
        self.roots.extend_from_slice(roots);
        Ok(())
    }
}

pub(super) fn assert_recoverable_opaque_db_error(error: &ServiceDbError, operation: &str) {
    let message = error.to_string();
    assert!(
        message.contains("recoverable-envelope DB field settings is opaque")
            && message.contains(operation),
        "{error}"
    );
}
