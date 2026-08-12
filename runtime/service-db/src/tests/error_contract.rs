use super::{super::*, support::*};

#[test]
fn service_db_error_wire_payload_preserves_db_decode_shape() {
    let payload = ServiceDbError::db_decode("std.db", "db value missing key field id").payload();

    assert_eq!(payload.code, "std.db.DecodeError");
    assert_eq!(payload.message, "db value missing key field id");
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "std.db",
            "message": "db value missing key field id",
        }))
    );
}

#[test]
fn service_db_error_wire_payload_preserves_lease_lost_shape() {
    let payload =
        ServiceDbError::LeaseLost("db lease Session.owner was lost".to_string()).payload();

    assert_eq!(payload.code, "LeaseLost");
    assert_eq!(payload.message, "db lease Session.owner was lost");
    assert_eq!(payload.status, None);
    assert_eq!(payload.details, None);
}

#[test]
fn service_db_write_conflict_is_a_sanitized_catchable_db_error() {
    let error = ServiceDbError::Mongo(mongo_command_error(112, "WriteConflict"));
    let payload = error.payload();

    assert_eq!(payload.code, "std.db.ConflictError");
    assert_eq!(
        payload.message,
        "database conflict; retry only at an explicit side-effect-safe boundary"
    );
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "std.db",
            "message": "database conflict; retry only at an explicit side-effect-safe boundary",
            "retryable": true,
        }))
    );
    assert!(!payload.message.contains("Mongo"));
    assert_eq!(
        WirePayload::catch_projection(&error),
        Some((
            skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity::DbConflict
                .catch_identity(),
            json!({
                "target": "std.db",
                "message": "database conflict; retry only at an explicit side-effect-safe boundary",
                "retryable": true,
            }),
        ))
    );
}

#[test]
fn service_db_duplicate_key_is_a_non_retryable_sanitized_constraint_error() {
    let error: MongoError = MongoErrorKind::Write(WriteFailure::WriteError(mongo_write_error(
        11000,
        "DuplicateKey-secret-physical-index",
    )))
    .into();
    let target = DbConstraintTarget::new("example.com/accounts", "user").unwrap();
    let error = ServiceDbError::Mongo(error).classify_write_constraint(&target);
    let payload = error.payload();

    assert!(matches!(
        &error,
        ServiceDbError::Constraint(violation)
            if violation.kind() == DbConstraintKind::Unique
                && violation.target() == &target
    ));
    assert_eq!(payload.code, "std.db.ConstraintError");
    assert_eq!(payload.message, "database constraint rejected the write");
    assert_eq!(
        payload.details,
        Some(json!({
            "kind": "unique",
            "packageId": "example.com/accounts",
            "collection": "user",
        }))
    );
    assert!(!payload.message.contains("DuplicateKey"));
    assert!(!payload.message.contains("physical-index"));
    assert!(payload
        .details
        .as_ref()
        .is_none_or(|details| details.get("retryable").is_none()));
    assert_eq!(
        WirePayload::catch_projection(&error),
        Some((
            skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity::DbConstraint
                .catch_identity(),
            json!({
                "kind": "unique",
                "packageId": "example.com/accounts",
                "collection": "user",
            }),
        ))
    );
}

#[test]
fn service_db_non_duplicate_write_error_is_not_a_constraint_error() {
    let error: MongoError = MongoErrorKind::Write(WriteFailure::WriteError(mongo_write_error(
        112,
        "WriteConflict",
    )))
    .into();
    let target = DbConstraintTarget::new("example.com/accounts", "user").unwrap();
    let error = ServiceDbError::Mongo(error).classify_write_constraint(&target);

    assert!(matches!(error, ServiceDbError::Mongo(_)));
}

#[test]
fn service_db_non_conflict_mongo_error_keeps_platform_error_behavior() {
    let error = ServiceDbError::Mongo(mongo_command_error(113, "ConflictingOperationInProgress"));
    let payload = error.payload();

    assert_eq!(payload.code, "PlatformMongoError");
    assert!(payload.message.contains("Error code 113"));
    assert_eq!(payload.details, None);
    assert_eq!(WirePayload::catch_projection(&error), None);
}

#[test]
fn service_db_db_decode_code_does_not_imply_a_catch_identity() {
    let error = ServiceDbError::db_decode("std.db", "db value missing key field id");

    assert_eq!(error.payload().code, "std.db.DecodeError");
    assert_eq!(WirePayload::catch_projection(&error), None);
}

#[test]
fn service_db_error_wire_payload_preserves_platform_bson_decode_code() {
    let bson_error = mongodb::bson::from_bson::<String>(Bson::Int32(42))
        .expect_err("integer BSON should not decode as string");
    let payload = ServiceDbError::BsonDe(bson_error).payload();

    assert_eq!(payload.code, "PlatformBsonDecodeError");
    assert_eq!(payload.status, None);
    assert_eq!(payload.details, None);
}

#[test]
fn service_db_error_wire_payload_preserves_invalid_metadata_code() {
    let payload =
        ServiceDbError::InvalidDbMetadata("runtime program db metadata is invalid".to_string())
            .payload();

    assert_eq!(payload.code, "InvalidArtifact");
    assert_eq!(payload.message, "runtime program db metadata is invalid");
    assert_eq!(payload.status, None);
    assert_eq!(payload.details, None);
}

#[test]
fn service_db_opaque_lower_error_delegates_payload_catch_and_any() {
    let boundary_error = skiff_runtime_boundary::error::RuntimeError::db_decode(
        "std.db",
        "db value missing key field id",
    );
    let expected_payload = boundary_error.payload();
    let expected_catch = boundary_error.catch_projection();

    let error = ServiceDbError::from(boundary_error);

    assert!(matches!(error, ServiceDbError::Opaque(_)));
    assert_eq!(error.payload(), expected_payload);
    assert_eq!(WirePayload::catch_projection(&error), expected_catch);
    assert!(WirePayload::as_any(&error).is::<skiff_runtime_boundary::error::RuntimeError>());
}
