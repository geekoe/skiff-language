use super::{super::*, recoverable_support::*, support::*};

#[test]
fn recoverable_envelope_field_roundtrips_plain_values_as_opaque_binary() {
    let binding = recoverable_envelope_metadata();
    let settings = json!({
        "mode": "dark",
        "enabled": true,
        "items": ["alpha", "beta"],
        "none": null
    });

    let (document, materialized) = binding
        .document_from_business_value(db_doc(json!({
            "id": "thread-1",
            "title": "Hello",
            "settings": settings
        })))
        .expect("recoverable-envelope field should store plain values");

    let Some(Bson::Binary(binary)) = document.get("settings") else {
        panic!("settings should be stored as BSON binary recoverable envelope");
    };
    assert_eq!(binary.subtype, BinarySubtype::Generic);
    assert!(!binary.bytes.is_empty());
    assert_eq!(
        materialized.as_value(),
        &json!({
            "id": "thread-1",
            "title": "Hello",
            "settings": settings
        })
    );

    let read = binding
        .business_value_from_document(document)
        .expect("recoverable-envelope field should decode");
    assert_eq!(read.as_value(), materialized.as_value());
}

#[test]
fn callback_capability_db_write_fails_closed_before_recoverable_hooks() {
    let binding = recoverable_envelope_metadata();
    let mut heap = RequestHeap::default();
    let callback = RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            "contract:reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                "contract:reader",
                "capability-1",
            )),
        ))
        .expect("callback capability should allocate"),
    );
    let row = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("thread-1".to_string())),
            ("title", RuntimeValue::String("Hello".to_string())),
            ("settings", callback),
        ],
    );

    let error = binding
        .document_from_runtime_business_value(&row, &heap, None)
        .expect_err("callback capability must never enter DB persistence");
    assert!(
        error
            .to_string()
            .contains("callback_capability_not_recoverable"),
        "unexpected DB callback rejection: {error}"
    );
}

#[test]
fn recoverable_envelope_runtime_read_ignores_historical_extra_record_fields() {
    let binding = recoverable_envelope_metadata();
    let document = recoverable_settings_document_with_expected(
        &binding,
        recoverable_settings_expected(&[
            ("mode", string_expected(), true),
            ("legacy", string_expected(), true),
        ]),
        runtime_settings_object([
            ("mode", RuntimeValue::String("dark".to_string())),
            ("legacy", RuntimeValue::String("old".to_string())),
        ]),
    );

    let decoded = recoverable_settings_runtime_read_with_expected(
        &binding,
        document,
        recoverable_settings_expected(&[("mode", string_expected(), true)]),
    )
    .expect("DB durable read should ignore historical extra fields");

    assert_eq!(
        decoded.get("mode"),
        Some(&RuntimeValue::String("dark".to_string()))
    );
    assert!(
        !decoded.contains_key("legacy"),
        "historical field must not be materialized into current runtime object"
    );
}

#[test]
fn recoverable_envelope_runtime_read_materializes_missing_nullable_fields() {
    let binding = recoverable_envelope_metadata();
    let document = recoverable_settings_document_with_expected(
        &binding,
        recoverable_settings_expected(&[("mode", string_expected(), true)]),
        runtime_settings_object([("mode", RuntimeValue::String("dark".to_string()))]),
    );

    let decoded = recoverable_settings_runtime_read_with_expected(
        &binding,
        document,
        recoverable_settings_expected(&[
            ("mode", string_expected(), true),
            ("nickname", nullable_string_expected(), false),
        ]),
    )
    .expect("DB durable read should materialize selected missing nullable fields");

    assert_eq!(
        decoded.get("mode"),
        Some(&RuntimeValue::String("dark".to_string()))
    );
    assert_eq!(decoded.get("nickname"), Some(&RuntimeValue::Null));
}

