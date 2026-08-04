use super::{MigrationReceipt, MigrationStatus};
use crate::migration_tool::engine::{CollectionInventory, CollectionScan};
use crate::migration_tool::model::{
    DocumentSanitizer, StorageEndpoint, ValidatedCollectionMapping,
};

#[test]
fn execution_receipt_resumes_only_the_exact_plan_and_mapping() {
    let mappings = vec![ValidatedCollectionMapping {
        mapping_id: "m-00000000000000000000000000000000".to_string(),
        source: endpoint("old", "providers"),
        target: endpoint("new", "_skiff_c1_target"),
        source_exists: true,
        expected_source_count: 1,
        encrypted_fields: vec!["apiKey".to_string()],
        target_indexes: Vec::new(),
        sanitizer: DocumentSanitizer::None,
    }];
    let mut receipt = MigrationReceipt::new("receipt-v1", "plan-a", "keyring-a", &mappings);
    receipt
        .entry_mut("m-00000000000000000000000000000000")
        .expect("entry")
        .status = MigrationStatus::Staged;
    receipt
        .validate_resume("receipt-v1", "plan-a", "keyring-a", &mappings)
        .expect("exact resume");
    assert!(receipt
        .validate_resume("receipt-v1", "plan-b", "keyring-a", &mappings)
        .is_err());
    assert!(receipt
        .validate_resume("receipt-v1", "plan-a", "keyring-b", &mappings)
        .is_err());
}

#[test]
fn source_and_final_target_index_receipts_are_verified_separately() {
    let mappings = vec![ValidatedCollectionMapping {
        mapping_id: "m-00000000000000000000000000000000".to_string(),
        source: endpoint("old", "providers"),
        target: endpoint("new", "_skiff_c1_target"),
        source_exists: true,
        expected_source_count: 1,
        encrypted_fields: Vec::new(),
        target_indexes: Vec::new(),
        sanitizer: DocumentSanitizer::None,
    }];
    let mut receipt = MigrationReceipt::new("receipt-v2", "plan", "keyring", &mappings);
    receipt
        .entry_mut(&mappings[0].mapping_id)
        .expect("entry")
        .bind_inventory(&CollectionInventory {
            mapping_id: mappings[0].mapping_id.clone(),
            source_count: 1,
            source_semantic_hash: "semantic".to_owned(),
            source_index_hash: "old-indexes".to_owned(),
            source_index_count: 1,
            target_index_hash: "final-indexes".to_owned(),
            target_index_count: 2,
        })
        .expect("bind inventory");
    let entry = receipt.entry(&mappings[0].mapping_id).expect("entry");
    entry
        .assert_source_unchanged(&CollectionScan {
            count: 1,
            semantic_hash: "semantic".to_owned(),
            index_hash: "old-indexes".to_owned(),
            index_count: 1,
        })
        .expect("source receipt");
    entry
        .assert_verified(&CollectionScan {
            count: 1,
            semantic_hash: "semantic".to_owned(),
            index_hash: "final-indexes".to_owned(),
            index_count: 2,
        })
        .expect("target receipt");
}

fn endpoint(database: &str, physical_collection: &str) -> StorageEndpoint {
    StorageEndpoint {
        profile: "dev".to_string(),
        service_id: "skiff.run/agine".to_string(),
        database: database.to_string(),
        package_id: "skiff.run/agent".to_string(),
        logical_collection: "Provider".to_string(),
        physical_collection: physical_collection.to_string(),
    }
}
