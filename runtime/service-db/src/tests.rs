use super::{
    mapping::{DbRecoverableRuntimeReadContext, DbRecoverableRuntimeWriteContext},
    metadata::{DbCollectionMetadata, ServiceDbMetadata},
    mongo::{
        is_mongo_duplicate_key_code, is_mongo_duplicate_key_error, update_without_set_on_insert,
    },
    *,
};
use crate::{DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans, ServiceDbError};
use mongodb::{
    bson::{doc, spec::BinarySubtype, Bson, DateTime},
    error::{Error as MongoError, ErrorKind as MongoErrorKind, WriteError, WriteFailure},
};
use serde_json::{json, Map, Value};
use skiff_artifact_model::DbMetadataIr;
use skiff_runtime_boundary::{
    db as db_boundary,
    recoverable::{
        RecoverableArtifactRetentionRootStore, RecoverableArtifactStore, RecoverableBehaviorHooks,
        RecoverableEncodedLocalInterfaceSelf, RecoverableInterfaceConformanceRequest,
        RecoverableInterfaceMethodTableRequest, RecoverableLocalInterfaceEncodeRequest,
        RecoverableLocalInterfaceRestoreRequest, RecoverableRemoteInterfaceCarrierRequest,
        RecoverableRestoredLocalInterfaceSelf,
    },
    Result as BoundaryResult,
};
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityError, DbDocument, DbKey, DbOneSelector, DbOrderDirection,
    DbOrderEntry, DbProviderBuildInput, DbProviderConfig, DbProviderFactory, DbQuery, FieldPath,
    ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_model::{
    error::WirePayload,
    recoverable::{
        LocalConcreteOwner, NominalObjectState, RecoverableArtifactRetentionRoot,
        RecoverableCodeIdentity, RecoverableField, RecoverableNode, RecoverableState,
        RecoverableValueKind, RecoverableVariantIdentity, RuntimeRecoverableBoundaryContext,
        RuntimeRecoverableBoundaryKind, RuntimeRecoverableExpectedRecordFieldPlan,
        RuntimeRecoverableExpectedTypeNode, RuntimeRecoverableExpectedTypePlan,
        RuntimeRecoverableServiceRef, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceCarrier, InterfaceMethodSlot, InterfaceMethodTable,
        InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue, RemoteOperationTable,
        RuntimeObject, RuntimeObjectFields, RuntimeValue,
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as TokioMutex;

fn db_key(value: serde_json::Value) -> DbKey {
    DbKey::new(value)
}

fn db_query(value: serde_json::Value) -> DbQuery {
    DbQuery::new(value)
}

fn db_doc(value: serde_json::Value) -> DbDocument {
    DbDocument::new(value)
}

fn db_metadata(mut value: Value) -> Vec<DbMetadataIr> {
    let entries = value
        .as_array_mut()
        .expect("test db metadata should be an array");
    for entry in entries {
        normalize_db_metadata_entry(entry);
    }
    serde_json::from_value(value).expect("test db metadata should decode as typed IR")
}

fn db_metadata_entry(value: Value) -> DbMetadataIr {
    let mut entries = db_metadata(json!([value]));
    entries
        .pop()
        .expect("test db metadata should contain one entry")
}

fn normalize_db_metadata_entry(entry: &mut Value) {
    let object = entry
        .as_object_mut()
        .expect("test db metadata entry should be an object");
    object
        .entry("modulePath")
        .or_insert_with(|| Value::String(String::new()));
    object
        .entry("sourceRole")
        .or_insert_with(|| Value::String("service".to_string()));
    let type_name = object
        .get("typeName")
        .and_then(Value::as_str)
        .expect("test db metadata entry should have typeName")
        .to_string();
    object.entry("type").or_insert_with(|| {
        json!({
            "kind": "dbObjectSymbol",
            "symbol": { "modulePath": "", "symbol": type_name }
        })
    });
    if !object.contains_key("collectionName")
        || object.get("collectionName").is_some_and(Value::is_null)
    {
        object.insert(
            "collectionName".to_string(),
            Value::String(
                type_name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&type_name)
                    .to_string(),
            ),
        );
    }
    normalize_db_key(object);
    normalize_db_fields(object);
    object.entry("leases").or_insert_with(|| json!([]));
    normalize_db_indexes(object);
}

fn normalize_db_key(object: &mut Map<String, Value>) {
    if let Some(key) = object.get_mut("key").and_then(Value::as_object_mut) {
        key.entry("type")
            .or_insert_with(|| json!({ "kind": "builtin", "name": "string" }));
    }
}

fn normalize_db_fields(object: &mut Map<String, Value>) {
    let fields = object.entry("fields").or_insert_with(|| json!([]));
    for field in fields
        .as_array_mut()
        .expect("test db metadata fields should be an array")
    {
        if let Some(field) = field.as_object_mut() {
            field
                .entry("type")
                .or_insert_with(|| json!({ "kind": "builtin", "name": "string" }));
        }
    }
}

fn normalize_db_indexes(object: &mut Map<String, Value>) {
    let indexes = object.entry("indexes").or_insert_with(|| json!([]));
    for index in indexes
        .as_array_mut()
        .expect("test db metadata indexes should be an array")
    {
        if let Some(index) = index.as_object_mut() {
            index.entry("unique").or_insert(Value::Bool(false));
            index.entry("fields").or_insert_with(|| json!([]));
            index.entry("where").or_insert(Value::Null);
        }
    }
}

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

#[test]
fn service_db_capability_context_does_not_require_request_frame() {
    let context = DbCapabilityContext::from_handle(ServiceDbCapabilityHandle::with_state(
        None,
        Arc::new(TokioMutex::new(DbRequestState::default())),
    ));

    let error = match context.require_store(
        "db.get",
        "serviceDb is not configured for this service activation",
    ) {
        Ok(_) => panic!("minimal unconfigured DB context should not create a store"),
        Err(error) => error,
    };

    match error {
        DbCapabilityError::ProviderUnavailable { target, reason } => {
            assert_eq!(target, "db.get");
            assert_eq!(
                reason,
                "serviceDb is not configured for this service activation"
            );
        }
        other => panic!("expected ProviderUnavailable, got {other:?}"),
    }
}

#[test]
fn mongo_provider_builds_db_capability_source_from_valid_opaque_config() {
    let source = MongoServiceDbProviderFactory
        .build(provider_input(json!({
            "mongoUrl": inert_mongo_url("provider-valid")
        })))
        .expect("valid provider config should build DB capability source");
    let context = source.context_for_request("owner", "request");

    context
        .require_store("std.db.findOne", "serviceDb is required")
        .expect("provider-built source should create a DB store");
}

#[test]
fn mongo_provider_rejects_invalid_opaque_config() {
    for (config, expected) in [
        (
            Value::Null,
            "serviceDb provider config must be a JSON object",
        ),
        (
            json!({}),
            "serviceDb provider config field mongoUrl is required",
        ),
        (
            json!({ "mongoUrl": 42 }),
            "serviceDb provider config field mongoUrl must be a string",
        ),
        (
            json!({ "mongoUrl": "" }),
            "serviceDb provider config field mongoUrl must be a non-empty string",
        ),
        (
            json!({
                "mongoUrl": inert_mongo_url("provider-unknown"),
                "retryWrites": false
            }),
            "serviceDb provider config field retryWrites is not supported",
        ),
    ] {
        let error = match MongoServiceDbProviderFactory.build(provider_input(config)) {
            Ok(_) => panic!("invalid provider config should fail"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }
}

fn provider_input(config: Value) -> DbProviderBuildInput {
    DbProviderBuildInput {
        service_id: service_id("provider"),
        config: DbProviderConfig::opaque(config),
        runtime_program_db: Vec::new(),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIdFixture {
    schema_version: u32,
    encoding: String,
    max_bytes: usize,
    valid: Vec<PublicationIdCase>,
    invalid: Vec<InvalidPublicationIdCase>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicationIdCase {
    canonical_id: String,
    runtime_target_component: String,
    applies_to: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvalidPublicationIdCase {
    applies_to: Vec<String>,
}

fn runtime_publication_id_fixture() -> PublicationIdFixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime crate should live under the skiff repository root")
        .join("cross-system-fixtures/publication-id-cases.json");
    let text = std::fs::read_to_string(&path).expect("publication id fixture should be readable");
    let fixture: PublicationIdFixture =
        serde_json::from_str(&text).expect("publication id fixture should parse");
    assert_eq!(fixture.schema_version, 1);
    assert_eq!(fixture.encoding, "url-like-with-storage-safe-projection");
    assert_eq!(fixture.max_bytes, 63);
    assert_publication_id_fixture_applies_to(&fixture);
    fixture
}

fn assert_publication_id_fixture_applies_to(fixture: &PublicationIdFixture) {
    for applies_to in fixture
        .valid
        .iter()
        .map(|case| &case.applies_to)
        .chain(fixture.invalid.iter().map(|case| &case.applies_to))
    {
        assert!(applies_to.len() >= 2);
        for (index, system) in applies_to.iter().enumerate() {
            assert!(
                matches!(system.as_str(), "compiler" | "runtime" | "router"),
                "invalid appliesTo system {system:?}"
            );
            assert!(
                !applies_to[..index].contains(system),
                "repeated appliesTo system {system:?}"
            );
        }
    }
}

#[test]
fn object_metadata_accepts_retention_field() {
    for retention in [Value::Null, json!({ "amount": 30, "unit": "days" })] {
        ServiceDbRuntime::new(
            "example.com/test".to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &object_metadata_with_retention(retention),
        )
        .expect("object DB metadata should allow retention");
    }
}

#[test]
fn service_db_runtime_projects_service_id_to_database_name() {
    let fixture = runtime_publication_id_fixture();
    for case in fixture
        .valid
        .iter()
        .filter(|case| case.applies_to.iter().any(|system| system == "runtime"))
    {
        let runtime = ServiceDbRuntime::new(
            case.canonical_id.clone(),
            "mongodb://127.0.0.1:27017".to_string(),
            &[],
        )
        .expect("service DB runtime should project service id to storage-safe database name");

        assert_eq!(runtime.database_name, case.runtime_target_component);
    }
}

#[test]
fn service_db_runtime_rejects_mongo_unsafe_database_names() {
    for service_id in [
        "",
        "std",
        "skiff.run/std$",
        "skiff.run/std value",
        "skiff~run~~std",
        "admin",
        "local",
        "config",
    ] {
        let error = ServiceDbRuntime::new(
            service_id.to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &[],
        )
        .err()
        .expect("unsafe service database name should be rejected");

        assert!(
            error.to_string().contains("service id"),
            "{service_id}: {error}"
        );
    }
}

#[tokio::test]
async fn service_db_runtime_reuses_client_cell_for_exact_mongo_url() {
    let mongo_url = inert_mongo_url("shared_cell");
    let first = ServiceDbRuntime::new(service_id("shared_a"), mongo_url.clone(), &[])
        .expect("first service DB runtime should build");
    let second = ServiceDbRuntime::new(service_id("shared_b"), mongo_url, &[])
        .expect("second service DB runtime should build");

    assert!(
        Arc::ptr_eq(&first.client, &second.client),
        "same exact mongoUrl should share the Mongo client cell"
    );
    assert!(
        first.client.get().is_none(),
        "shared cell should still initialize lazily"
    );

    let _first_client = first
        .client()
        .await
        .expect("inert Mongo URL should still build a client handle");
    assert!(
        second.client.get().is_some(),
        "initializing one runtime should initialize the shared cell for the other"
    );
    let _second_client = second
        .client()
        .await
        .expect("second runtime should clone the shared client handle");
}

#[test]
fn service_db_runtime_does_not_share_client_cell_for_different_mongo_urls() {
    let first = ServiceDbRuntime::new(service_id("distinct_a"), inert_mongo_url("distinct_a"), &[])
        .expect("first service DB runtime should build");
    let second =
        ServiceDbRuntime::new(service_id("distinct_b"), inert_mongo_url("distinct_b"), &[])
            .expect("second service DB runtime should build");

    assert!(
        !Arc::ptr_eq(&first.client, &second.client),
        "different exact mongoUrl values should not share the Mongo client cell"
    );
}

#[test]
fn service_db_client_cache_drops_dead_cells_and_urls() {
    let stale_url = inert_mongo_url("drop_stale");
    let stale_cell = {
        let first = ServiceDbRuntime::new(service_id("drop_a"), stale_url.clone(), &[])
            .expect("first service DB runtime should build");
        let second = ServiceDbRuntime::new(service_id("drop_b"), stale_url.clone(), &[])
            .expect("second service DB runtime should build");

        assert!(
            Arc::ptr_eq(&first.client, &second.client),
            "same mongoUrl should share the cell while runtimes are live"
        );
        Arc::downgrade(&first.client)
    };
    assert!(
        stale_cell.upgrade().is_none(),
        "global cache must not keep dropped runtime cells alive"
    );

    let live_url = inert_mongo_url("drop_live");
    let live_runtime = ServiceDbRuntime::new(service_id("drop_live"), live_url.clone(), &[])
        .expect("live service DB runtime should build");

    let cells = SERVICE_DB_CLIENT_CELLS
        .get()
        .expect("service DB client cache should be initialized");
    let cells = cells
        .lock()
        .expect("service DB client cache lock should not be poisoned");
    assert!(
        !cells.contains_key(&stale_url),
        "accessing the cache should remove dead URL entries"
    );
    let live_cell = cells
        .get(&live_url)
        .and_then(std::sync::Weak::upgrade)
        .expect("live runtime cell should remain upgradeable in the cache");
    assert!(
        Arc::ptr_eq(&live_runtime.client, &live_cell),
        "global cache should point at the live runtime cell"
    );
}

#[test]
fn service_db_runtime_keeps_database_name_and_metadata_isolated_when_client_cell_is_shared() {
    let mongo_url = inert_mongo_url("isolated_runtime");
    let account = ServiceDbRuntime::new(
        service_id("account"),
        mongo_url.clone(),
        &object_metadata_for_type("AccountOnly"),
    )
    .expect("account service DB runtime should build");
    let registry = ServiceDbRuntime::new(
        service_id("registry"),
        mongo_url,
        &object_metadata_for_type("RegistryOnly"),
    )
    .expect("registry service DB runtime should build");

    assert!(
        Arc::ptr_eq(&account.client, &registry.client),
        "same mongoUrl should share only the Mongo client cell"
    );
    assert_ne!(
        account.database_name, registry.database_name,
        "different service ids must keep separate Mongo database names"
    );
    account
        .metadata
        .collection_for_type("AccountOnly")
        .expect("account metadata should remain on account runtime");
    assert!(
        account
            .metadata
            .collection_for_type("RegistryOnly")
            .is_err(),
        "registry metadata must not leak into account runtime"
    );
    registry
        .metadata
        .collection_for_type("RegistryOnly")
        .expect("registry metadata should remain on registry runtime");
    assert!(
        registry
            .metadata
            .collection_for_type("AccountOnly")
            .is_err(),
        "account metadata must not leak into registry runtime"
    );
}

#[tokio::test]
async fn service_db_client_cache_does_not_store_failed_initialization() {
    let invalid_url = "http://127.0.0.1:1".to_string();
    let first = ServiceDbRuntime::new(service_id("invalid_a"), invalid_url.clone(), &[])
        .expect("first service DB runtime should build before connecting");
    let second = ServiceDbRuntime::new(service_id("invalid_b"), invalid_url, &[])
        .expect("second service DB runtime should build before connecting");

    assert!(
        Arc::ptr_eq(&first.client, &second.client),
        "same invalid mongoUrl should still address the same retryable cell"
    );
    first
        .client()
        .await
        .expect_err("invalid Mongo URL should fail client initialization");
    assert!(
        first.client.get().is_none(),
        "failed initialization must not fill the shared cell"
    );
    second
        .client()
        .await
        .expect_err("same invalid Mongo URL should retry and fail again");
    assert!(
        second.client.get().is_none(),
        "failed retry must not permanently poison the shared cell"
    );
}

#[tokio::test]
async fn service_db_client_options_disable_retryable_writes_by_default() {
    let options =
        service_db_client_options("mongodb://127.0.0.1:8500/?directConnection=true&appName=skiff")
            .await
            .expect("service DB Mongo options should parse");

    assert_eq!(options.retry_writes, Some(false));
    assert_eq!(options.direct_connection, Some(true));
    assert_eq!(options.app_name.as_deref(), Some("skiff"));
}

#[tokio::test]
async fn service_db_client_options_ignore_replica_set_for_direct_connection() {
    let options = service_db_client_options(
        "mongodb://127.0.0.1:8500/?directConnection=true&replicaSet=rs0&retryWrites=false",
    )
    .await
    .expect("service DB Mongo options should parse");

    assert_eq!(options.direct_connection, Some(true));
    assert_eq!(options.repl_set_name, None);
    assert_eq!(options.retry_writes, Some(false));
}

#[tokio::test]
async fn service_db_client_options_override_retryable_writes() {
    let options =
        service_db_client_options("mongodb://127.0.0.1:8500/?retryWrites=true&w=majority")
            .await
            .expect("service DB Mongo options should parse");

    assert_eq!(options.retry_writes, Some(false));
    assert!(options.write_concern.is_some());
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

    assert_eq!(browser_session.collection_name, "BrowserSession");
    assert_eq!(track_event.collection_name, "TrackEvent");
}

#[test]
fn object_metadata_uses_final_collection_name_from_service_unit_db() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&db_metadata(json!([
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
            .collection_for_type("Session")
            .expect("Session metadata should resolve")
            .collection_name,
        "registry_session"
    );
    assert_eq!(
        metadata
            .collection_for_type("httpSession.db.Session")
            .expect("canonical Session metadata should resolve")
            .collection_name,
        "registry_session"
    );
}

#[test]
fn object_metadata_rejects_reserved_skiff_collection_name() {
    let error = ServiceDbRuntime::new(
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &db_metadata(json!([
            {
                "kind": "object",
                "typeName": "File",
                "collectionName": "_skiff_file",
                "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
                "fields": [],
                "indexes": []
            }
        ])),
    )
    .err()
    .expect("reserved system collection should be rejected");

    assert!(
        error
            .to_string()
            .contains("reserved _skiff_ system namespace"),
        "{error}"
    );
}

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
fn object_metadata_rejects_reserved_legacy_skiff_type_key_and_field_names() {
    let key_error = ServiceDbRuntime::new(
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &db_metadata(json!([
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
        "example.com/test".to_string(),
        "mongodb://127.0.0.1:27017".to_string(),
        &db_metadata(json!([
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
    let metadata = ServiceDbMetadata::from_runtime_program_db(&db_metadata(json!([
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
        .collection_for_type("Interaction")
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
    let metadata = ServiceDbMetadata::from_runtime_program_db(&db_metadata(json!([
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
        .collection_for_type("Thread")
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
fn metadata_lookup_supports_canonical_type_without_overwriting_bare_name() {
    let metadata = ServiceDbMetadata::from_runtime_program_db(&db_metadata(json!([
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
            .collection_for_type("internal.models.Thread")
            .expect("canonical models Thread should resolve")
            .collection_name,
        "threads_a"
    );
    assert_eq!(
        metadata
            .collection_for_type("internal.archive.Thread")
            .expect("canonical archive Thread should resolve")
            .collection_name,
        "threads_b"
    );

    let error = metadata
        .collection_for_type("Thread")
        .expect_err("bare duplicate Thread lookup should be ambiguous");
    assert!(
        error
            .to_string()
            .contains("runtime program db metadata has ambiguous type Thread"),
        "{error}"
    );
}

#[test]
fn page_order_maps_business_key_to_mongo_id() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
    let options = ServiceDbFindOptions::default();

    assert_eq!(binding.page_sort_document(&options).unwrap(), None);
}

#[test]
fn page_order_uses_only_explicit_order_fields() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

    assert_eq!(
        binding
            .query_filter(db_query(json!({ "id": "thread-1" })))
            .unwrap(),
        doc! { "_id": "thread-1" }
    );
}

#[test]
fn key_selector_does_not_require_sort() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

    let (filter, sort) = binding
        .selector_filter_sort(DbOneSelector::Key(db_key(json!("thread-1"))))
        .unwrap();

    assert_eq!(filter, doc! { "_id": "thread-1" });
    assert_eq!(sort, None);
}

#[test]
fn upsert_insert_value_uses_selector_key() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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
            Some(&db_key(json!("thread-1"))),
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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
fn guarded_filter_fences_held_lease_key() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
    let hold = DbLeaseHold {
        type_name: "Thread".to_string(),
        key: db_key(json!("thread-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };

    let filter = guarded_filter(&binding, doc! { "title": "Hello" }, &[hold], 1000)
        .expect("guarded filter should build");

    assert_eq!(
        filter,
        doc! {
            "$and": [
                { "title": "Hello" },
                {
                    "$or": [
                        { "_id": { "$ne": "thread-1" } },
                        {
                            "_id": "thread-1",
                            "$or": [
                                { "__skiffLeases.writer.maxExpiresAtMs": Bson::Null },
                                { "__skiffLeases.writer.maxExpiresAtMs": { "$exists": false } },
                                { "__skiffLeases.writer.maxExpiresAtMs": { "$gt": 1000_i64 } }
                            ],
                            "__skiffLeases.writer.token": "token-1",
                            "__skiffLeases.writer.expiresAtMs": { "$gt": 1000_i64 }
                        }
                    ]
                }
            ]
        }
    );
}

#[test]
fn lease_live_key_filter_requires_ttl_and_max_to_be_live() {
    let filter = lease_live_key_filter(
        "writer",
        Bson::String("thread-1".to_string()),
        "token-1",
        1000,
    );

    assert_eq!(
        filter,
        doc! {
            "_id": "thread-1",
            "$or": [
                { "__skiffLeases.writer.maxExpiresAtMs": Bson::Null },
                { "__skiffLeases.writer.maxExpiresAtMs": { "$exists": false } },
                { "__skiffLeases.writer.maxExpiresAtMs": { "$gt": 1000_i64 } }
            ],
            "__skiffLeases.writer.token": "token-1",
            "__skiffLeases.writer.expiresAtMs": { "$gt": 1000_i64 }
        }
    );
}

#[test]
fn lease_claim_expires_at_clamps_initial_ttl_to_max_deadline() {
    assert_eq!(lease_claim_expires_at_ms(1_000, 60_000, None), 61_000);
    assert_eq!(
        lease_claim_expires_at_ms(1_000, 60_000, Some(31_000)),
        31_000
    );
}

#[test]
fn guarded_filter_ignores_other_type_leases() {
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
    let hold = DbLeaseHold {
        type_name: "Other".to_string(),
        key: db_key(json!("thread-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };

    let filter = guarded_filter(&binding, doc! { "title": "Hello" }, &[hold], 1000)
        .expect("guarded filter should build");

    assert_eq!(filter, doc! { "title": "Hello" });
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

    assert_eq!(hooks.encode_calls.get(), 1);
    let Some(Bson::Binary(binary)) = document.get("provider") else {
        panic!("provider should be stored as recoverable-envelope BSON binary");
    };
    assert_eq!(binary.subtype, BinarySubtype::Generic);
    assert!(!binary.bytes.is_empty());
    assert!(
        root_store.roots.is_empty(),
        "LocalConcrete recoverable self nodes do not create artifact retention roots"
    );

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

    assert_eq!(hooks.restore_calls.get(), 1);
    assert_eq!(hooks.conformance_calls.get(), 1);
    assert_eq!(hooks.table_calls.get(), 1);
    assert_decoded_provider_runtime_value(&decoded, &read_heap, "binding-1", "anthropic");
}

#[test]
fn recoverable_runtime_context_reexport_preserves_write_contract_fields() {
    let hooks = Arc::new(ThreadSafeTestDbBehaviorHooks::default());
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

#[tokio::test]
async fn service_db_runtime_create_and_find_runtime_roundtrips_local_interface() {
    let service_id = format!(
        "skiff.run/p5dbprodtest-{}-{}",
        std::process::id(),
        service_db_now_ms()
    );
    let runtime = ServiceDbRuntime::new(
        service_id,
        "mongodb://127.0.0.1:27017/?directConnection=true".to_string(),
        &recoverable_provider_metadata_value(),
    )
    .expect("service DB runtime should build");
    let database_name = runtime.database_name_for_test();
    let client = runtime
        .client()
        .await
        .expect("local Mongo service DB should be available for production-path DB test");
    client
        .database(&database_name)
        .drop()
        .await
        .expect("test database should drop before run");

    let mut heap = RequestHeap::default();
    let provider = local_provider_runtime_value(&mut heap, "openai");
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("binding-1".to_string())),
            ("provider", provider),
        ],
    );
    let hooks = Arc::new(ThreadSafeTestDbBehaviorHooks::default());
    let context = production_runtime_context(hooks.clone());

    runtime
        .create_runtime("ProviderBinding", &value, &heap, context.clone(), None)
        .await
        .expect("production service DB runtime create should encode local interface");

    let mut read_heap = RequestHeap::default();
    let read = runtime
        .find_one_by_key_runtime(
            "ProviderBinding",
            db_key(json!("binding-1")),
            None,
            &mut read_heap,
            context.clone(),
            None,
        )
        .await
        .expect("production service DB runtime read should decode local interface")
        .expect("created provider binding should exist");

    assert_decoded_provider_runtime_value(&read, &read_heap, "binding-1", "openai");

    let plain_find_many_error = runtime
        .find_many_page(
            "ProviderBinding",
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            None,
        )
        .await
        .expect_err("plain find many should not decode behavior recoverable envelope fields");
    assert!(
        plain_find_many_error
            .to_string()
            .contains("recoverable-envelope DB field decode failed"),
        "{plain_find_many_error}"
    );

    let mut page_heap = RequestHeap::default();
    let page = runtime
        .find_many_page_runtime(
            "ProviderBinding",
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            &mut page_heap,
            context.clone(),
            None,
        )
        .await
        .expect("production service DB runtime find many should decode local interface");
    assert_eq!(page.len(), 1);
    assert_decoded_provider_runtime_value(&page[0], &page_heap, "binding-1", "openai");

    let replacement_provider = local_provider_runtime_value(&mut heap, "anthropic");
    let replacement_value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("binding-1".to_string())),
            ("provider", replacement_provider),
        ],
    );
    let replaced = runtime
        .replace_one_runtime(
            "ProviderBinding",
            DbOneSelector::Key(db_key(json!("binding-1"))),
            &replacement_value,
            &mut heap,
            context.clone(),
            &[],
            None,
        )
        .await
        .expect("production service DB runtime replace should encode local interface")
        .expect("created provider binding should be replaced");

    assert_decoded_provider_runtime_value(&replaced, &heap, "binding-1", "anthropic");

    let mut reread_heap = RequestHeap::default();
    let reread = runtime
        .find_one_by_key_runtime(
            "ProviderBinding",
            db_key(json!("binding-1")),
            None,
            &mut reread_heap,
            context,
            None,
        )
        .await
        .expect("production service DB runtime read should decode replaced local interface")
        .expect("replaced provider binding should exist");

    assert_decoded_provider_runtime_value(&reread, &reread_heap, "binding-1", "anthropic");
    assert!(hooks.restore_calls() >= 1);
    client
        .database(&database_name)
        .drop()
        .await
        .expect("test database should drop after run");
}

#[test]
fn recoverable_envelope_runtime_field_roundtrips_remote_carrier_without_local_encode_hook() {
    let binding = recoverable_provider_metadata();
    let mut heap = RequestHeap::default();
    let provider = remote_provider_runtime_value(&mut heap);
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
        retention_expires_at_epoch_millis: None,
    };

    let document = binding
        .document_from_runtime_business_value(&value, &heap, Some(&mut write_context))
        .expect("remote interface carrier should encode as an owner-internal recoverable envelope");

    assert_eq!(hooks.encode_calls.get(), 0);
    let Some(Bson::Binary(binary)) = document.get("provider") else {
        panic!("provider should be stored as recoverable-envelope BSON binary");
    };
    assert_eq!(binary.subtype, BinarySubtype::Generic);
    assert!(!binary.bytes.is_empty());
    assert!(
        root_store.roots.is_empty(),
        "remote public-instance carriers do not create artifact retention roots"
    );

    let mut read_heap = RequestHeap::default();
    let read_context = DbRecoverableRuntimeReadContext {
        behavior_hooks: &hooks,
        boundary_context: None,
        recoverable_expected_override: Some(&expected),
        recoverable_expected_overrides: None,
    };
    let decoded = binding
        .runtime_business_value_from_document(document, &mut read_heap, Some(&read_context))
        .expect("remote interface carrier should decode from DB recoverable envelope");

    assert_eq!(hooks.encode_calls.get(), 0);
    assert_eq!(hooks.restore_calls.get(), 0);
    assert_eq!(hooks.conformance_calls.get(), 0);
    assert_eq!(hooks.table_calls.get(), 0);
    assert_eq!(hooks.remote_table_calls.get(), 2);
    assert_decoded_remote_provider_runtime_value(&decoded, &read_heap, "binding-1");
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
    assert!(root_store.roots.is_empty());

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
    let binding = DbCollectionMetadata::from_ir(&object_metadata_with_retention(Value::Null)[0], 0)
        .expect("object metadata should parse");
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

    let error = DbCollectionMetadata::from_ir(
        &recoverable_envelope_metadata_value(json!([
            {
                "name": "settings_filter",
                "fields": [
                    {
                        "field": { "text": "title", "segments": ["title"] },
                        "direction": "asc"
                    }
                ],
                "where": { "settings.mode": "dark" }
            }
        ]))[0],
        0,
    )
    .expect_err("index predicate on envelope field should be rejected");
    assert_recoverable_opaque_db_error(&error, "predicate");
}

#[test]
fn db_change_update_document_uses_last_value_for_duplicate_operator_fields() {
    let binding = DbCollectionMetadata::from_ir(&object_metadata_with_retention(Value::Null)[0], 0)
        .expect("object metadata should parse");
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");
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
    let metadata = object_metadata_with_retention(Value::Null);
    let binding =
        DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse");

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

    let binding = DbCollectionMetadata::from_ir(&object_metadata_with_retention(Value::Null)[0], 0)
        .expect("object metadata should parse");

    let mut change = ServiceDbChange::new();
    change.unset("title.__skiffBusiness");
    let error = binding
        .validated_change_update("Thread", change)
        .unwrap_err();
    assert_reserved_skiff_metadata_error(&error);
}

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

fn object_metadata_with_retention(retention: Value) -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } }
            ],
            "indexes": [],
            "retention": retention
        }
    ]))
}

fn object_metadata_for_type(type_name: &str) -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": type_name,
            "collectionName": type_name,
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } }
            ],
            "indexes": []
        }
    ]))
}

fn inert_mongo_url(label: &str) -> String {
    format!(
        "mongodb://127.0.0.1:1/?directConnection=true&appName=skiff-service-db-{label}-{}",
        uuid::Uuid::new_v4().simple()
    )
}

fn service_id(label: &str) -> String {
    format!("example.com/{label}_{}", uuid::Uuid::new_v4().simple())
}

fn recoverable_envelope_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_envelope_metadata_value(json!([]))[0], 0)
        .expect("recoverable-envelope metadata should parse")
}

fn recoverable_nullable_envelope_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_nullable_envelope_metadata_value()[0], 0)
        .expect("nullable recoverable-envelope metadata should parse")
}

fn recoverable_envelope_metadata_value(indexes: Value) -> Vec<DbMetadataIr> {
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

fn recoverable_nullable_envelope_metadata_value() -> Vec<DbMetadataIr> {
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

fn recoverable_provider_metadata() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir(&recoverable_provider_metadata_value()[0], 0)
        .expect("recoverable provider metadata should parse")
}

fn recoverable_provider_metadata_value() -> Vec<DbMetadataIr> {
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

const TEST_PROVIDER_INTERFACE: &str = "pkg.ToolProvider";
const TEST_PROVIDER_PROJECTION: &str = "projection:pkg.ToolProvider:pkg.StaticProvider";
const TEST_PROVIDER_METHOD: &str = "method:pkg.ToolProvider:complete";
const TEST_PROVIDER_IMPL: &str = "pkg.StaticProvider";
const TEST_SERVICE_ARTIFACT: &str = "svc/llm";
const TEST_SERVICE_BUILD: &str = "build-provider-a";

fn test_provider_expected_plan() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan::any_interface(
        "any ToolProvider",
        TEST_PROVIDER_INTERFACE,
        TEST_PROVIDER_PROJECTION,
    )
}

fn recoverable_settings_expected(
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

fn string_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "string".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::String,
    }
}

fn nullable_string_expected() -> RuntimeRecoverableExpectedTypePlan {
    RuntimeRecoverableExpectedTypePlan {
        label: "string?".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Nullable {
            inner: Box::new(string_expected()),
        },
    }
}

fn runtime_settings_object<const N: usize>(
    fields: [(&str, RuntimeValue); N],
) -> [(&str, RuntimeValue); N] {
    fields
}

fn recoverable_settings_document_with_expected<const N: usize>(
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

fn recoverable_settings_runtime_read_with_expected(
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

fn runtime_object<const N: usize>(
    heap: &mut RequestHeap,
    fields: [(&str, RuntimeValue); N],
) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from(
            fields.map(|(field, value)| (field.to_string(), value)),
        )))
        .expect("object should allocate"),
    )
}

