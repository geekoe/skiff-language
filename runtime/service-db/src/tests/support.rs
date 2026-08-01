#![allow(unused_imports)]

pub(super) use super::super::{
    mapping::{DbRecoverableRuntimeReadContext, DbRecoverableRuntimeWriteContext},
    metadata::{DbCollectionMetadata, ServiceDbMetadata},
    mongo::{
        is_mongo_db_conflict_error, is_mongo_duplicate_key_code, is_mongo_duplicate_key_error,
        is_mongo_write_conflict_code, is_mongo_write_conflict_error,
        mongo_db_conflict_markers_match, update_without_set_on_insert,
    },
    *,
};
pub(super) use crate::{
    DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans, ServiceDbError,
};
pub(super) use mongodb::{
    bson::{doc, spec::BinarySubtype, Bson, DateTime},
    error::{
        BulkWriteError, CommandError, Error as MongoError, ErrorKind as MongoErrorKind,
        InsertManyError, WriteConcernError, WriteError, WriteFailure, TRANSIENT_TRANSACTION_ERROR,
        UNKNOWN_TRANSACTION_COMMIT_RESULT,
    },
};
pub(super) use serde_json::{json, Map, Value};
pub(super) use skiff_artifact_model::{
    DbMetadataIr, FileIrRef, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
};
pub(super) use skiff_runtime_boundary::{
    db as db_boundary,
    recoverable::{
        RecoverableArtifactRetentionRootStore, RecoverableArtifactStore, RecoverableBehaviorHooks,
        RecoverableEncodedLocalInterfaceSelf, RecoverableInterfaceConformanceRequest,
        RecoverableInterfaceMethodTableRequest, RecoverableLocalInterfaceEncodeRequest,
        RecoverableLocalInterfaceRestoreRequest, RecoverableRestoredLocalInterfaceSelf,
    },
    Result as BoundaryResult,
};
pub(super) use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityError, DbCapabilityTarget, DbCapabilityTargetId, DbDocument,
    DbKey, DbOneSelector, DbOrderDirection, DbOrderEntry, DbProviderBuildInput, DbProviderConfig,
    DbProviderFactory, DbProviderTargetMetadata, DbQuery, FieldPath, ServiceDbChange,
    ServiceDbFindOptions,
};
pub(super) use skiff_runtime_model::{
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
        CallbackCapabilityCarrier, HeapNode, InterfaceCarrier, InterfaceMethodSlot,
        InterfaceMethodTable, InterfaceMethodTarget, InterfaceReceiverCallAbi, InterfaceValue,
        RuntimeObject, RuntimeObjectFields, RuntimeValue,
    },
};
pub(super) use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
pub(super) use tokio::sync::Mutex as TokioMutex;
pub(super) fn db_key(value: serde_json::Value) -> DbKey {
    DbKey::new(value)
}

pub(super) fn db_query(value: serde_json::Value) -> DbQuery {
    DbQuery::new(value)
}

pub(super) fn db_doc(value: serde_json::Value) -> DbDocument {
    DbDocument::new(value)
}

pub(super) fn mongo_command_error(code: i32, code_name: &str) -> MongoError {
    let command_error: CommandError = serde_json::from_value(json!({
        "code": code,
        "codeName": code_name,
        "errmsg": format!("Mongo command error {code_name}"),
    }))
    .expect("mongodb CommandError should deserialize");
    MongoErrorKind::Command(command_error).into()
}

pub(super) fn mongo_write_error(code: i32, code_name: &str) -> WriteError {
    serde_json::from_value(json!({
        "code": code,
        "codeName": code_name,
        "errmsg": format!("Mongo write error {code_name}"),
    }))
    .expect("mongodb WriteError should deserialize")
}

pub(super) fn mongo_write_concern_error(code: i32, code_name: &str) -> WriteConcernError {
    serde_json::from_value(json!({
        "code": code,
        "codeName": code_name,
        "errmsg": format!("Mongo write concern error {code_name}"),
        "errInfo": null,
    }))
    .expect("mongodb WriteConcernError should deserialize")
}

pub(super) fn db_metadata(mut value: Value) -> Vec<DbMetadataIr> {
    let entries = value
        .as_array_mut()
        .expect("test db metadata should be an array");
    for entry in entries {
        normalize_db_metadata_entry(entry);
    }
    serde_json::from_value(value).expect("test db metadata should decode as typed IR")
}

pub(super) fn db_metadata_entry(value: Value) -> DbMetadataIr {
    let mut entries = db_metadata(json!([value]));
    entries
        .pop()
        .expect("test db metadata should contain one entry")
}

pub(super) fn provider_metadata(value: Value) -> Vec<DbProviderTargetMetadata> {
    provider_metadata_from_ir(db_metadata(value))
}

pub(super) fn provider_metadata_from_ir(
    entries: Vec<DbMetadataIr>,
) -> Vec<DbProviderTargetMetadata> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, metadata)| {
            let module_path = metadata.module_path.clone();
            let type_name = metadata.type_name.clone();
            DbProviderTargetMetadata {
                target: test_db_target(index, &module_path, &type_name),
                metadata,
            }
        })
        .collect()
}

pub(super) fn test_db_target(
    index: usize,
    module_path: &str,
    type_name: &str,
) -> DbCapabilityTarget {
    test_db_target_for_package(
        index,
        module_path,
        type_name,
        &format!("test.local/provider-{type_name}-{index}"),
        &format!("test-build-{type_name}-{index}"),
    )
}

