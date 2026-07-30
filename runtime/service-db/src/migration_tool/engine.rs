//! Mongo staging, verification, resume, and atomic publication engine.

use futures_util::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    Client, Collection, IndexModel,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{index::canonical_managed_index_matches, mongo::is_mongo_duplicate_key_error};
use crate::{DbMigrationCrypto, MigrationSemanticCommitment};

use super::{
    model::{ValidatedCollectionMapping, ValidatedMigrationPlan},
    receipt::{CollectionReceipt, MigrationReceipt, MigrationStatus, SecureReceiptStore},
    MigrationToolError, RECEIPT_SCHEMA,
};

pub(super) async fn migrate(
    client: &Client,
    crypto: &DbMigrationCrypto,
    plan: &ValidatedMigrationPlan,
    plan_commitment: &str,
    keyring_fingerprint: &str,
    store: &SecureReceiptStore,
) -> Result<(), MigrationToolError> {
    let mut receipt = match store.load()? {
        Some(receipt) => {
            receipt.validate_resume(
                RECEIPT_SCHEMA,
                plan_commitment,
                keyring_fingerprint,
                &plan.mappings,
            )?;
            receipt
        }
        None => MigrationReceipt::new(
            RECEIPT_SCHEMA,
            plan_commitment,
            keyring_fingerprint,
            &plan.mappings,
        ),
    };

    // Every source and destination is inventoried before the first write.
    let inventories = inventory(client, crypto, plan, Some(&receipt)).await?;
    for (mapping, inventory) in plan.mappings.iter().zip(&inventories) {
        let entry = receipt.entry_mut(&mapping.mapping_id)?;
        entry.bind_inventory(inventory)?;
    }
    store.store(&receipt)?;

    // Build and verify every temporary collection before publishing any one of
    // them. The old collections are read-only throughout.
    for mapping in &plan.mappings {
        let entry = receipt.entry(&mapping.mapping_id)?.clone();
        if entry.status == MigrationStatus::Committed {
            verify_committed_target(client, crypto, mapping, &entry).await?;
            continue;
        }
        let already_published = stage_collection(client, crypto, mapping, &entry).await?;
        let verified = scan_v2_collection(
            client,
            crypto,
            mapping,
            if already_published {
                &mapping.target.physical_collection
            } else {
                &entry.staging_collection
            },
        )
        .await?;
        entry.assert_verified(&verified)?;
        receipt.entry_mut(&mapping.mapping_id)?.status = if already_published {
            MigrationStatus::Committed
        } else {
            MigrationStatus::Staged
        };
        store.store(&receipt)?;
    }

    // Each rename is MongoDB's atomic publish point. A crash between renames is
    // resumed from the secure receipt; a source collection is never deleted.
    for mapping in &plan.mappings {
        let entry = receipt.entry(&mapping.mapping_id)?.clone();
        if entry.status == MigrationStatus::Committed {
            continue;
        }
        let fresh_source = scan_v1_collection(client, crypto, mapping).await?;
        entry.assert_source_unchanged(&fresh_source)?;
        publish_staging_collection(client, mapping, &entry).await?;
        let committed =
            scan_v2_collection(client, crypto, mapping, &mapping.target.physical_collection)
                .await?;
        entry.assert_verified(&committed)?;
        receipt.entry_mut(&mapping.mapping_id)?.status = MigrationStatus::Committed;
        store.store(&receipt)?;
    }
    Ok(())
}