#[test]
fn recoverable_envelope_runtime_read_rejects_missing_required_fields() {
    let binding = recoverable_envelope_metadata();
    let document = recoverable_settings_document_with_expected(
        &binding,
        recoverable_settings_expected(&[("mode", string_expected(), true)]),
        runtime_settings_object([("mode", RuntimeValue::String("dark".to_string()))]),
    );

    let error = recoverable_settings_runtime_read_with_expected(
        &binding,
        document,
        recoverable_settings_expected(&[
            ("mode", string_expected(), true),
            ("nickname", string_expected(), true),
        ]),
    )
    .expect_err("DB durable read must still reject missing required fields");

    assert!(
        error
            .to_string()
            .contains("recoverable-envelope DB field decode failed"),
        "{error}"
    );
}

#[test]
fn nullable_recoverable_envelope_bson_null_decodes_to_business_json_null() {
    let binding = recoverable_nullable_envelope_metadata();

    let read = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "title": "Hello",
            "settings": Bson::Null
        })
        .expect("nullable recoverable-envelope BSON null should decode");

    assert_eq!(
        read.as_value(),
        &json!({
            "id": "thread-1",
            "title": "Hello",
            "settings": null
        })
    );
}

#[test]
fn nullable_recoverable_envelope_bson_null_decodes_to_runtime_null() {
    let binding = recoverable_nullable_envelope_metadata();
    let mut heap = RequestHeap::default();

    let read = binding
        .runtime_business_value_from_document(
            doc! {
                "_id": "thread-1",
                "title": "Hello",
                "settings": Bson::Null
            },
            &mut heap,
            None,
        )
        .expect("nullable recoverable-envelope BSON null should decode to runtime null");

    let RuntimeValue::Heap(handle) = read else {
        panic!("decoded DB row should be an object");
    };
    let HeapNode::Object(object) = heap.get(handle).expect("decoded object handle") else {
        panic!("decoded DB row should be an object");
    };
    assert_eq!(object.fields().get("settings"), Some(&RuntimeValue::Null));
}

#[test]
fn non_nullable_recoverable_envelope_bson_null_remains_decode_error() {
    let binding = recoverable_envelope_metadata();

    let error = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "title": "Hello",
            "settings": Bson::Null
        })
        .expect_err("non-nullable recoverable-envelope BSON null should still fail");

    assert!(
        error
            .to_string()
            .contains("recoverable-envelope DB field stored non-binary null"),
        "{error}"
    );
}

#[test]
fn recoverable_envelope_runtime_field_roundtrips_local_interface_with_hooks() {
    let binding = recoverable_provider_metadata();
    let mut heap = RequestHeap::default();
    let provider = local_provider_runtime_value(&mut heap, "anthropic");
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("binding-1".to_string())),
            ("provider", provider),
        ],
    );
    let hooks = TestDbBehaviorHooks::default();
    let expected = test_provider_expected_plan();
    let artifact_store =
        TestDbArtifactStore::default().with_available(TEST_SERVICE_ARTIFACT, TEST_SERVICE_BUILD);
    let mut root_store = TestDbRootStore::default();
    let mut write_context = DbRecoverableRuntimeWriteContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: Some(&expected),
        recoverable_expected_overrides: None,
        artifact_store: Some(&artifact_store),
        retention_root_store: Some(&mut root_store),
        retention_expires_at_epoch_millis: Some(1_609_459_200_000),
    };

    let document = binding
        .document_from_runtime_business_value(&value, &heap, Some(&mut write_context))
        .expect("local interface envelope should encode through DB runtime hook outlet");

    assert_eq!(hooks.encode_calls(), 1);
    let Some(Bson::Binary(binary)) = document.get("provider") else {
        panic!("provider should be stored as recoverable-envelope BSON binary");
    };
    assert_eq!(binary.subtype, BinarySubtype::Generic);
    assert!(!binary.bytes.is_empty());

    let mut read_heap = RequestHeap::default();
    let read_context = DbRecoverableRuntimeReadContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: Some(&expected),
        recoverable_expected_overrides: None,
    };
    let decoded = binding
        .runtime_business_value_from_document(document, &mut read_heap, Some(&read_context))
        .expect("local interface envelope should decode through DB runtime hook outlet");

    assert_eq!(hooks.restore_calls(), 1);
    assert_eq!(hooks.conformance_calls(), 1);
    assert_eq!(hooks.table_calls(), 1);
    assert_decoded_provider_runtime_value(&decoded, &read_heap, "binding-1", "anthropic");

    let rewritten = binding
        .document_from_runtime_business_value(&decoded, &read_heap, Some(&mut write_context))
        .expect("decoded local interface should re-encode through DB runtime hook outlet");
    assert_eq!(hooks.encode_calls(), 2);
    let Some(Bson::Binary(rewritten_binary)) = rewritten.get("provider") else {
        panic!("rewritten provider should be stored as recoverable-envelope BSON binary");
    };
    assert_eq!(rewritten_binary.subtype, BinarySubtype::Generic);
    assert!(!rewritten_binary.bytes.is_empty());
    assert!(
        root_store.is_empty(),
        "LocalConcrete recoverable self nodes do not create artifact retention roots"
    );

    let mut reread_heap = RequestHeap::default();
    let reread = binding
        .runtime_business_value_from_document(rewritten, &mut reread_heap, Some(&read_context))
        .expect("rewritten local interface envelope should decode through DB runtime hook outlet");
    assert_decoded_provider_runtime_value(&reread, &reread_heap, "binding-1", "anthropic");
}