fn local_provider_runtime_value(heap: &mut RequestHeap, provider_name: &str) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            TEST_PROVIDER_INTERFACE.to_string(),
            InterfaceCarrier::Local {
                concrete_type: TEST_PROVIDER_IMPL.to_string(),
                method_table: test_provider_method_table(),
                payload: RuntimeValue::String(provider_name.to_string()),
            },
        ))
        .expect("local provider interface should allocate"),
    )
}

fn remote_provider_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_interface(InterfaceValue::new(
            TEST_PROVIDER_INTERFACE.to_string(),
            InterfaceCarrier::Remote {
                dependency_ref: "skiff.run/remote-provider".to_string(),
                public_instance_key: "provider#1".to_string(),
                operations: test_remote_provider_operation_table(),
            },
        ))
        .expect("remote provider interface should allocate"),
    )
}

fn test_remote_provider_operation_table() -> RemoteOperationTable {
    RemoteOperationTable::new(
        "remote:provider".to_string(),
        TEST_PROVIDER_INTERFACE.to_string(),
        Vec::new(),
    )
}

fn test_provider_method_table() -> InterfaceMethodTable {
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

fn provider_self_node(provider_name: &str) -> RecoverableNode {
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

fn assert_decoded_provider_runtime_value(
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
    assert_eq!(concrete_type, TEST_PROVIDER_IMPL);
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

fn assert_decoded_remote_provider_runtime_value(
    value: &RuntimeValue,
    heap: &RequestHeap,
    expected_id: &str,
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
    let InterfaceCarrier::Remote {
        dependency_ref,
        public_instance_key,
        operations,
    } = provider.carrier()
    else {
        panic!("provider should decode as a remote carrier");
    };
    assert_eq!(dependency_ref, "skiff.run/remote-provider");
    assert_eq!(public_instance_key, "provider#1");
    assert_eq!(operations.id(), "remote:provider");
    assert_eq!(operations.interface_abi_id(), TEST_PROVIDER_INTERFACE);
    assert!(operations.slots().is_empty());
}

struct TestDbBehaviorHooks {
    encode_calls: Cell<usize>,
    restore_calls: Cell<usize>,
    conformance_calls: Cell<usize>,
    table_calls: Cell<usize>,
    remote_table_calls: Cell<usize>,
    table_projection_identity: RefCell<String>,
}

impl Default for TestDbBehaviorHooks {
    fn default() -> Self {
        Self {
            encode_calls: Cell::new(0),
            restore_calls: Cell::new(0),
            conformance_calls: Cell::new(0),
            table_calls: Cell::new(0),
            remote_table_calls: Cell::new(0),
            table_projection_identity: RefCell::new(TEST_PROVIDER_PROJECTION.to_string()),
        }
    }
}

impl RecoverableBehaviorHooks for TestDbBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        _heap: &RequestHeap,
    ) -> BoundaryResult<Option<RecoverableEncodedLocalInterfaceSelf>> {
        self.encode_calls.set(self.encode_calls.get() + 1);
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
        self.restore_calls.set(self.restore_calls.get() + 1);
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
            payload: RuntimeValue::String(provider_name),
        }))
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> BoundaryResult<bool> {
        self.conformance_calls.set(self.conformance_calls.get() + 1);
        Ok(request.concrete_type_identity == TEST_PROVIDER_IMPL
            && request.interface_identity == TEST_PROVIDER_INTERFACE)
    }

    fn rebuild_local_interface_method_table(
        &self,
        request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> BoundaryResult<Option<InterfaceMethodTable>> {
        self.table_calls.set(self.table_calls.get() + 1);
        if request.method_projection_identity != *self.table_projection_identity.borrow() {
            return Ok(None);
        }
        Ok(Some(test_provider_method_table()))
    }

    fn rebuild_remote_interface_operation_table(
        &self,
        request: RecoverableRemoteInterfaceCarrierRequest<'_>,
    ) -> BoundaryResult<Option<RemoteOperationTable>> {
        self.remote_table_calls
            .set(self.remote_table_calls.get() + 1);
        if request.interface_identity != TEST_PROVIDER_INTERFACE
            || request.carrier.dependency_ref != "skiff.run/remote-provider"
            || request.carrier.public_instance_key != "provider#1"
            || request.carrier.operations.id != "remote:provider"
            || request.carrier.operations.interface_abi_id != TEST_PROVIDER_INTERFACE
            || !request.carrier.operations.slots.is_empty()
        {
            return Ok(None);
        }
        Ok(Some(test_remote_provider_operation_table()))
    }
}

