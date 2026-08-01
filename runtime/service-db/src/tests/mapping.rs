use super::{super::*, support::*};

#[test]
fn skiff_file_record_document_preserves_capability_record_fields() {
    let record = FileCapabilityRecord {
        id: "file-1".to_string(),
        sha256: "abc123".to_string(),
        size: 42,
        content_type: Some("text/plain".to_string()),
        purpose: Some("profile".to_string()),
        blob_key: "cas/abc123-42".to_string(),
        created_at: "2026-07-01T00:00:00Z".to_string(),
    };

    let document = skiff_file_record_document(record.clone());

    assert_eq!(
        document,
        doc! {
            "_id": "file-1",
            "id": "file-1",
            "sha256": "abc123",
            "size": Bson::Int64(42),
            "content_type": "text/plain",
            "purpose": "profile",
            "blob_key": "cas/abc123-42",
            "created_at": "2026-07-01T00:00:00Z",
        }
    );
    assert_eq!(
        skiff_file_record_from_document(document).expect("_skiff_file document should decode"),
        record
    );

    let minimal = skiff_file_record_document(FileCapabilityRecord {
        id: "file-2".to_string(),
        sha256: "def456".to_string(),
        size: 7,
        content_type: None,
        purpose: None,
        blob_key: "cas/def456-7".to_string(),
        created_at: "2026-07-01T00:00:01Z".to_string(),
    });
    assert!(!minimal.contains_key("content_type"));
    assert!(!minimal.contains_key("purpose"));
}

#[test]
fn page_order_maps_business_key_to_mongo_id() {
    let binding = thread_binding();
    let options = ServiceDbFindOptions {
        order: vec![DbOrderEntry {
            field: FieldPath {
                text: "id".to_string(),
                segments: vec!["id".to_string()],
            },
            direction: DbOrderDirection::Asc,
        }],
        ..Default::default()
    };

    assert_eq!(
        binding.page_sort_document(&options).unwrap(),
        Some(doc! { "_id": 1 })
    );
}

#[test]
fn page_order_maps_descending_business_key_to_mongo_id() {
    let binding = thread_binding();
    let options = ServiceDbFindOptions {
        order: vec![DbOrderEntry {
            field: FieldPath {
                text: "id".to_string(),
                segments: vec!["id".to_string()],
            },
            direction: DbOrderDirection::Desc,
        }],
        ..Default::default()
    };

    assert_eq!(
        binding.page_sort_document(&options).unwrap(),
        Some(doc! { "_id": -1 })
    );
}

#[test]
fn page_order_without_explicit_order_does_not_sort() {
    let binding = thread_binding();
    let options = ServiceDbFindOptions::default();

    assert_eq!(binding.page_sort_document(&options).unwrap(), None);
}

#[test]
fn page_order_uses_only_explicit_order_fields() {
    let binding = thread_binding();
    let options = ServiceDbFindOptions {
        order: vec![DbOrderEntry {
            field: FieldPath {
                text: "title".to_string(),
                segments: vec!["title".to_string()],
            },
            direction: DbOrderDirection::Asc,
        }],
        ..Default::default()
    };

    assert_eq!(
        binding.page_sort_document(&options).unwrap(),
        Some(doc! { "title": 1 })
    );
}

#[test]
fn projection_paths_use_db_boundary_path_policy() {
    let binding = thread_binding();

    assert_eq!(
        binding
            .projection_document(Some(&[field_path_with_text_and_segments("", &["title"])]))
            .expect("segments fallback should resolve declared projection paths"),
        Some(doc! { "_id": 1, "title": 1 })
    );
    assert_eq!(
        binding
            .projection_document(Some(&[field_path_with_text("_id")]))
            .expect("_id should remain accepted for mongo-facing projection paths"),
        Some(doc! { "_id": 1 })
    );

    let error = binding
        .projection_document(Some(&[field_path_with_text("title.__skiffType")]))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let error = binding
        .projection_document(Some(&[field_path_with_text("missing.nested")]))
        .unwrap_err();
    assert!(
        error.to_string().contains("is not declared on Thread"),
        "{error}"
    );
}

