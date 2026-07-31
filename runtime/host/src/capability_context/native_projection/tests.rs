use super::*;
use serde_json::json;

#[test]
fn diagnosed_db_decode_projects_to_native_opaque_with_frames() {
    let error = runtime_error::RuntimeError::Opaque(Box::new(
        skiff_runtime_service_db::ServiceDbError::db_decode(
            "std.db",
            "db value missing key field id",
        ),
    ))
    .with_source(7, json!({ "sourceId": 7 }));

    let native = runtime_error_to_native(error);

    match native {
        RuntimeError::Opaque(error) => {
            let payload = error.payload();
            assert_eq!(payload.code, "std.db.DecodeError");
            assert_eq!(payload.message, "db value missing key field id");
            assert_eq!(
                payload.details.expect("diagnostic details should exist")["sourceId"].as_u64(),
                Some(7)
            );
        }
        error => panic!("expected native Opaque, got {error:?}"),
    }
}

#[test]
fn diagnosed_lease_lost_projects_to_native_opaque_with_frames() {
    let error = runtime_error::RuntimeError::Opaque(Box::new(
        skiff_runtime_service_db::ServiceDbError::LeaseLost("lease abc was lost".to_string()),
    ))
    .with_diagnostic_frame(json!({ "sourceId": 7 }));

    let native = runtime_error_to_native(error);

    match native {
        RuntimeError::Opaque(error) => {
            let payload = error.payload();
            assert_eq!(payload.code, "LeaseLost");
            assert_eq!(payload.message, "lease abc was lost");
            assert_eq!(
                payload.details.expect("diagnostic details should exist")["frames"][0]["sourceId"]
                    .as_u64(),
                Some(7)
            );
        }
        error => panic!("expected native Opaque, got {error:?}"),
    }
}

#[test]
fn host_small_root_projects_to_native_opaque() {
    let native = runtime_error_to_native(runtime_error::RuntimeError::Decode(
        "internal invariant failed".to_string(),
    ));

    match native {
        RuntimeError::Opaque(error) => {
            let payload = error.payload();
            assert_eq!(payload.code, "InternalError");
            assert_eq!(payload.message, "internal invariant failed");
        }
        error => panic!("expected native Opaque, got {error:?}"),
    }
}
