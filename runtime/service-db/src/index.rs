use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
};

use futures_util::{stream, StreamExt};
use mongodb::{
    bson::{Bson, Document},
    options::{Collation, IndexOptions},
    Collection, IndexModel,
};
use sha2::{Digest, Sha256};

use crate::{
    mongo::{is_mongo_duplicate_key_error, is_mongo_namespace_not_found_error},
    Result, ServiceDbError, ServiceDbRuntime,
};

const MANAGED_INDEX_PREFIX: &str = "skiff_midx_v1_";
const UNIQUE_PROVISION_FAILURE: &str =
    "service database unique index cannot be provisioned because existing records violate the declared constraint";
/// Bounds Mongo catalog/index work during assembly activation. A database is the concurrency
/// unit: collections within one database remain strictly ordered, while independent databases
/// can make progress together. Eight is a conservative bound compatible with the Mongo driver's
/// current ten-connection default, leaving two connections available for unrelated work.
const DATABASE_RECONCILIATION_CONCURRENCY: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedIndexSpec {
    name: String,
    keys: Vec<(String, i32)>,
    unique: bool,
}

impl ManagedIndexSpec {
    fn mongo_model(&self) -> IndexModel {
        let mut keys = Document::new();
        for (field, direction) in &self.keys {
            keys.insert(field, *direction);
        }
        let options = IndexOptions::builder()
            .name(self.name.clone())
            .unique(self.unique)
            .collation(Collation::builder().locale("simple").build())
            .build();
        IndexModel::builder().keys(keys).options(options).build()
    }
}

/// Builds the exact managed Mongo index model without performing storage I/O.
///
/// Migration and reconciliation must share this builder so staging cannot produce an index
/// identity or option set that activation would later reject.
#[cfg(any(feature = "migration-tool", test))]
pub(crate) fn canonical_managed_index_model(
    package_id: &str,
    logical_collection: &str,
    logical_index: &str,
    keys: Vec<(String, i32)>,
    unique: bool,
) -> Result<IndexModel> {
    canonical_managed_index_spec(package_id, logical_collection, logical_index, keys, unique)
        .map(|spec| spec.mongo_model())
}

