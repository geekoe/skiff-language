use super::{super::*, support::*};

#[test]
fn object_metadata_accepts_retention_field() {
    for retention in [Value::Null, json!({ "amount": 30, "unit": "days" })] {
        ServiceDbRuntime::new(
            test_profile(),
            "example.com/test".to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &provider_metadata_from_ir(object_metadata_with_retention(retention)),
        )
        .expect("object DB metadata should allow retention");
    }
}

#[test]
fn object_metadata_uses_typed_collection_name_from_service_unit_db() {
    let metadata = db_metadata(json!([
        {
            "kind": "object",
            "typeName": "BrowserSession",
            "collectionName": "BrowserSession",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "kind": "object",
            "typeName": "internal.events.TrackEvent",
            "collectionName": "TrackEvent",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ]));

    let browser_session =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("metadata should parse");
    let track_event =
        DbCollectionMetadata::from_ir(&metadata[1], 1).expect("metadata should parse");

    assert_eq!(
        browser_session.collection_name,
        service_storage_collection_name("test.local/package", "BrowserSession")
            .expect("physical collection name")
    );
    assert_eq!(
        track_event.collection_name,
        service_storage_collection_name("test.local/package", "TrackEvent")
            .expect("physical collection name")
    );
}

#[test]
fn index_plan_maps_business_key_to_id_and_preserves_nested_order() {
    let metadata = db_metadata(json!([{
        "kind": "object",
        "typeName": "User",
        "collectionName": "users",
        "key": { "name": "id" },
        "fields": [{ "name": "profile" }],
        "indexes": [{
            "name": "byIdAndEmail",
            "unique": true,
            "fields": [
                {
                    "field": { "text": "id", "segments": ["id"] },
                    "direction": "asc"
                },
                {
                    "field": { "text": "profile.email", "segments": ["profile", "email"] },
                    "direction": "desc"
                }
            ]
        }]
    }]));
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("index metadata should parse");

    assert_eq!(
        binding
            .index_key_document(&binding.indexes[0])
            .expect("index keys should project"),
        doc! { "_id": 1, "profile.email": -1 }
    );
}

#[test]
fn object_metadata_encodes_package_owned_logical_collection_name() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&provider_metadata(json!([
        {
            "modulePath": "httpSession.db",
            "kind": "object",
            "typeName": "Session",
            "collectionName": "registry_session",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ])))
    .expect("db metadata should parse");

    assert_eq!(
        metadata
            .collection_for_target(&test_db_target(0, "httpSession.db", "Session"))
            .expect("exact Session metadata should resolve")
            .collection_name,
        service_storage_collection_name("test.local/provider-Session-0", "registry_session")
            .expect("physical collection name")
    );
}

#[test]
fn object_metadata_system_encodes_skiff_prefixed_logical_collection_name() {
    let metadata = provider_metadata(json!([
        {
            "kind": "object",
            "typeName": "File",
            "collectionName": "_skiff_file",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [],
            "indexes": []
        }
    ]));
    let runtime = ServiceDbRuntime::new(
        test_profile(),
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &metadata,
    )
    .expect("logical names cannot collide with system physical collections");

    assert_eq!(
        runtime
            .metadata
            .collection_for_target(&test_db_target(0, "", "File"))
            .expect("File metadata")
            .collection_name,
        service_storage_collection_name("test.local/provider-File-0", "_skiff_file")
            .expect("physical collection name")
    );
}

#[test]
fn object_metadata_rejects_duplicate_logical_collection_identity_within_package() {
    let metadata = db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Session",
            "collectionName": "shared",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "kind": "object",
            "typeName": "Audit",
            "collectionName": "shared",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ]));
    let entries = metadata
        .into_iter()
        .enumerate()
        .map(|(index, metadata)| DbProviderTargetMetadata {
            target: test_db_target_for_package(
                index,
                &metadata.module_path,
                &metadata.type_name,
                "example.com/provider",
                "provider-build",
            ),
            metadata,
        })
        .collect::<Vec<_>>();

    let error = ServiceDbMetadata::from_runtime_program_db(&entries)
        .err()
        .expect("duplicate Package logical collection identity must fail");
    assert!(
        error
            .to_string()
            .contains("repeats one Package logical collection identity"),
        "{error}"
    );
}

#[test]
fn object_metadata_rejects_one_package_id_resolved_to_different_builds() {
    let metadata = db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Session",
            "collectionName": "session",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "kind": "object",
            "typeName": "Audit",
            "collectionName": "audit",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ]));
    let entries = metadata
        .into_iter()
        .enumerate()
        .map(|(index, metadata)| DbProviderTargetMetadata {
            target: test_db_target_for_package(
                index,
                &metadata.module_path,
                &metadata.type_name,
                "example.com/provider",
                &format!("provider-build-{index}"),
            ),
            metadata,
        })
        .collect::<Vec<_>>();

    let error = ServiceDbMetadata::from_runtime_program_db(&entries)
        .err()
        .expect("one Package ID cannot select different builds");
    assert!(
        error
            .to_string()
            .contains("resolves package ID example.com/provider to different exact Package builds"),
        "{error}"
    );
}