#[test]
fn query_selector_without_order_does_not_sort() {
    let binding = thread_binding();

    let (filter, sort) = binding
        .selector_filter_sort(DbOneSelector::Query {
            query: db_query(json!({ "title": "Hello" })),
            order: Vec::new(),
        })
        .unwrap();

    assert_eq!(filter, doc! { "title": "Hello" });
    assert_eq!(sort, None);
}

#[test]
fn query_filter_maps_business_key_to_mongo_id() {
    let binding = thread_binding();

    assert_eq!(
        binding
            .query_filter(db_query(json!({ "id": "thread-1" })))
            .unwrap(),
        doc! { "_id": "thread-1" }
    );
}

#[test]
fn key_selector_does_not_require_sort() {
    let binding = thread_binding();

    let (filter, sort) = binding
        .selector_filter_sort(DbOneSelector::Key(db_key(json!("thread-1"))))
        .unwrap();

    assert_eq!(filter, doc! { "_id": "thread-1" });
    assert_eq!(sort, None);
}

#[test]
fn upsert_insert_value_uses_selector_key() {
    let binding = thread_binding();

    let insert = binding
        .upsert_insert_value_with_key(
            db_doc(json!({ "title": "Hello" })),
            &db_key(json!("thread-1")),
        )
        .unwrap();
    assert_eq!(
        insert.as_value(),
        &json!({ "id": "thread-1", "title": "Hello" })
    );

    let error = binding
        .upsert_insert_value_with_key(
            db_doc(json!({ "id": "thread-2", "title": "Hello" })),
            &db_key(json!("thread-1")),
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("must match selector key"),
        "{error}"
    );
}

#[test]
fn document_mapping_rejects_reserved_legacy_skiff_type_metadata_in_writes() {
    let binding = thread_binding();

    let error = binding
        .document_from_business_value(db_doc(json!({
            "id": "thread-1",
            "__skiffType": "Thread",
            "title": "Hello"
        })))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let error = binding
        .document_from_business_value(db_doc(json!({
            "id": "thread-1",
            "title": {
                "__skiffType": "local type marker",
                "text": "Hello"
            }
        })))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let normalized = binding
        .normalize_one_selector(DbOneSelector::Key(db_key(json!("thread-1"))))
        .unwrap();
    let error = binding
        .replacement_document_from_business_value(
            db_doc(json!({
                "id": "thread-1",
                "title": {
                    "items": [
                        { "__skiffType": "nested type marker", "value": "one" }
                    ]
                }
            })),
            normalized.normalized_key(),
            normalized.encrypted_context(),
        )
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let insert = binding
        .upsert_insert_value_with_key(
            db_doc(json!({
                "title": {
                    "__skiffType": "local type marker",
                    "text": "Hello"
                }
            })),
            &db_key(json!("thread-1")),
        )
        .expect("upsert insert should inject the selector key before DB mapping");
    let error = binding.document_from_business_value(insert).unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);
}

#[test]
fn document_mapping_ignores_top_level_reserved_skiff_metadata_when_reading() {
    let binding = thread_binding();

    let value = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "__skiffType": "Thread",
            "__skiffLeases": {
                "writer": {
                    "token": "lease-token",
                    "expiresAtMs": 2000_i64
                }
            },
            "title": "Hello"
        })
        .expect("top-level system metadata should be stripped from business values");
    assert_eq!(
        value.as_value(),
        &json!({ "id": "thread-1", "title": "Hello" })
    );

    let error = binding
        .business_value_from_document(doc! {
            "_id": "thread-1",
            "title": {
                "__skiffType": "local type marker",
                "text": "Hello"
            }
        })
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);
}