#[derive(Default)]
struct ThreadSafeTestDbBehaviorHooks {
    inner: Mutex<TestDbBehaviorHooks>,
}

impl ThreadSafeTestDbBehaviorHooks {
    fn restore_calls(&self) -> usize {
        self.inner
            .lock()
            .expect("test hook mutex")
            .restore_calls
            .get()
    }
}

impl RecoverableBehaviorHooks for ThreadSafeTestDbBehaviorHooks {
    fn encode_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceEncodeRequest<'_>,
        heap: &RequestHeap,
    ) -> BoundaryResult<Option<RecoverableEncodedLocalInterfaceSelf>> {
        let inner = self.inner.lock().expect("test hook mutex");
        RecoverableBehaviorHooks::encode_local_interface_self(&*inner, request, heap)
    }

    fn restore_local_interface_self(
        &self,
        request: RecoverableLocalInterfaceRestoreRequest<'_>,
        heap: &mut RequestHeap,
    ) -> BoundaryResult<Option<RecoverableRestoredLocalInterfaceSelf>> {
        let inner = self.inner.lock().expect("test hook mutex");
        RecoverableBehaviorHooks::restore_local_interface_self(&*inner, request, heap)
    }

    fn concrete_type_conforms_to_interface(
        &self,
        request: RecoverableInterfaceConformanceRequest<'_>,
    ) -> BoundaryResult<bool> {
        let inner = self.inner.lock().expect("test hook mutex");
        RecoverableBehaviorHooks::concrete_type_conforms_to_interface(&*inner, request)
    }

    fn rebuild_local_interface_method_table(
        &self,
        request: RecoverableInterfaceMethodTableRequest<'_>,
    ) -> BoundaryResult<Option<InterfaceMethodTable>> {
        let inner = self.inner.lock().expect("test hook mutex");
        RecoverableBehaviorHooks::rebuild_local_interface_method_table(&*inner, request)
    }

    fn rebuild_remote_interface_operation_table(
        &self,
        request: RecoverableRemoteInterfaceCarrierRequest<'_>,
    ) -> BoundaryResult<Option<RemoteOperationTable>> {
        let inner = self.inner.lock().expect("test hook mutex");
        RecoverableBehaviorHooks::rebuild_remote_interface_operation_table(&*inner, request)
    }
}