pub(super) async fn inventory(
    client: &Client,
    crypto: &DbMigrationCrypto,
    plan: &ValidatedMigrationPlan,
    resume: Option<&MigrationReceipt>,
) -> Result<Vec<CollectionInventory>, MigrationToolError> {
    let mut inventory = Vec::with_capacity(plan.mappings.len());
    for mapping in &plan.mappings {
        let target_names = client
            .database(&mapping.target.database)
            .list_collection_names()
            .await
            .map_err(|_| MigrationToolError::Mongo)?;
        if target_names.contains(&mapping.target.physical_collection) {
            let count = target_collection(client, mapping)
                .count_documents(doc! {})
                .await
                .map_err(|_| MigrationToolError::Mongo)?;
            let resumable = resume
                .and_then(|receipt| receipt.entry(&mapping.mapping_id).ok())
                .is_some_and(|entry| {
                    matches!(
                        entry.status,
                        MigrationStatus::Staged | MigrationStatus::Committed
                    )
                });
            if !resumable && count != 0 {
                return Err(MigrationToolError::TargetNotEmpty(
                    mapping.mapping_id.clone(),
                ));
            }
            // Even an empty pre-existing target is refused: publishing must
            // never drop or overwrite an operator-created collection.
            if !resumable {
                return Err(MigrationToolError::TargetAlreadyExists(
                    mapping.mapping_id.clone(),
                ));
            }
        }
        let source = scan_v1_collection(client, crypto, mapping).await?;
        if source.count != mapping.expected_source_count {
            return Err(MigrationToolError::Verification(mapping.mapping_id.clone()));
        }
        inventory.push(CollectionInventory {
            mapping_id: mapping.mapping_id.clone(),
            source_count: source.count,
            source_semantic_hash: source.semantic_hash,
            source_index_hash: source.index_hash,
            source_index_count: source.index_count,
            target_index_hash: index_hash(&mapping.target_indexes)?,
            target_index_count: mapping.target_indexes.len() as u64,
        });
    }
    Ok(inventory)
}

async fn stage_collection(
    client: &Client,
    crypto: &DbMigrationCrypto,
    mapping: &ValidatedCollectionMapping,
    receipt: &CollectionReceipt,
) -> Result<bool, MigrationToolError> {
    let database = client.database(&mapping.target.database);
    let names = database
        .list_collection_names()
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    if names.contains(&mapping.target.physical_collection) {
        // A prior rename can complete just before the receipt is flushed.
        if !names.contains(&receipt.staging_collection) {
            let committed =
                scan_v2_collection(client, crypto, mapping, &mapping.target.physical_collection)
                    .await?;
            receipt.assert_verified(&committed)?;
            return Ok(true);
        }
        return Err(MigrationToolError::TargetAlreadyExists(
            mapping.mapping_id.clone(),
        ));
    }
    if !names.contains(&receipt.staging_collection) {
        database
            .create_collection(&receipt.staging_collection)
            .await
            .map_err(|_| MigrationToolError::Mongo)?;
        create_target_indexes(client, mapping, &receipt.staging_collection).await?;
    }

    let source = source_collection(client, mapping);
    let staging = database.collection::<Document>(&receipt.staging_collection);
    validate_target_indexes(
        &list_indexes(&staging).await?,
        &mapping.target_indexes,
        &mapping.mapping_id,
    )?;
    if !mapping.source_exists {
        return Ok(false);
    }
    let mut cursor = source
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    while let Some(document) = cursor
        .try_next()
        .await
        .map_err(|_| MigrationToolError::Mongo)?
    {
        let id = document
            .get("_id")
            .cloned()
            .ok_or_else(|| MigrationToolError::InvalidSource(mapping.mapping_id.clone()))?;
        let mut migrated = crypto
            .migrate_v1_document(
                &mapping.source.service_id,
                &mapping.source.physical_collection,
                mapping.target_context(),
                &mapping.encrypted_fields,
                document,
            )
            .map_err(|_| MigrationToolError::Crypto)?;
        mapping.sanitize(&mut migrated.document)?;
        let intended_commitment = crypto
            .verify_v2_document(
                mapping.target_context(),
                &mapping.encrypted_fields,
                &migrated.document,
            )
            .map_err(|_| MigrationToolError::Crypto)?;
        if let Some(existing) = staging
            .find_one(doc! { "_id": id.clone() })
            .await
            .map_err(|_| MigrationToolError::Mongo)?
        {
            let existing_commitment = crypto
                .verify_v2_document(
                    mapping.target_context(),
                    &mapping.encrypted_fields,
                    &existing,
                )
                .map_err(|_| MigrationToolError::Crypto)?;
            ensure_resume_commitment(
                &mapping.mapping_id,
                existing_commitment.as_bytes(),
                intended_commitment.as_bytes(),
            )?;
            continue;
        }
        staging
            .insert_one(migrated.document)
            .await
            .map_err(|error| {
                if is_mongo_duplicate_key_error(&error) {
                    MigrationToolError::UniqueConstraint(mapping.mapping_id.clone())
                } else {
                    MigrationToolError::Mongo
                }
            })?;
    }
    Ok(false)
}