#[test]
fn document_mapping_converts_date_fields_to_bson_dates() {
    let binding = date_metadata();

    let (document, materialized) = binding
        .document_from_business_value(db_doc(json!({
            "id": "event-1",
            "createdAt": "1970-01-01T00:00:00Z",
            "payload": {
                "recoverAt": "2026-06-04T23:12:03.456+08:00",
                "attempts": [
                    { "at": "2026-06-04T15:12:03.456Z" }
                ]
            }
        })))
        .expect("Date fields should map to Mongo Date values");

    assert_eq!(document.get_str("_id"), Ok("event-1"));
    assert_eq!(
        document
            .get_datetime("createdAt")
            .expect("createdAt should be a BSON Date")
            .timestamp_millis(),
        0
    );
    let payload = document
        .get_document("payload")
        .expect("payload should be a document");
    assert_eq!(
        payload
            .get_datetime("recoverAt")
            .expect("nested Date should be a BSON Date")
            .timestamp_millis(),
        DateTime::parse_rfc3339_str("2026-06-04T15:12:03.456Z")
            .expect("fixture Date should parse")
            .timestamp_millis()
    );
    let attempts = payload
        .get_array("attempts")
        .expect("attempts should be an array");
    let Bson::Document(first_attempt) = &attempts[0] else {
        panic!("attempt should be a document");
    };
    assert!(matches!(first_attempt.get("at"), Some(Bson::DateTime(_))));

    assert_eq!(
        materialized.as_value(),
        &json!({
            "id": "event-1",
            "createdAt": "1970-01-01T00:00:00.000Z",
            "payload": {
                "recoverAt": "2026-06-04T15:12:03.456Z",
                "attempts": [
                    { "at": "2026-06-04T15:12:03.456Z" }
                ]
            }
        })
    );
}

#[test]
fn document_mapping_reads_bson_dates_as_rfc3339_strings() {
    let binding = date_metadata();

    let value = binding
        .business_value_from_document(doc! {
            "_id": "event-1",
            "createdAt": DateTime::from_millis(0),
            "payload": {
                "recoverAt": DateTime::parse_rfc3339_str("2026-06-04T15:12:03.456Z")
                    .expect("fixture Date should parse"),
                "attempts": [
                    { "at": DateTime::from_millis(0) }
                ]
            }
        })
        .expect("BSON Date fields should map to business JSON strings");

    assert_eq!(
        value.as_value(),
        &json!({
            "id": "event-1",
            "createdAt": "1970-01-01T00:00:00.000Z",
            "payload": {
                "recoverAt": "2026-06-04T15:12:03.456Z",
                "attempts": [
                    { "at": "1970-01-01T00:00:00.000Z" }
                ]
            }
        })
    );
}

#[test]
fn query_and_change_values_convert_date_fields() {
    let binding = date_metadata();

    let filter = binding
        .query_filter(db_query(json!({
            "createdAt": { "$gte": "1970-01-01T00:00:00Z" },
            "payload.recoverAt": "2026-06-04T15:12:03.456Z"
        })))
        .expect("Date query values should map to BSON Date values");
    assert!(matches!(
        filter
            .get_document("createdAt")
            .expect("createdAt query should be a document")
            .get("$gte"),
        Some(Bson::DateTime(_))
    ));
    assert!(matches!(
        filter.get("payload.recoverAt"),
        Some(Bson::DateTime(_))
    ));

    let mut change = ServiceDbChange::new();
    change.set("payload.recoverAt", json!("1970-01-01T00:00:00Z"));
    change.add_to_set("payload.attempts", json!({ "at": "1970-01-01T00:00:00Z" }));
    let update = binding
        .validated_change_update("Event", change)
        .expect("Date change values should map to BSON Date values");

    assert!(matches!(
        update
            .get_document("$set")
            .expect("$set should exist")
            .get("payload.recoverAt"),
        Some(Bson::DateTime(_))
    ));
    let add_to_set_attempt = update
        .get_document("$addToSet")
        .expect("$addToSet should exist")
        .get_document("payload.attempts")
        .expect("attempt should be a document");
    assert!(matches!(
        add_to_set_attempt.get("at"),
        Some(Bson::DateTime(_))
    ));
}