#[cfg(any(feature = "migration-tool", test))]
pub(crate) fn canonical_managed_index_matches(actual: &IndexModel, expected: &IndexModel) -> bool {
    let Some(expected_name) = expected
        .options
        .as_ref()
        .and_then(|options| options.name.clone())
    else {
        return false;
    };
    if actual
        .options
        .as_ref()
        .and_then(|options| options.name.as_deref())
        != Some(expected_name.as_str())
    {
        return false;
    }
    let Some(keys) = index_keys(&expected.keys) else {
        return false;
    };
    let spec = ManagedIndexSpec {
        name: expected_name,
        keys,
        unique: expected
            .options
            .as_ref()
            .and_then(|options| options.unique)
            .unwrap_or(false),
    };
    mongo_index_matches(actual, &spec)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CollectionIndexPlan {
    package_id: String,
    logical_collection: String,
    physical_collection: String,
    indexes: BTreeMap<String, ManagedIndexSpec>,
}

#[derive(Clone, Eq, PartialEq)]
struct DatabaseIndexPlan {
    mongo_url: String,
    database_name: String,
    collections: BTreeMap<String, CollectionIndexPlan>,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct ServiceDbIndexProvisionPlan {
    databases: BTreeMap<(String, String), DatabaseIndexPlan>,
}

impl ServiceDbIndexProvisionPlan {
    pub(crate) fn from_runtimes(runtimes: &[ServiceDbRuntime]) -> Result<Self> {
        let mut plan = Self::default();
        for runtime in runtimes {
            let database_key = (runtime.mongo_url.clone(), runtime.database_name.clone());
            let database =
                plan.databases
                    .entry(database_key)
                    .or_insert_with(|| DatabaseIndexPlan {
                        mongo_url: runtime.mongo_url.clone(),
                        database_name: runtime.database_name.clone(),
                        collections: BTreeMap::new(),
                    });
            for collection in runtime.metadata.collections() {
                let collection_plan = collection_index_plan(collection)?;
                merge_collection_plan(database, collection_plan)?;
            }
        }
        Ok(plan)
    }

    pub(crate) async fn reconcile(&self) -> Result<()> {
        reconcile_databases_bounded(
            self.databases.values(),
            DATABASE_RECONCILIATION_CONCURRENCY,
            reconcile_database,
        )
        .await
    }
}

async fn reconcile_database(database: &DatabaseIndexPlan) -> Result<()> {
    let client_cell = crate::service_db_client_cell(&database.mongo_url);
    let client = client_cell
        .get_or_try_init(|| async {
            let options = crate::service_db_client_options(&database.mongo_url).await?;
            mongodb::Client::with_options(options)
        })
        .await
        .cloned()
        .map_err(ServiceDbError::from)?;
    let mongo_database = client.database(&database.database_name);
    reconcile_collections_in_order(database.collections.values(), |collection| {
        reconcile_collection(
            mongo_database.collection::<Document>(&collection.physical_collection),
            collection,
        )
    })
    .await
}

/// Runs independent database units with a fixed in-task bound. Success means every database has
/// completed. The first observed failure returns immediately and drops the remaining futures,
/// preserving fail-fast activation when a peer database is slow or permanently pending. No task
/// is task per database: `buffer_unordered` owns at most `concurrency_limit` live futures.
async fn reconcile_databases_bounded<Input, Error, Reconcile, ReconcileFuture>(
    databases: impl IntoIterator<Item = Input>,
    concurrency_limit: usize,
    reconcile: Reconcile,
) -> std::result::Result<(), Error>
where
    Reconcile: Fn(Input) -> ReconcileFuture,
    ReconcileFuture: Future<Output = std::result::Result<(), Error>>,
{
    assert!(
        concurrency_limit > 0,
        "database reconciliation concurrency must be non-zero"
    );
    let mut pending =
        stream::iter(databases.into_iter().map(reconcile)).buffer_unordered(concurrency_limit);
    while let Some(result) = pending.next().await {
        result?;
    }
    Ok(())
}

/// Preserves the existing per-database contract: collections are reconciled in plan order and the
/// first collection failure stops that database before a later collection can begin.
async fn reconcile_collections_in_order<Input, Error, Reconcile, ReconcileFuture>(
    collections: impl IntoIterator<Item = Input>,
    reconcile: Reconcile,
) -> std::result::Result<(), Error>
where
    Reconcile: Fn(Input) -> ReconcileFuture,
    ReconcileFuture: Future<Output = std::result::Result<(), Error>>,
{
    for collection in collections {
        reconcile(collection).await?;
    }
    Ok(())
}

fn collection_index_plan(
    collection: &crate::metadata::DbCollectionMetadata,
) -> Result<CollectionIndexPlan> {
    let mut indexes = BTreeMap::new();
    for index in &collection.indexes {
        let keys = collection
            .index_key_document(index)?
            .into_iter()
            .map(|(field, direction)| {
                let direction = match direction {
                    Bson::Int32(direction) => direction,
                    _ => unreachable!("service DB index direction is always encoded as Int32"),
                };
                (field, direction)
            })
            .collect::<Vec<_>>();
        let spec = canonical_managed_index_spec(
            &collection.package_id,
            &collection.logical_collection_name,
            &index.name,
            keys,
            index.unique,
        )?;
        if indexes.insert(spec.name.clone(), spec).is_some() {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "runtime program DB metadata repeats managed index identity for {}",
                collection.logical_collection_name
            )));
        }
    }
    Ok(CollectionIndexPlan {
        package_id: collection.package_id.clone(),
        logical_collection: collection.logical_collection_name.clone(),
        physical_collection: collection.collection_name.clone(),
        indexes,
    })
}

fn merge_collection_plan(
    database: &mut DatabaseIndexPlan,
    incoming: CollectionIndexPlan,
) -> Result<()> {
    let Some(existing) = database.collections.get_mut(&incoming.physical_collection) else {
        database
            .collections
            .insert(incoming.physical_collection.clone(), incoming);
        return Ok(());
    };
    if existing.package_id != incoming.package_id
        || existing.logical_collection != incoming.logical_collection
    {
        return Err(ServiceDbError::InvalidDbMetadata(format!(
            "whole service DB candidate maps different logical collections to physical collection {}",
            incoming.physical_collection
        )));
    }
    for (name, spec) in incoming.indexes {
        match existing.indexes.get(&name) {
            Some(current) if current != &spec => {
                return Err(ServiceDbError::InvalidDbMetadata(format!(
                    "whole service DB candidate gives managed index {name} conflicting definitions"
                )));
            }
            Some(_) => {}
            None => {
                existing.indexes.insert(name, spec);
            }
        }
    }
    Ok(())
}

async fn reconcile_collection(
    mongo_collection: Collection<Document>,
    plan: &CollectionIndexPlan,
) -> Result<()> {
    let existing = list_indexes_or_empty(&mongo_collection).await?;
    let missing = classify_existing_indexes(&existing, plan)?;
    if missing.is_empty() {
        return Ok(());
    }
    let models = missing
        .iter()
        .filter_map(|name| plan.indexes.get(name))
        .map(ManagedIndexSpec::mongo_model)
        .collect::<Vec<_>>();
    match mongo_collection.create_indexes(models).await {
        Ok(_) => Ok(()),
        Err(error) if is_mongo_duplicate_key_error(&error) => {
            Err(ServiceDbError::provision(UNIQUE_PROVISION_FAILURE))
        }
        Err(error) => {
            // A concurrent replica can win the exact createIndexes race. Re-read and accept
            // only the complete canonical result; otherwise preserve the original provider error.
            let after = list_indexes_or_empty(&mongo_collection).await?;
            if classify_existing_indexes(&after, plan)?.is_empty() {
                Ok(())
            } else {
                Err(ServiceDbError::Mongo(error))
            }
        }
    }
}

