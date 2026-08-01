use super::{super::*, support::*};

#[test]
fn encrypted_json_mapping_round_trips_without_plaintext_and_binds_record_context() {
    let binding = encrypted_binding();
    let sentinel = "unique-mapping-plaintext-sentinel";
    let (first, materialized) = binding
        .document_from_business_value(db_doc(json!({
            "id": "credential-1", "apiKey": sentinel, "label": "primary"
        })))
        .expect("encrypted insert mapping");
    let (second, _) = binding
        .document_from_business_value(db_doc(json!({
            "id": "credential-1", "apiKey": sentinel, "label": "primary"
        })))
        .expect("second encrypted insert mapping");

    assert_eq!(materialized.as_value()["apiKey"], sentinel);
    assert_eq!(first.get_str("_id").unwrap(), "credential-1");
    assert_ne!(first.get("apiKey"), second.get("apiKey"));
    assert!(!format!("{first:?}").contains(sentinel));
    assert_eq!(
        binding
            .business_value_from_document(first.clone())
            .expect("encrypted read")
            .as_value()["apiKey"],
        sentinel
    );

    let mut copied = first;
    copied.insert("_id", "credential-2");
    let error = binding
        .business_value_from_document(copied)
        .expect_err("ciphertext copied to another record must fail");
    assert!(error
        .to_string()
        .contains("encrypted DB field decode failed"));
    assert!(!error.to_string().contains(sentinel));
}

#[test]
fn encrypted_insert_many_encodes_each_record_independently() {
    let binding = encrypted_binding();
    let sentinel = "insert-many-plaintext-sentinel";
    let (documents, materialized) = binding
        .documents_from_business_values(vec![
            db_doc(json!({ "id": "credential-1", "apiKey": sentinel, "label": "one" })),
            db_doc(json!({ "id": "credential-2", "apiKey": sentinel, "label": "two" })),
        ])
        .expect("encrypted insert many mapping");
    assert_eq!(documents.len(), 2);
    assert_eq!(materialized.len(), 2);
    assert_ne!(documents[0].get("apiKey"), documents[1].get("apiKey"));
    assert!(!format!("{documents:?}").contains(sentinel));
    for (index, document) in documents.into_iter().enumerate() {
        assert_eq!(
            binding
                .business_value_from_document(document)
                .unwrap()
                .as_value()["apiKey"],
            sentinel,
            "record {index}"
        );
    }
}

#[test]
fn encrypted_runtime_value_mapping_uses_the_same_storage_pipeline() {
    let binding = encrypted_binding();
    let mut heap = RequestHeap::default();
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("credential-1".to_string())),
            ("apiKey", RuntimeValue::String("runtime-secret".to_string())),
            ("label", RuntimeValue::String("primary".to_string())),
        ],
    );
    let stored = binding
        .document_from_runtime_business_value(&value, &heap, None)
        .expect("runtime encrypted mapping");
    assert!(!format!("{stored:?}").contains("runtime-secret"));

    let mut read_heap = RequestHeap::default();
    let read = binding
        .runtime_business_value_from_document(stored, &mut read_heap, None)
        .expect("runtime encrypted read");
    let RuntimeValue::Heap(handle) = read else {
        panic!("object")
    };
    let HeapNode::Object(object) = read_heap.get(handle).unwrap() else {
        panic!("object")
    };
    assert_eq!(
        object.fields().get("apiKey"),
        Some(&RuntimeValue::String("runtime-secret".to_string()))
    );
}

#[test]
fn encrypted_replacement_uses_selector_or_body_key_for_record_context() {
    let binding = encrypted_binding();
    let key = db_key(json!("credential-1"));
    let upsert_insert = binding
        .upsert_insert_value_with_key(
            db_doc(json!({ "apiKey": "upsert-secret", "label": "insert" })),
            &key,
        )
        .and_then(|value| binding.document_from_business_value(value))
        .expect("upsert insert uses selector key context")
        .0;
    assert_eq!(upsert_insert.get_str("_id").unwrap(), "credential-1");
    assert!(!format!("{upsert_insert:?}").contains("upsert-secret"));

    let normalized = binding
        .normalize_one_selector(DbOneSelector::Key(key))
        .unwrap();
    let by_key = binding
        .replacement_document_from_business_value(
            db_doc(json!({ "apiKey": "replacement-secret", "label": "key" })),
            normalized.normalized_key(),
            normalized.encrypted_context(),
        )
        .expect("key replacement uses selector key");
    assert_eq!(by_key.get_str("_id").unwrap(), "credential-1");
    assert!(!format!("{by_key:?}").contains("replacement-secret"));
    assert_eq!(
        binding
            .business_value_from_document(by_key)
            .unwrap()
            .as_value()["apiKey"],
        "replacement-secret"
    );

    let by_query = binding
        .replacement_document_from_business_value(
            db_doc(json!({
                "id": "credential-2", "apiKey": "query-secret", "label": "query"
            })),
            None,
            None,
        )
        .expect("query replacement uses body key");
    assert_eq!(by_query.get_str("_id").unwrap(), "credential-2");
    assert_eq!(
        binding
            .business_value_from_document(by_query)
            .unwrap()
            .as_value()["apiKey"],
        "query-secret"
    );
    assert!(binding
        .replacement_document_from_business_value(
            db_doc(json!({ "apiKey": "missing-key", "label": "query" })),
            None,
            None,
        )
        .is_err());
}