async fn publish_staging_collection(
    client: &Client,
    mapping: &ValidatedCollectionMapping,
    receipt: &CollectionReceipt,
) -> Result<(), MigrationToolError> {
    let database = client.database(&mapping.target.database);
    let names = database
        .list_collection_names()
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    if names.contains(&mapping.target.physical_collection) {
        if !names.contains(&receipt.staging_collection) {
            return Ok(());
        }
        return Err(MigrationToolError::TargetAlreadyExists(
            mapping.mapping_id.clone(),
        ));
    }
    if !names.contains(&receipt.staging_collection) {
        return Err(MigrationToolError::MissingStaging(
            mapping.mapping_id.clone(),
        ));
    }
    client
        .database("admin")
        .run_command(doc! {
            "renameCollection": format!(
                "{}.{}",
                mapping.target.database, receipt.staging_collection
            ),
            "to": format!(
                "{}.{}",
                mapping.target.database, mapping.target.physical_collection
            ),
            "dropTarget": false,
        })
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    Ok(())
}

async fn verify_committed_target(
    client: &Client,
    crypto: &DbMigrationCrypto,
    mapping: &ValidatedCollectionMapping,
    receipt: &CollectionReceipt,
) -> Result<(), MigrationToolError> {
    let names = client
        .database(&mapping.target.database)
        .list_collection_names()
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    if !names.contains(&mapping.target.physical_collection) {
        return Err(MigrationToolError::MissingTarget(
            mapping.mapping_id.clone(),
        ));
    }
    let target =
        scan_v2_collection(client, crypto, mapping, &mapping.target.physical_collection).await?;
    receipt.assert_verified(&target)
}

async fn scan_v1_collection(
    client: &Client,
    crypto: &DbMigrationCrypto,
    mapping: &ValidatedCollectionMapping,
) -> Result<CollectionScan, MigrationToolError> {
    let source_names = client
        .database(&mapping.source.database)
        .list_collection_names()
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    let source_exists = source_names.contains(&mapping.source.physical_collection);
    if source_exists != mapping.source_exists {
        return Err(if mapping.source_exists {
            MigrationToolError::MissingSource(mapping.mapping_id.clone())
        } else {
            MigrationToolError::InvalidSource(mapping.mapping_id.clone())
        });
    }
    if !source_exists {
        return Ok(CollectionScan {
            count: 0,
            semantic_hash: SemanticAccumulator::new().finish(),
            index_hash: index_hash(&[])?,
            index_count: 0,
        });
    }
    let collection = source_collection(client, mapping);
    let indexes = list_indexes(&collection).await?;
    let mut accumulator = SemanticAccumulator::new();
    let mut cursor = collection
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    while let Some(document) = cursor
        .try_next()
        .await
        .map_err(|_| MigrationToolError::Mongo)?
    {
        let mut migrated = crypto
            .migrate_v1_document(
                &mapping.source.service_id,
                &mapping.source.physical_collection,
                mapping.target_context(),
                &mapping.encrypted_fields,
                document,
            )
            .map_err(|_| MigrationToolError::Crypto)?;
        mapping.sanitize(&mut migrated.document)?;
        let commitment = crypto
            .verify_v2_document(
                mapping.target_context(),
                &mapping.encrypted_fields,
                &migrated.document,
            )
            .map_err(|_| MigrationToolError::Crypto)?;
        accumulator.push(commitment);
    }
    Ok(CollectionScan {
        count: accumulator.count,
        semantic_hash: accumulator.finish(),
        index_hash: index_hash(&indexes)?,
        index_count: indexes.len() as u64,
    })
}