#[test]
fn object_metadata_rejects_reserved_legacy_skiff_type_key_and_field_names() {
    let key_error = ServiceDbRuntime::new(
        test_profile(),
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &provider_metadata(json!([
            {
                "kind": "object",
                "typeName": "Thread",
                "collectionName": "Thread",
                "key": { "name": "__skiffType", "type": { "kind": "builtin", "name": "string" } },
                "fields": [
                    { "name": "title", "type": { "kind": "builtin", "name": "string" } }
                ],
                "indexes": []
            }
        ])),
    )
    .err()
    .expect("reserved key metadata should be rejected");
    assert_reserved_legacy_skiff_type_error(&key_error);

    let field_error = ServiceDbRuntime::new(
        test_profile(),
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &provider_metadata(json!([
            {
                "kind": "object",
                "typeName": "Thread",
                "collectionName": "Thread",
                "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
                "fields": [
                    { "name": "__skiffType", "type": { "kind": "builtin", "name": "string" } }
                ],
                "indexes": []
            }
        ])),
    )
    .err()
    .expect("reserved field metadata should be rejected");
    assert_reserved_legacy_skiff_type_error(&field_error);
}

#[test]
fn object_metadata_tracks_direct_and_nullable_immutable_file_fields() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&provider_metadata(json!([
        {
            "kind": "object",
            "typeName": "Interaction",
            "collectionName": "Interaction",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                {
                    "name": "requestFile",
                    "type": {
                        "kind": "serviceSymbol",
                        "symbol": { "modulePath": "std.file", "symbol": "ImmutableFile" }
                    }
                },
                {
                    "name": "responseFile",
                    "type": {
                        "kind": "nullable",
                        "inner": {
                            "kind": "serviceSymbol",
                            "symbol": { "modulePath": "std.file", "symbol": "ImmutableFile" }
                        }
                    }
                },
                { "name": "title", "type": { "kind": "builtin", "name": "string" } }
            ],
            "indexes": []
        }
    ])))
    .expect("metadata should parse");

    let binding = metadata
        .collection_for_target(&test_db_target(0, "", "Interaction"))
        .expect("Interaction should resolve");
    assert_eq!(
        binding.immutable_file_paths,
        vec![
            vec!["requestFile".to_string()],
            vec!["responseFile".to_string()]
        ]
    );
}

#[test]
fn object_metadata_tracks_nested_immutable_file_fields() {
    let binding = DbCollectionMetadata::from_ir(
        &db_metadata_entry(json!({
            "kind": "object",
            "typeName": "Envelope",
            "collectionName": "Envelope",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                {
                    "name": "payload",
                    "type": {
                        "kind": "record",
                        "fields": {
                            "file": {
                                "kind": "serviceSymbol",
                                "symbol": { "modulePath": "std.file", "symbol": "ImmutableFile" }
                            }
                        }
                    }
                }
            ],
            "indexes": []
        })),
        0,
    )
    .expect("metadata should parse");

    assert_eq!(
        binding.immutable_file_paths,
        vec![vec!["payload".to_string(), "file".to_string()]]
    );
}

#[test]
fn object_metadata_builds_db_boundary_plans_for_key_and_fields() {
    let binding = DbCollectionMetadata::from_ir(
        &db_metadata_entry(json!({
            "kind": "object",
            "typeName": "Event",
            "collectionName": "Event",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "createdAt", "type": { "kind": "builtin", "name": "Date" } }
            ],
            "indexes": []
        })),
        0,
    )
    .expect("metadata should parse");

    let key_plan = binding
        .key_ty
        .as_ref()
        .expect("key type should build a plan");
    assert!(matches!(
        db_boundary::db_value_projection(key_plan.write_projection_ref()),
        db_boundary::DbValueProjection::Scalar
    ));

    let field_plan = binding
        .fields
        .get("createdAt")
        .and_then(|field| field.ty.as_ref())
        .expect("field type should build a plan");
    assert!(matches!(
        db_boundary::db_value_projection(field_plan.result_decode_ref()),
        db_boundary::DbValueProjection::Date
    ));
}