#[test]
fn encrypted_runtime_replacement_and_projected_read_materialize_primary_key() {
    let binding = encrypted_binding();
    let mut heap = RequestHeap::default();
    let replacement = runtime_object(
        &mut heap,
        [
            (
                "apiKey",
                RuntimeValue::String("runtime-replace".to_string()),
            ),
            ("label", RuntimeValue::String("primary".to_string())),
        ],
    );
    let key = db_key(json!("credential-1"));
    let normalized = binding
        .normalize_one_selector(DbOneSelector::Key(key))
        .unwrap();
    let mut stored = binding
        .replacement_document_from_runtime_business_value(
            &replacement,
            &heap,
            None,
            normalized.normalized_key(),
            normalized.encrypted_context(),
        )
        .expect("runtime key replacement uses selector key");
    assert_eq!(stored.get_str("_id").unwrap(), "credential-1");
    assert!(!format!("{stored:?}").contains("runtime-replace"));

    stored.remove("label");
    let mut read_heap = RequestHeap::default();
    let read = binding
        .runtime_business_value_from_document(stored, &mut read_heap, None)
        .expect("projected runtime read");
    let RuntimeValue::Heap(handle) = read else {
        panic!("object")
    };
    let HeapNode::Object(object) = read_heap.get(handle).unwrap() else {
        panic!("object")
    };
    assert_eq!(
        object.fields().get("id"),
        Some(&RuntimeValue::String("credential-1".to_string()))
    );
    assert_eq!(
        object.fields().get("apiKey"),
        Some(&RuntimeValue::String("runtime-replace".to_string()))
    );
    assert!(!object.fields().contains_key("label"));
}

#[test]
fn encrypted_storage_policy_requires_key_context_and_keeps_projection_id() {
    let binding = encrypted_binding();
    assert_eq!(
        binding
            .projection_document(Some(&[field_path_with_text("apiKey")]))
            .unwrap(),
        Some(doc! { "_id": 1, "apiKey": 1 })
    );
    assert!(binding
        .query_filter(db_query(json!({ "apiKey": "secret" })))
        .is_err());
    assert!(binding
        .order_document(&[DbOrderEntry {
            field: field_path_with_text("apiKey"),
            direction: DbOrderDirection::Asc,
        }])
        .is_err());

    let mut change = ServiceDbChange::new();
    change.set("apiKey", json!("new-secret"));
    assert!(binding
        .validated_change_update("Credential", change.clone())
        .is_err());
    let key = db_key(json!("credential-1"));
    let normalized = binding
        .normalize_one_selector(DbOneSelector::Key(key))
        .unwrap();
    let context = normalized.encrypted_context();
    let update = binding
        .validated_change_update_with_context("Credential", change, context)
        .expect("key update encrypted set");
    assert!(!format!("{update:?}").contains("new-secret"));

    let heap = RequestHeap::default();
    let runtime_update = binding
        .runtime_change_update_document(
            "Credential",
            DbRuntimeChange {
                wire_change: ServiceDbChange::new(),
                set_ops: vec![DbRuntimeSetOp {
                    field: "apiKey".to_string(),
                    value: RuntimeValue::String("runtime-update-secret".to_string()),
                }],
            },
            &heap,
            None,
            context,
        )
        .expect("runtime key update encrypted set");
    assert!(!format!("{runtime_update:?}").contains("runtime-update-secret"));

    let mut partial = ServiceDbChange::new();
    partial.unset("apiKey");
    assert!(binding
        .validated_change_update_with_context("Credential", partial, context)
        .is_err());
}