#[test]
fn db_change_update_document_uses_last_value_for_duplicate_operator_fields() {
    let binding = thread_binding();
    let mut change = ServiceDbChange::new();
    change.set("title", json!("first set"));
    change.set("title", json!("last set"));
    change.inc("title", json!(1));
    change.inc("title", json!(2));
    change.unset("title");
    change.unset("title");
    change.add_to_set("title", json!("first add"));
    change.add_to_set("title", json!("last add"));
    change.pull("title", json!("first pull"));
    change.pull("title", json!("last pull"));

    let update = binding
        .validated_change_update("Thread", change)
        .expect("duplicate field change should materialize");

    assert_eq!(
        update
            .get_document("$set")
            .expect("$set should exist")
            .get_str("title"),
        Ok("last set")
    );
    assert!(matches!(
        update
            .get_document("$inc")
            .expect("$inc should exist")
            .get("title"),
        Some(Bson::Int32(2)) | Some(Bson::Int64(2))
    ));
    assert_eq!(
        update
            .get_document("$unset")
            .expect("$unset should exist")
            .get("title"),
        Some(&Bson::Int32(1))
    );
    assert_eq!(
        update
            .get_document("$addToSet")
            .expect("$addToSet should exist")
            .get_str("title"),
        Ok("last add")
    );
    assert_eq!(
        update
            .get_document("$pull")
            .expect("$pull should exist")
            .get_str("title"),
        Ok("last pull")
    );
}

#[test]
fn db_change_values_reject_reserved_legacy_skiff_type_metadata() {
    let binding = thread_binding();
    let mut change = ServiceDbChange::new();
    change.set(
        "title",
        json!({
            "__skiffType": "local type marker",
            "text": "Hello",
            "items": [
                {
                    "__skiffType": "nested type marker",
                    "value": "one"
                }
            ]
        }),
    );

    let error = binding
        .validated_change_update("Thread", change)
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);
}

#[test]
fn db_field_paths_reject_reserved_legacy_skiff_type_metadata() {
    let binding = thread_binding();

    let error = binding
        .query_filter(db_query(json!({ "__skiffType": "Thread" })))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let error = binding
        .query_filter(db_query(json!({ "title.__skiffType": "Thread" })))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let error = binding
        .query_filter(db_query(json!({
            "title": {
                "__skiffType": "local type marker",
                "text": "Hello"
            }
        })))
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);

    let mut change = ServiceDbChange::new();
    change.unset("title.__skiffType");
    let error = binding
        .validated_change_update("Thread", change)
        .unwrap_err();
    assert_reserved_legacy_skiff_type_error(&error);
}

#[test]
fn db_rejects_skiff_prefixed_business_fields() {
    let metadata = metadata_with_skiff_business_field();
    let error = DbCollectionMetadata::from_ir(&metadata[0], 0).unwrap_err();
    assert_reserved_skiff_metadata_error(&error);

    let binding = thread_binding();

    let mut change = ServiceDbChange::new();
    change.unset("title.__skiffBusiness");
    let error = binding
        .validated_change_update("Thread", change)
        .unwrap_err();
    assert_reserved_skiff_metadata_error(&error);
}

fn metadata_with_skiff_business_field() -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } },
                { "name": "__skiffBusiness", "type": { "kind": "builtin", "name": "string" } }
            ],
            "indexes": []
        }
    ]))
}

fn date_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(
        &db_metadata_entry(json!({
            "kind": "object",
            "typeName": "Event",
            "collectionName": "Event",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "createdAt", "type": { "kind": "builtin", "name": "Date" } },
                {
                    "name": "payload",
                    "type": {
                        "kind": "record",
                        "fields": {
                            "recoverAt": { "kind": "builtin", "name": "Date" },
                            "attempts": {
                                "kind": "builtin",
                                "name": "Array",
                                "args": [
                                    {
                                        "kind": "record",
                                        "fields": {
                                            "at": { "kind": "builtin", "name": "Date" }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            ],
            "indexes": []
        })),
        0,
    )
    .expect("Date metadata should parse")
}