#[test]
fn recoverable_runtime_context_reexport_preserves_write_contract_fields() {
    let hooks = Arc::new(TestDbBehaviorHooks::default());
    let context = production_runtime_context(hooks);
    let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
    let mut root_store = CollectedRecoverableRootStore::default();
    let write_context = recoverable_write_context(&context, &artifact_store, &mut root_store);
    let overrides = write_context
        .recoverable_expected_overrides
        .expect("expected plans should be forwarded");

    assert!(context.expected_plans.field("provider").is_some());
    assert!(context.expected_plans.field("provider.name").is_some());
    assert!(overrides.contains_key("provider"));
    assert_eq!(context.artifact_identity, TEST_SERVICE_ARTIFACT);
    assert_eq!(context.build_id, TEST_SERVICE_BUILD);
    assert_eq!(
        context.boundary_context.kind,
        RuntimeRecoverableBoundaryKind::DbValue
    );
    assert_eq!(
        write_context
            .boundary_context
            .expect("boundary context should be forwarded")
            .kind,
        RuntimeRecoverableBoundaryKind::DbValue
    );
    assert_eq!(
        write_context.retention_expires_at_epoch_millis,
        Some(1_609_459_200_000)
    );
    assert!(artifact_store.can_load_artifact(TEST_SERVICE_ARTIFACT, TEST_SERVICE_BUILD));
}

#[test]
fn recoverable_envelope_runtime_field_requires_hooks_but_not_artifact_outlets_for_local_concrete() {
    let binding = recoverable_provider_metadata();
    let mut heap = RequestHeap::default();
    let value = {
        let provider = local_provider_runtime_value(&mut heap, "openai");
        runtime_object(
            &mut heap,
            [
                ("id", RuntimeValue::String("binding-1".to_string())),
                ("provider", provider),
            ],
        )
    };

    let error = binding
        .document_from_runtime_business_value(&value, &heap, None)
        .expect_err("local interface without behavior hooks should fail before write");
    assert!(
        error
            .to_string()
            .contains("recoverable-envelope DB value encode failed"),
        "{error}"
    );

    let hooks = TestDbBehaviorHooks::default();
    let mut root_store = TestDbRootStore::default();
    let mut missing_artifact_store = DbRecoverableRuntimeWriteContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: None,
        recoverable_expected_overrides: None,
        artifact_store: None,
        retention_root_store: Some(&mut root_store),
        retention_expires_at_epoch_millis: None,
    };
    binding
        .document_from_runtime_business_value(&value, &heap, Some(&mut missing_artifact_store))
        .expect("LocalConcrete behavior envelope should not require an artifact store");
    assert!(root_store.is_empty());

    let artifact_store =
        TestDbArtifactStore::default().with_available(TEST_SERVICE_ARTIFACT, TEST_SERVICE_BUILD);
    let mut missing_retention_store = DbRecoverableRuntimeWriteContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: None,
        recoverable_expected_overrides: None,
        artifact_store: Some(&artifact_store),
        retention_root_store: None,
        retention_expires_at_epoch_millis: None,
    };
    binding
        .document_from_runtime_business_value(&value, &heap, Some(&mut missing_retention_store))
        .expect("LocalConcrete behavior envelope should not require a retention root store");
}

