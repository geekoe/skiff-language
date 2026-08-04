use super::{
    mapping_id, DocumentSanitizer, StorageEndpoint, ValidatedCollectionMapping,
    ValidatedMigrationPlan,
};
use mongodb::bson::doc;

fn endpoint(database: &str, collection: &str) -> StorageEndpoint {
    StorageEndpoint {
        profile: "dev".to_owned(),
        service_id: "example.test/service".to_owned(),
        database: database.to_owned(),
        package_id: "example.test/package".to_owned(),
        logical_collection: "Item".to_owned(),
        physical_collection: collection.to_owned(),
    }
}

#[test]
fn mapping_identity_is_stable_and_bound_to_both_namespaces() {
    let source = endpoint("old", "items");
    let target = endpoint("new", "_skiff_c1_items");
    let first = mapping_id(&source, &target);
    assert_eq!(first.len(), 34);
    assert_eq!(first, mapping_id(&source, &target));
    assert_ne!(
        first,
        mapping_id(&source, &endpoint("other", "_skiff_c1_items"))
    );
}

#[test]
fn tool_provider_sanitizer_removes_only_live_connection_state() {
    let mapping = ValidatedCollectionMapping {
        mapping_id: "m-00000000000000000000000000000000".to_owned(),
        source: endpoint("old", "ToolProvider"),
        target: endpoint("new", "_skiff_c1_tool_provider"),
        source_exists: true,
        expected_source_count: 1,
        encrypted_fields: Vec::new(),
        target_indexes: Vec::new(),
        sanitizer: DocumentSanitizer::ToolProvider,
    };
    let mut document = doc! {
        "_id": "provider",
        "presence": "online",
        "actorSubjectId": "actor",
        "activeConnectionId": "connection",
        "lastSeenAt": "now",
        "updatedAt": "business-time",
        "metadata": {
            "hostIdHash": "stable",
            "currentDirectory": "/tmp",
            "capabilities": ["tool"],
            "setting": true
        }
    };
    mapping.sanitize(&mut document).expect("sanitize");
    assert_eq!(document.get_str("presence"), Ok("offline"));
    assert!(!document.contains_key("actorSubjectId"));
    assert!(!document.contains_key("activeConnectionId"));
    assert!(!document.contains_key("lastSeenAt"));
    assert_eq!(document.get_str("updatedAt"), Ok("business-time"));
    assert_eq!(
        document.get_document("metadata").expect("metadata").clone(),
        doc! { "hostIdHash": "stable", "setting": true }
    );
}

#[test]
#[ignore = "requires explicit audited filtered receipt paths"]
fn audited_filtered_receipts_build_an_exact_plan() {
    let allowlist_path =
        std::env::var("SKIFF_TEST_DB_MIGRATION_ALLOWLIST").expect("allowlist path");
    let sanitization_path =
        std::env::var("SKIFF_TEST_DB_MIGRATION_SANITIZATION").expect("sanitization path");
    let allowlist = std::fs::read(&allowlist_path).expect("allowlist");
    let sanitization = std::fs::read(sanitization_path).expect("sanitization");
    let file_name = std::path::Path::new(&allowlist_path)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("allowlist file name");
    let plan = ValidatedMigrationPlan::parse(&allowlist, &sanitization, "dev", file_name)
        .expect("exact audited plan");
    assert_eq!(plan.mappings.len(), 13);
    assert_eq!(
        plan.mappings
            .iter()
            .map(|mapping| mapping.expected_source_count)
            .sum::<u64>(),
        376
    );
    assert_eq!(
        plan.mappings
            .iter()
            .filter(|mapping| !mapping.source_exists)
            .count(),
        5
    );
    assert_eq!(
        plan.mappings
            .iter()
            .map(|mapping| mapping.target_indexes.len())
            .sum::<usize>(),
        19
    );
}