async fn scan_v2_collection(
    client: &Client,
    crypto: &DbMigrationCrypto,
    mapping: &ValidatedCollectionMapping,
    collection_name: &str,
) -> Result<CollectionScan, MigrationToolError> {
    let collection = client
        .database(&mapping.target.database)
        .collection::<Document>(collection_name);
    let indexes = list_indexes(&collection).await?;
    validate_target_indexes(&indexes, &mapping.target_indexes, &mapping.mapping_id)?;
    let mut accumulator = SemanticAccumulator::new();
    let mut cursor = collection
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    while let Some(document) = cursor
        .try_next()
        .await
        .map_err(|_| MigrationToolError::Mongo)?
    {
        let commitment = crypto
            .verify_v2_document(
                mapping.target_context(),
                &mapping.encrypted_fields,
                &document,
            )
            .map_err(|_| MigrationToolError::Crypto)?;
        accumulator.push(commitment);
    }
    Ok(CollectionScan {
        count: accumulator.count,
        semantic_hash: accumulator.finish(),
        index_hash: index_hash(&mapping.target_indexes)?,
        index_count: mapping.target_indexes.len() as u64,
    })
}

async fn create_target_indexes(
    client: &Client,
    mapping: &ValidatedCollectionMapping,
    staging_collection: &str,
) -> Result<(), MigrationToolError> {
    if mapping.target_indexes.is_empty() {
        return Ok(());
    }
    let target = client
        .database(&mapping.target.database)
        .collection::<Document>(staging_collection);
    target
        .create_indexes(mapping.target_indexes.clone())
        .await
        .map_err(|error| {
            if is_mongo_duplicate_key_error(&error) {
                MigrationToolError::UniqueConstraint(mapping.mapping_id.clone())
            } else {
                MigrationToolError::Mongo
            }
        })?;
    Ok(())
}

fn validate_target_indexes(
    actual: &[IndexModel],
    expected: &[IndexModel],
    mapping_id: &str,
) -> Result<(), MigrationToolError> {
    let mut actual_by_name = std::collections::BTreeMap::new();
    for model in actual {
        let name = index_name(model);
        if name == "_id_" {
            continue;
        }
        if name.is_empty() || actual_by_name.insert(name.to_owned(), model).is_some() {
            return Err(MigrationToolError::Verification(mapping_id.to_owned()));
        }
    }
    if actual_by_name.len() != expected.len() {
        return Err(MigrationToolError::Verification(mapping_id.to_owned()));
    }
    for expected_model in expected {
        let name = index_name(expected_model);
        let Some(actual_model) = actual_by_name.get(name) else {
            return Err(MigrationToolError::Verification(mapping_id.to_owned()));
        };
        if !canonical_managed_index_matches(actual_model, expected_model) {
            return Err(MigrationToolError::Verification(mapping_id.to_owned()));
        }
    }
    Ok(())
}

async fn list_indexes(
    collection: &Collection<Document>,
) -> Result<Vec<IndexModel>, MigrationToolError> {
    let mut indexes = collection
        .list_indexes()
        .await
        .map_err(|_| MigrationToolError::Mongo)?;
    let mut values = Vec::new();
    while let Some(index) = indexes
        .try_next()
        .await
        .map_err(|_| MigrationToolError::Mongo)?
    {
        values.push(index);
    }
    values.sort_by(|left, right| index_name(left).cmp(index_name(right)));
    Ok(values)
}

fn index_name(index: &IndexModel) -> &str {
    index
        .options
        .as_ref()
        .and_then(|options| options.name.as_deref())
        .unwrap_or("")
}