pub(super) fn test_db_target_for_package(
    index: usize,
    module_path: &str,
    type_name: &str,
    package_id: &str,
    package_build_id: &str,
) -> DbCapabilityTarget {
    DbCapabilityTarget::new(
        DbCapabilityTargetId {
            package_artifact_ref: PackageArtifactRef {
                package_id: package_id.to_string(),
                package_version: "1.0.0".to_string(),
                package_build_id: PackageBuildId::new(package_build_id),
                package_local_abi_identity: PackageLocalAbiIdentity::new(format!(
                    "test-abi-{package_build_id}"
                )),
            },
            file_ir_ref: FileIrRef::new(format!("test-file-{type_name}-{index}"), module_path),
            type_index: index,
        },
        type_name,
    )
}

pub(super) fn normalize_db_metadata_entry(entry: &mut Value) {
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

pub(super) fn normalize_db_key(object: &mut Map<String, Value>) {
    if let Some(key) = object.get_mut("key").and_then(Value::as_object_mut) {
        key.entry("type")
            .or_insert_with(|| json!({ "kind": "builtin", "name": "string" }));
    }
}

pub(super) fn normalize_db_fields(object: &mut Map<String, Value>) {
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

pub(super) fn normalize_db_indexes(object: &mut Map<String, Value>) {
    let indexes = object.entry("indexes").or_insert_with(|| json!([]));
    for index in indexes
        .as_array_mut()
        .expect("test db metadata indexes should be an array")
    {
        if let Some(index) = index.as_object_mut() {
            index.entry("unique").or_insert(Value::Bool(false));
            index.entry("fields").or_insert_with(|| json!([]));
        }
    }
}
pub(super) fn encrypted_metadata(
    key_type: &str,
    field_type: &str,
    indexes: Value,
) -> Vec<DbMetadataIr> {
    db_metadata(json!([{
        "modulePath": "internal.credential",
        "kind": "object",
        "typeName": "Credential",
        "collectionName": "credential",
        "key": { "name": "id", "type": { "kind": "builtin", "name": key_type } },
        "fields": [
            { "name": "apiKey", "type": { "kind": "builtin", "name": field_type }, "storage": "encrypted" },
            { "name": "label", "type": { "kind": "builtin", "name": "string" } }
        ],
        "indexes": indexes
    }]))
}

pub(super) fn encrypted_metadata_with_field_type(field_type: Value) -> Vec<DbMetadataIr> {
    db_metadata(json!([{
        "modulePath": "internal.credential",
        "kind": "object",
        "typeName": "Credential",
        "collectionName": "credential",
        "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
        "fields": [
            { "name": "apiKey", "type": field_type, "storage": "encrypted" },
            { "name": "label", "type": { "kind": "builtin", "name": "string" } }
        ],
        "indexes": []
    }]))
}

pub(super) fn test_encryption_keyring() -> Arc<DbEncryptionKeyring> {
    Arc::new(
        DbEncryptionKeyring::parse_json(
            br#"{"format":"skiff-service-db-keyring-v1","activeKeyId":"test-key","keys":{"test-key":"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="}}"#,
        )
        .expect("test keyring"),
    )
}

pub(super) fn encrypted_binding() -> DbCollectionMetadata {
    DbCollectionMetadata::from_ir_with_encryption(
        &encrypted_metadata("string", "string", json!([]))[0],
        0,
        "example.com/credential",
        "test",
        "example.com/credential",
        Some(test_encryption_keyring().cipher()),
    )
    .expect("encrypted metadata")
}

pub(super) fn object_metadata_with_retention(retention: Value) -> Vec<DbMetadataIr> {
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

pub(super) fn thread_binding() -> DbCollectionMetadata {
    let metadata = object_metadata_with_retention(Value::Null);
    DbCollectionMetadata::from_ir(&metadata[0], 0).expect("object metadata should parse")
}

pub(super) fn object_metadata_for_type(type_name: &str) -> Vec<DbMetadataIr> {
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

pub(super) fn inert_mongo_url(label: &str) -> String {
    format!(
        "mongodb://127.0.0.1:1/?directConnection=true&appName=skiff-service-db-{label}-{}",
        uuid::Uuid::new_v4().simple()
    )
}

pub(super) fn service_id(label: &str) -> String {
    format!("example.com/{label}_{}", uuid::Uuid::new_v4().simple())
}

pub(super) fn test_environment() -> String {
    "test".to_string()
}

pub(super) fn runtime_object<const N: usize>(
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

pub(super) fn field_path_with_text(text: &str) -> FieldPath {
    field_path_with_text_and_segments(text, &[text])
}

pub(super) fn field_path_with_text_and_segments(text: &str, segments: &[&str]) -> FieldPath {
    FieldPath {
        text: text.to_string(),
        segments: segments.iter().map(|segment| segment.to_string()).collect(),
    }
}

pub(super) fn assert_reserved_legacy_skiff_type_error(error: &ServiceDbError) {
    assert_reserved_skiff_metadata_error(error);
}

pub(super) fn assert_reserved_skiff_metadata_error(error: &ServiceDbError) {
    assert!(
        error.to_string().contains("reserved Skiff metadata"),
        "{error}"
    );
}