fn production_runtime_context(
    hooks: Arc<ThreadSafeTestDbBehaviorHooks>,
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
struct TestDbArtifactStore {
    available: HashSet<(String, String)>,
}

impl TestDbArtifactStore {
    fn with_available(mut self, artifact_identity: &str, build_id: &str) -> Self {
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
struct TestDbRootStore {
    roots: Vec<RecoverableArtifactRetentionRoot>,
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

fn field_path_with_text(text: &str) -> FieldPath {
    field_path_with_text_and_segments(text, &[text])
}

fn field_path_with_text_and_segments(text: &str, segments: &[&str]) -> FieldPath {
    FieldPath {
        text: text.to_string(),
        segments: segments.iter().map(|segment| segment.to_string()).collect(),
    }
}

fn assert_reserved_legacy_skiff_type_error(error: &ServiceDbError) {
    assert_reserved_skiff_metadata_error(error);
}

fn assert_reserved_skiff_metadata_error(error: &ServiceDbError) {
    assert!(
        error.to_string().contains("reserved Skiff metadata"),
        "{error}"
    );
}

fn assert_recoverable_opaque_db_error(error: &ServiceDbError, operation: &str) {
    let message = error.to_string();
    assert!(
        message.contains("recoverable-envelope DB field settings is opaque")
            && message.contains(operation),
        "{error}"
    );
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
