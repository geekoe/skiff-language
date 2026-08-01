use super::{super::*, support::*};

#[test]
fn duplicate_key_code_detection_is_exact() {
    assert!(is_mongo_duplicate_key_code(11000));
    assert!(!is_mongo_duplicate_key_code(11001));
    assert!(!is_mongo_duplicate_key_code(12582));
}

#[test]
fn duplicate_key_error_detection_uses_mongo_write_error_code() {
    let write_error: WriteError = serde_json::from_value(json!({
        "code": 11000,
        "codeName": "DuplicateKey",
        "errmsg": "duplicate key"
    }))
    .expect("mongodb WriteError should deserialize");
    let error: MongoError = MongoErrorKind::Write(WriteFailure::WriteError(write_error)).into();

    assert!(is_mongo_duplicate_key_error(&error));
}

#[test]
fn write_conflict_code_detection_is_exact() {
    assert!(is_mongo_write_conflict_code(112));
    assert!(!is_mongo_write_conflict_code(111));
    assert!(!is_mongo_write_conflict_code(113));
    assert!(!is_mongo_write_conflict_code(11000));
}

#[test]
fn write_conflict_error_detection_covers_write_command_variants() {
    let command = mongo_command_error(112, "WriteConflict");
    let write_error: MongoError = MongoErrorKind::Write(WriteFailure::WriteError(
        mongo_write_error(112, "WriteConflict"),
    ))
    .into();
    let write_concern: MongoError = MongoErrorKind::Write(WriteFailure::WriteConcernError(
        mongo_write_concern_error(112, "WriteConflict"),
    ))
    .into();
    let insert_many: InsertManyError = serde_json::from_value(json!({
        "writeErrors": [{
            "index": 0,
            "code": 112,
            "codeName": "WriteConflict",
            "errmsg": "write conflict"
        }],
        "writeConcernError": null
    }))
    .expect("mongodb InsertManyError should deserialize");
    let insert_many: MongoError = MongoErrorKind::InsertMany(insert_many).into();
    let mut bulk_write = BulkWriteError::default();
    bulk_write
        .write_errors
        .insert(0, mongo_write_error(112, "WriteConflict"));
    bulk_write
        .write_concern_errors
        .push(mongo_write_concern_error(112, "WriteConflict"));
    let bulk_write: MongoError = MongoErrorKind::BulkWrite(bulk_write).into();

    for error in [command, write_error, write_concern, insert_many, bulk_write] {
        assert!(is_mongo_write_conflict_error(&error), "{error}");
    }
    assert!(!is_mongo_write_conflict_error(&mongo_command_error(
        113,
        "ConflictingOperationInProgress"
    )));
    assert!(is_mongo_db_conflict_error(&mongo_command_error(
        112,
        "WriteConflict"
    )));
}

#[test]
fn db_conflict_classification_accepts_code_or_transient_transaction_label() {
    assert!(mongo_db_conflict_markers_match(true, |_| false));
    assert!(mongo_db_conflict_markers_match(false, |label| {
        label == TRANSIENT_TRANSACTION_ERROR
    }));
    assert!(!mongo_db_conflict_markers_match(false, |label| {
        label == UNKNOWN_TRANSACTION_COMMIT_RESULT
    }));
    assert!(!mongo_db_conflict_markers_match(false, |_| false));
}

#[test]
fn retry_update_drops_set_on_insert() {
    let update = doc! {
        "$set": { "title": "updated" },
        "$setOnInsert": { "_id": "thread-1", "title": "created" }
    };

    assert_eq!(
        update_without_set_on_insert(&update),
        Some(doc! { "$set": { "title": "updated" } })
    );
    assert!(update.contains_key("$setOnInsert"));
}

#[test]
fn retry_update_is_skipped_when_only_set_on_insert_remains() {
    let update = doc! {
        "$setOnInsert": { "_id": "thread-1", "title": "created" }
    };

    assert_eq!(update_without_set_on_insert(&update), None);
}