#[test]
fn encrypted_metadata_and_provider_activation_fail_closed() {
    let valid = encrypted_metadata("string", "string", json!([]));
    assert!(DbCollectionMetadata::from_ir(&valid[0], 0).is_err());
    assert!(DbCollectionMetadata::from_ir_with_encryption(
        &encrypted_metadata("number", "string", json!([]))[0],
        0,
        "example.com/credential",
        "test",
        "example.com/credential",
        Some(test_encryption_keyring().cipher())
    )
    .is_err());
    assert!(DbCollectionMetadata::from_ir_with_encryption(
        &encrypted_metadata("string", "number", json!([]))[0],
        0,
        "example.com/credential",
        "test",
        "example.com/credential",
        Some(test_encryption_keyring().cipher())
    )
    .is_err());
    let indexed = json!([{
        "name": "byApiKey", "unique": false,
        "fields": [{ "field": { "text": "apiKey", "segments": ["apiKey"] }, "direction": "asc" }]
    }]);
    let indexed_error = DbCollectionMetadata::from_ir_with_encryption(
        &encrypted_metadata("string", "string", indexed)[0],
        0,
        "example.com/credential",
        "test",
        "example.com/credential",
        Some(test_encryption_keyring().cipher()),
    )
    .expect_err("runtime-forged encrypted index must fail");
    assert!(
        indexed_error
            .to_string()
            .contains("encrypted DB field apiKey cannot be used for index"),
        "{indexed_error}"
    );
    let error = match MongoServiceDbProviderFactory::default().build(DbProviderBuildInput {
        environment: "test".to_string(),
        service_id: "example.com/credential".to_string(),
        config: DbProviderConfig::opaque(json!({ "mongoUrl": inert_mongo_url("encrypted") })),
        runtime_program_db: provider_metadata_from_ir(valid.clone()),
    }) {
        Ok(_) => panic!("encrypted activation without keyring must fail"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("no service DB encryption keyring"));
    MongoServiceDbProviderFactory::new(Some(test_encryption_keyring()))
        .build(DbProviderBuildInput {
            environment: "test".to_string(),
            service_id: "example.com/credential".to_string(),
            config: DbProviderConfig::opaque(
                json!({ "mongoUrl": inert_mongo_url("encrypted-ok") }),
            ),
            runtime_program_db: provider_metadata_from_ir(valid),
        })
        .expect("encrypted activation with keyring");
}

#[test]
fn forged_encrypted_metadata_rejects_nullable_recoverable_and_immutable_file_lanes() {
    let cases = [
        (
            "nullable string",
            json!({
                "kind": "nullable",
                "inner": { "kind": "builtin", "name": "string" }
            }),
        ),
        (
            "recoverable envelope",
            json!({
                "kind": "anyInterface",
                "interface": {
                    "interfaceAbiId": "pkg.ToolProvider",
                    "canonicalTypeArgs": []
                }
            }),
        ),
        (
            "immutable file",
            json!({
                "kind": "serviceSymbol",
                "symbol": { "modulePath": "std.file", "symbol": "ImmutableFile" }
            }),
        ),
    ];

    for (label, field_type) in cases {
        let forged = encrypted_metadata_with_field_type(field_type);
        let error = DbCollectionMetadata::from_ir_with_encryption(
            &forged[0],
            0,
            "example.com/credential",
            "test",
            "example.com/credential",
            Some(test_encryption_keyring().cipher()),
        )
        .expect_err(label);
        assert!(
            error
                .to_string()
                .contains("encrypted field apiKey must be a plain string"),
            "{label}: {error}"
        );
    }
}

#[test]
fn forged_encrypted_primary_key_field_fails_activation_even_with_keyring() {
    let forged = provider_metadata(json!([{
        "modulePath": "internal.credential",
        "kind": "object",
        "typeName": "Credential",
        "collectionName": "credential",
        "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
        "fields": [
            { "name": "id", "type": { "kind": "builtin", "name": "string" }, "storage": "encrypted" },
            { "name": "label", "type": { "kind": "builtin", "name": "string" } }
        ],
        "indexes": []
    }]));
    let cipher = test_encryption_keyring().cipher();

    let metadata_error = match ServiceDbMetadata::from_runtime_program_db_with_encryption(
        &forged,
        "test",
        "example.com/credential",
        Some(cipher),
    ) {
        Ok(_) => {
            panic!("forged encrypted primary key must fail before either mapping path is built")
        }
        Err(error) => error,
    };
    assert!(
        metadata_error
            .to_string()
            .contains("encrypted storage cannot target primary key field id"),
        "{metadata_error}"
    );

    let provider_error = match MongoServiceDbProviderFactory::new(Some(test_encryption_keyring()))
        .build(DbProviderBuildInput {
            environment: "test".to_string(),
            service_id: "example.com/credential".to_string(),
            config: DbProviderConfig::opaque(
                json!({ "mongoUrl": inert_mongo_url("forged-encrypted-key") }),
            ),
            runtime_program_db: forged,
        }) {
        Ok(_) => panic!("provider must reject forged encrypted primary key metadata"),
        Err(error) => error,
    };
    assert!(provider_error
        .to_string()
        .contains("encrypted storage cannot target primary key field id"));
}