#[test]
fn schema_projectable_runtime_field_rejects_local_interface_without_lane_switch() {
    let binding = thread_binding();
    let mut heap = RequestHeap::default();
    let title = local_provider_runtime_value(&mut heap, "not-a-title");
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("thread-1".to_string())),
            ("title", title),
        ],
    );

    let error = binding
        .document_from_runtime_business_value(&value, &heap, None)
        .expect_err("schema-projectable DB lane must reject runtime behavior values");

    assert!(
        error
            .to_string()
            .contains("schema-projectable DB value encode failed"),
        "{error}"
    );
}

#[test]
fn recoverable_envelope_field_decode_failure_is_stable() {
    let binding = recoverable_envelope_metadata();

    let error = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "title": "Hello",
            "settings": Bson::Binary(mongodb::bson::Binary {
                subtype: BinarySubtype::Generic,
                bytes: vec![0, 1, 2, 3],
            })
        })
        .expect_err("bad recoverable envelope bytes should fail the row");

    assert!(
        error
            .to_string()
            .contains("recoverable-envelope DB field decode failed"),
        "{error}"
    );
}

#[test]
fn projection_omitting_recoverable_envelope_field_does_not_decode_it() {
    let binding = recoverable_envelope_metadata();

    let projection = binding
        .projection_document(Some(&[field_path_with_text("title")]))
        .expect("projection omitting envelope field should build");
    assert_eq!(projection, Some(doc! { "_id": 1, "title": 1 }));

    let value = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "title": "Hello"
        })
        .expect("omitted envelope field should not be decoded");
    assert_eq!(
        value.as_value(),
        &json!({ "id": "thread-1", "title": "Hello" })
    );
}

#[test]
fn recoverable_envelope_field_rejects_nested_projection_predicate_order_and_partial_change() {
    let binding = recoverable_envelope_metadata();

    let top_projection = binding
        .projection_document(Some(&[field_path_with_text("settings")]))
        .expect("top-level envelope projection should select the full field");
    assert_eq!(top_projection, Some(doc! { "_id": 1, "settings": 1 }));

    let error = binding
        .projection_document(Some(&[field_path_with_text("settings.mode")]))
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "nested projection");

    let error = binding
        .query_filter(db_query(json!({ "settings": { "mode": "dark" } })))
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "predicate");

    let error = binding
        .query_filter(db_query(json!({ "settings.mode": "dark" })))
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "predicate");

    let error = binding
        .order_document(&[DbOrderEntry {
            field: field_path_with_text("settings.mode"),
            direction: DbOrderDirection::Asc,
        }])
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "order");

    let error = binding
        .order_document(&[DbOrderEntry {
            field: field_path_with_text("settings"),
            direction: DbOrderDirection::Asc,
        }])
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "order");

    let mut change = ServiceDbChange::new();
    change.set("settings.mode", json!("light"));
    let error = binding
        .validated_change_update("Thread", change)
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "partial set");

    let mut full_set = ServiceDbChange::new();
    full_set.set("settings", json!({ "mode": "light" }));
    let update = binding
        .validated_change_update("Thread", full_set)
        .expect("top-level envelope set should be a full field write");
    assert!(matches!(
        update
            .get_document("$set")
            .expect("$set should exist")
            .get("settings"),
        Some(Bson::Binary(_))
    ));

    let mut unset = ServiceDbChange::new();
    unset.unset("settings");
    let error = binding
        .validated_change_update("Thread", unset)
        .unwrap_err();
    assert_recoverable_opaque_db_error(&error, "partial change");
}

#[test]
fn recoverable_envelope_field_rejects_indexes() {
    let error = DbCollectionMetadata::from_ir(
        &recoverable_envelope_metadata_value(json!([
            {
                "name": "settings_mode",
                "fields": [
                    {
                        "field": { "text": "settings.mode", "segments": ["settings", "mode"] },
                        "direction": "asc"
                    }
                ]
            }
        ]))[0],
        0,
    )
    .expect_err("nested index on envelope field should be rejected");
    assert_recoverable_opaque_db_error(&error, "index");
}