async fn list_indexes_or_empty(collection: &Collection<Document>) -> Result<Vec<IndexModel>> {
    let mut cursor = match collection.list_indexes().await {
        Ok(cursor) => cursor,
        Err(error) if is_mongo_namespace_not_found_error(&error) => return Ok(Vec::new()),
        Err(error) => return Err(ServiceDbError::Mongo(error)),
    };
    let mut indexes = Vec::new();
    while let Some(index) = cursor.next().await.transpose()? {
        indexes.push(index);
    }
    Ok(indexes)
}

fn classify_existing_indexes(
    existing: &[IndexModel],
    plan: &CollectionIndexPlan,
) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for model in existing {
        let name = model
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref())
            .unwrap_or_default();
        if name == "_id_" || !name.starts_with(MANAGED_INDEX_PREFIX) {
            continue;
        }
        let Some(expected) = plan.indexes.get(name) else {
            return Err(ServiceDbError::provision(format!(
                "service database contains stale managed index {name}; automatic destructive index removal is forbidden"
            )));
        };
        if !mongo_index_matches(model, expected) {
            return Err(ServiceDbError::provision(format!(
                "service database managed index {name} differs from the admitted service contract"
            )));
        }
        found.insert(name.to_string());
    }
    Ok(plan
        .indexes
        .keys()
        .filter(|name| !found.contains(*name))
        .cloned()
        .collect())
}

fn mongo_index_matches(model: &IndexModel, expected: &ManagedIndexSpec) -> bool {
    let keys = index_keys(&model.keys);
    if keys.as_deref() != Some(expected.keys.as_slice()) {
        return false;
    }
    let options = model.options.as_ref();
    let unique = options.and_then(|options| options.unique).unwrap_or(false);
    if unique != expected.unique {
        return false;
    }
    let simple_collation = options
        .and_then(|options| options.collation.as_ref())
        .map_or(true, |collation| collation.locale == "simple");
    simple_collation && !has_noncanonical_managed_options(options)
}

fn index_keys(keys: &Document) -> Option<Vec<(String, i32)>> {
    keys.iter()
        .map(|(field, direction)| match direction {
            Bson::Int32(direction) => Some((field.clone(), *direction)),
            Bson::Int64(direction) => i32::try_from(*direction)
                .ok()
                .map(|direction| (field.clone(), direction)),
            _ => None,
        })
        .collect()
}

fn has_noncanonical_managed_options(options: Option<&IndexOptions>) -> bool {
    let Some(options) = options else {
        return false;
    };
    options.background.unwrap_or(false)
        || options.sparse.unwrap_or(false)
        || options.expire_after.is_some()
        || options.storage_engine.is_some()
        || options.partial_filter_expression.is_some()
        || options.hidden.unwrap_or(false)
        || options.default_language.is_some()
        || options.language_override.is_some()
        || options.text_index_version.is_some()
        || options.weights.is_some()
        || options.sphere_2d_index_version.is_some()
        || options.bits.is_some()
        || options.max.is_some()
        || options.min.is_some()
        || options.bucket_size.is_some()
        || options.wildcard_projection.is_some()
        || options.clustered().unwrap_or(false)
}

pub(crate) fn managed_index_name(
    package_id: &str,
    logical_collection: &str,
    logical_index: &str,
) -> String {
    let mut hasher = Sha256::new();
    for value in [package_id, logical_collection, logical_index] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{MANAGED_INDEX_PREFIX}{}", hex::encode(hasher.finalize()))
}

fn canonical_managed_index_spec(
    package_id: &str,
    logical_collection: &str,
    logical_index: &str,
    keys: Vec<(String, i32)>,
    unique: bool,
) -> Result<ManagedIndexSpec> {
    for (label, value) in [
        ("package ID", package_id),
        ("logical collection", logical_collection),
        ("logical index", logical_index),
    ] {
        if value.trim().is_empty() || value != value.trim() {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "managed index {label} must be a non-empty canonical string"
            )));
        }
    }
    if keys.is_empty() {
        return Err(ServiceDbError::InvalidDbMetadata(
            "managed index must declare at least one physical key".to_string(),
        ));
    }
    let mut fields = BTreeSet::new();
    for (field, direction) in &keys {
        if field.trim().is_empty() || field != field.trim() {
            return Err(ServiceDbError::InvalidDbMetadata(
                "managed index physical field must be a non-empty canonical path".to_string(),
            ));
        }
        if !fields.insert(field.clone()) {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "managed index repeats physical field {field}"
            )));
        }
        if *direction != -1 && *direction != 1 {
            return Err(ServiceDbError::InvalidDbMetadata(format!(
                "managed index physical field {field} has unsupported direction {direction}"
            )));
        }
    }
    Ok(ManagedIndexSpec {
        name: managed_index_name(package_id, logical_collection, logical_index),
        keys,
        unique,
    })
}

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod tests;