#[test]
fn object_metadata_parses_lease_slots() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&provider_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id" },
            "fields": [],
            "leases": [
                { "name": "writer", "ttlMs": 1000, "maxMs": 5000 },
                { "name": "reader", "ttlMs": 250 }
            ],
            "indexes": []
        }
    ])))
    .expect("db metadata should parse");

    let binding = metadata
        .collection_for_target(&test_db_target(0, "", "Thread"))
        .expect("Thread metadata should resolve");
    let writer = binding
        .lease("writer")
        .expect("writer lease should resolve");
    assert_eq!(writer.ttl_ms, 1000);
    assert_eq!(writer.max_ms, Some(5000));
    let reader = binding
        .lease("reader")
        .expect("reader lease should resolve");
    assert_eq!(reader.ttl_ms, 250);
    assert_eq!(reader.max_ms, None);
}

#[test]
fn object_metadata_rejects_unsafe_lease_slot_names() {
    for name in ["owner.lock", "$owner", "owner$lock", "owner\0lock"] {
        let error = DbCollectionMetadata::from_ir(
            &db_metadata_entry(json!({
                "kind": "object",
                "typeName": "Thread",
                "collectionName": "Thread",
                "key": { "name": "id" },
                "fields": [],
                "leases": [{ "name": name, "ttlMs": 1000 }],
                "indexes": []
            })),
            0,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot contain '.', '$', or NUL"),
            "{name:?}: {error}"
        );
    }
}

#[test]
fn metadata_lookup_uses_exact_target_identity_instead_of_type_name() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&provider_metadata(json!([
        {
            "modulePath": "internal.models",
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "threads_a",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "modulePath": "internal.archive",
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "threads_b",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ])))
    .expect("db metadata should parse");

    assert_eq!(
        metadata
            .collection_for_target(&test_db_target(0, "internal.models", "Thread"))
            .expect("exact models Thread should resolve")
            .collection_name,
        service_storage_collection_name("test.local/provider-Thread-0", "threads_a")
            .expect("physical collection name")
    );
    assert_eq!(
        metadata
            .collection_for_target(&test_db_target(1, "internal.archive", "Thread"))
            .expect("exact archive Thread should resolve")
            .collection_name,
        service_storage_collection_name("test.local/provider-Thread-1", "threads_b")
            .expect("physical collection name")
    );

    let error = metadata
        .collection_for_target(&test_db_target(2, "internal.models", "Thread"))
        .expect_err("substituted exact target must not resolve");
    assert!(
        error
            .to_string()
            .contains("does not declare the exact DB target"),
        "{error}"
    );
}

#[test]
fn metadata_keeps_identical_type_names_from_distinct_exact_targets_separate() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&provider_metadata(json!([
        {
            "modulePath": "model",
            "kind": "object",
            "typeName": "Session",
            "collectionName": "sessions_a",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "modulePath": "model",
            "kind": "object",
            "typeName": "Session",
            "collectionName": "sessions_b",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ])))
    .expect("distinct exact DB targets must not collide by display type name");

    assert_eq!(
        metadata
            .collection_for_target(&test_db_target(0, "model", "Session"))
            .expect("first exact Session target")
            .collection_name,
        service_storage_collection_name("test.local/provider-Session-0", "sessions_a")
            .expect("physical collection name")
    );
    assert_eq!(
        metadata
            .collection_for_target(&test_db_target(1, "model", "Session"))
            .expect("second exact Session target")
            .collection_name,
        service_storage_collection_name("test.local/provider-Session-1", "sessions_b")
            .expect("physical collection name")
    );
}

#[test]
fn lease_guards_do_not_cross_distinct_exact_targets_with_the_same_type_name() {
    let entries = provider_metadata(json!([
        {
            "modulePath": "model",
            "kind": "object",
            "typeName": "Session",
            "collectionName": "sessions_a",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        },
        {
            "modulePath": "model",
            "kind": "object",
            "typeName": "Session",
            "collectionName": "sessions_b",
            "key": { "name": "id" },
            "fields": [],
            "indexes": []
        }
    ]));
    let first_target = entries[0].target.clone();
    let second_target = entries[1].target.clone();
    let metadata =
        ServiceDbMetadata::from_runtime_program_db(&entries).expect("DB metadata should parse");
    let first = metadata
        .collection_for_target(&first_target)
        .expect("first exact Session target");
    let filter = doc! { "status": "open" };
    let other_target_hold = DbLeaseHold {
        target_key: second_target.lookup_key().to_string(),
        type_name: "Session".to_string(),
        key: db_key(json!("session-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };
    assert_eq!(
        guarded_filter(first, filter.clone(), &[other_target_hold], 1000)
            .expect("foreign exact target guard should be ignored"),
        filter
    );

    let exact_target_hold = DbLeaseHold {
        target_key: first_target.lookup_key().to_string(),
        type_name: "Session".to_string(),
        key: db_key(json!("session-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };
    assert_ne!(
        guarded_filter(first, filter.clone(), &[exact_target_hold], 1000)
            .expect("same exact target guard should fence"),
        filter
    );
}