fn index_hash(indexes: &[IndexModel]) -> Result<String, MigrationToolError> {
    let encoded = serde_json::to_vec(indexes).map_err(|_| MigrationToolError::Output)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn source_collection(
    client: &Client,
    mapping: &ValidatedCollectionMapping,
) -> Collection<Document> {
    client
        .database(&mapping.source.database)
        .collection::<Document>(&mapping.source.physical_collection)
}

fn target_collection(
    client: &Client,
    mapping: &ValidatedCollectionMapping,
) -> Collection<Document> {
    client
        .database(&mapping.target.database)
        .collection::<Document>(&mapping.target.physical_collection)
}

fn ensure_resume_commitment(
    mapping_id: &str,
    existing: &[u8],
    intended: &[u8],
) -> Result<(), MigrationToolError> {
    if existing == intended {
        Ok(())
    } else {
        Err(MigrationToolError::DuplicateId(mapping_id.to_owned()))
    }
}

struct SemanticAccumulator {
    hasher: Sha256,
    count: u64,
}

impl SemanticAccumulator {
    fn new() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"skiff-service-db-hardcut-collection-commitment-v1");
        Self { hasher, count: 0 }
    }

    fn push(&mut self, commitment: MigrationSemanticCommitment) {
        self.hasher.update(commitment.as_bytes());
        self.count += 1;
    }

    fn finish(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

#[derive(Clone)]
pub(super) struct CollectionScan {
    pub(super) count: u64,
    pub(super) semantic_hash: String,
    pub(super) index_hash: String,
    pub(super) index_count: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CollectionInventory {
    pub(super) mapping_id: String,
    pub(super) source_count: u64,
    #[serde(skip_serializing)]
    pub(super) source_semantic_hash: String,
    #[serde(skip_serializing)]
    pub(super) source_index_hash: String,
    pub(super) source_index_count: u64,
    pub(super) target_index_hash: String,
    pub(super) target_index_count: u64,
}

#[cfg(test)]
mod tests {
    use mongodb::{
        bson::doc,
        options::{Collation, IndexOptions},
        IndexModel,
    };

    use super::{ensure_resume_commitment, validate_target_indexes};
    use crate::index::canonical_managed_index_model;
    use crate::migration_tool::MigrationToolError;

    #[test]
    fn resume_accepts_identical_document_and_rejects_id_collision() {
        let mapping_id = "m-00000000000000000000000000000000";
        ensure_resume_commitment(mapping_id, b"same", b"same")
            .expect("identical staged document must be resumable");
        let error = ensure_resume_commitment(mapping_id, b"existing", b"different")
            .expect_err("same _id with different content must fail closed");
        assert!(matches!(error, MigrationToolError::DuplicateId(id) if id == mapping_id));
    }

    #[test]
    fn staging_catalog_must_exactly_match_the_final_declared_indexes() {
        let expected = canonical_managed_index_model(
            "example.test/package",
            "Item",
            "byOwner",
            vec![("ownerId".to_owned(), 1), ("createdAt".to_owned(), -1)],
            true,
        )
        .expect("canonical index");
        let primary = IndexModel::builder()
            .keys(doc! { "_id": 1 })
            .options(IndexOptions::builder().name("_id_".to_owned()).build())
            .build();
        validate_target_indexes(
            &[primary.clone(), expected.clone()],
            std::slice::from_ref(&expected),
            "mapping",
        )
        .expect("exact final indexes");

        let mut drift = expected.clone();
        drift.options.as_mut().expect("options").collation =
            Some(Collation::builder().locale("en").build());
        assert!(validate_target_indexes(
            &[primary.clone(), drift],
            std::slice::from_ref(&expected),
            "mapping"
        )
        .is_err());

        let unmanaged = IndexModel::builder()
            .keys(doc! { "operator": 1 })
            .options(IndexOptions::builder().name("operator".to_owned()).build())
            .build();
        assert!(validate_target_indexes(
            &[primary, expected.clone(), unmanaged],
            &[expected],
            "mapping"
        )
        .is_err());
    }
}
