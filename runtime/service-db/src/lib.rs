use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use mongodb::{
    bson::{doc, Bson, Document},
    options::ClientOptions,
    Client, ClientSession, Collection,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skiff_artifact_model::DbMetadataIr;
use skiff_runtime_boundary::recoverable::{
    RecoverableArtifactRetentionRootStore, RecoverableArtifactStore,
};
use skiff_runtime_capability_context::{
    DbDocument, DbKey, DbOneSelector, DbOrderEntry, DbPageResult, DbQuery, DbWriteResult,
    FieldPath, ServiceDbChange, ServiceDbFindOptions,
};
pub use skiff_runtime_capability_context::{
    DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans, DbRuntimeChange,
    DbRuntimeSetOp, FileCapabilityRecord, FileCapabilityRecord as SkiffFileRecord,
};
use skiff_runtime_model::{
    recoverable::RecoverableArtifactRetentionRoot, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use tokio::sync::OnceCell;

use skiff_runtime_boundary::date_value;

mod capability;
mod cascade;
mod error;
mod lease;
mod mapping;
mod metadata;
mod mongo;
mod provider;
mod store;

pub use capability::{
    ServiceDbCapabilityFactory, ServiceDbCapabilityHandle, ServiceDbCapabilityStore,
};
pub use error::{Result, ServiceDbError};
pub use provider::MongoServiceDbProviderFactory;

use cascade::{
    cascade_plan_for_change, cascade_plan_for_changed_documents,
    cascade_plan_for_deleted_documents, cascade_plan_for_replacement, CascadeFileDeletePlan,
};
use lease::{
    add_ms, and_filter, guarded_filter, has_matching_lease_guards, key_bson,
    lease_available_filter, lease_claim_expires_at_ms, lease_document, lease_field, lease_i64,
    lease_live_key_filter, lease_lost_error, lease_slot_path, matching_lease_guards,
    LEASE_CLAIMED_AT_MS_FIELD, LEASE_EXPIRES_AT_MS_FIELD, LEASE_MAX_EXPIRES_AT_MS_FIELD,
    LEASE_OWNER_FIELD, LEASE_REQUEST_ID_FIELD, LEASE_TOKEN_FIELD, SKIFF_LEASES_FIELD,
};
pub use lease::{service_db_now_ms, DbLeaseHandle, DbLeaseHold};
use metadata::{DbCollectionMetadata, ServiceDbMetadata};
use mongo::{
    is_mongo_duplicate_key_error, update_without_set_on_insert, MongoFindManyPlan,
    MongoFindOnePlan, MongoOneWritePlan, MongoSessionExecutor,
};
pub use store::ServiceDbStore;
pub type DbStore = ServiceDbStore;

#[derive(Clone, Debug)]
pub struct ServiceDbConfig {
    pub mongo_url: String,
}

#[derive(Debug)]
pub struct DbTransactionState {
    session: ClientSession,
}

#[derive(Debug)]
pub struct DbRequestState {
    pub transaction: Option<DbTransactionState>,
    pub owner: String,
    pub request_id: String,
    pub leases: Vec<DbLeaseHold>,
    pub lease_lost: bool,
}

impl DbRequestState {
    pub fn new(owner: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self {
            transaction: None,
            owner: owner.into(),
            request_id: request_id.into(),
            leases: Vec::new(),
            lease_lost: false,
        }
    }

    fn ensure_lease_live(&self) -> Result<()> {
        if self.lease_lost {
            Err(ServiceDbError::LeaseLost(
                "db lease was lost by the current request".to_string(),
            ))
        } else {
            Ok(())
        }
    }
}

impl Default for DbRequestState {
    fn default() -> Self {
        Self::new("local-runtime", "local-request")
    }
}

#[derive(Clone)]
pub struct ServiceDbRuntime {
    mongo_url: String,
    database_name: String,
    metadata: Arc<ServiceDbMetadata>,
    client: Arc<OnceCell<Client>>,
}

static SERVICE_DB_CLIENT_CELLS: OnceLock<Mutex<HashMap<String, Weak<OnceCell<Client>>>>> =
    OnceLock::new();

const SKIFF_FILE_COLLECTION: &str = "_skiff_file";
const SKIFF_FILE_ID_FIELD: &str = "id";
const SKIFF_RECOVERABLE_ARTIFACT_ROOT_COLLECTION: &str = "_skiff_recoverable_artifact_root";

struct CurrentRequestRecoverableArtifactStore {
    artifact_identity: String,
    build_id: String,
}

impl CurrentRequestRecoverableArtifactStore {
    fn new(context: &DbRecoverableRuntimeContext) -> Self {
        Self {
            artifact_identity: context.artifact_identity.clone(),
            build_id: context.build_id.clone(),
        }
    }
}

impl RecoverableArtifactStore for CurrentRequestRecoverableArtifactStore {
    fn can_load_artifact(&self, artifact_identity: &str, build_id: &str) -> bool {
        artifact_identity == self.artifact_identity && build_id == self.build_id
    }
}

#[derive(Default)]
struct CollectedRecoverableRootStore {
    roots: Vec<RecoverableArtifactRetentionRoot>,
}

impl RecoverableArtifactRetentionRootStore for CollectedRecoverableRootStore {
    fn persist_roots(
        &mut self,
        roots: &[RecoverableArtifactRetentionRoot],
    ) -> std::result::Result<(), String> {
        self.roots.extend_from_slice(roots);
        Ok(())
    }
}

impl ServiceDbRuntime {
    pub fn new(
        service_id: String,
        mongo_url: String,
        runtime_program_db: &[DbMetadataIr],
    ) -> Result<Self> {
        Self::new_with_config(
            service_id,
            ServiceDbConfig { mongo_url },
            runtime_program_db,
        )
    }

    pub fn new_with_config(
        service_id: String,
        config: ServiceDbConfig,
        runtime_program_db: &[DbMetadataIr],
    ) -> Result<Self> {
        let database_name = service_id_storage_database_name(&service_id)?;
        validate_service_database_name(&database_name)?;
        let mongo_url = config.mongo_url;
        let client = service_db_client_cell(&mongo_url);
        Ok(Self {
            mongo_url,
            database_name,
            metadata: Arc::new(ServiceDbMetadata::from_runtime_program_db(
                runtime_program_db,
            )?),
            client,
        })
    }

    pub async fn client(&self) -> Result<Client> {
        self.client
            .get_or_try_init(|| async {
                let options = service_db_client_options(&self.mongo_url).await?;
                Client::with_options(options)
            })
            .await
            .cloned()
            .map_err(ServiceDbError::from)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn database_name_for_test(&self) -> String {
        self.database_name.clone()
    }

    pub async fn find_one_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.key_filter(&key)?;
        let document = self
            .find_one_document(binding, filter, None, projection.as_deref(), session)
            .await?;
        document
            .map(|document| binding.business_value_from_document(document))
            .transpose()
    }

    pub async fn find_one_by_key_runtime(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.key_filter(&key)?;
        let document = self
            .find_one_document(binding, filter, None, projection.as_deref(), session)
            .await?;
        let read_context = recoverable_read_context(&context);
        document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    pub async fn find_one_by_query(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let sort = binding.order_document(&order)?;
        let document = self
            .find_one_document(binding, filter, sort, projection.as_deref(), session)
            .await?;
        document
            .map(|document| binding.business_value_from_document(document))
            .transpose()
    }

    pub async fn find_one_by_query_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let sort = binding.order_document(&order)?;
        let document = self
            .find_one_document(binding, filter, sort, projection.as_deref(), session)
            .await?;
        let read_context = recoverable_read_context(&context);
        document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    pub async fn find_many_page(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        session: Option<&mut ClientSession>,
    ) -> Result<DbPageResult> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let sort = binding.page_sort_document(&options)?;
        if options.limit == Some(0) {
            return Ok(DbPageResult { values: Vec::new() });
        }
        let projection_doc = binding.projection_document(projection.as_deref())?;
        let documents = self
            .mongo_executor(&binding.collection_name, session)
            .await?
            .find_many(MongoFindManyPlan {
                filter,
                sort,
                projection: projection_doc,
                limit: options.limit,
                offset: options.offset,
            })
            .await?;
        let values = documents
            .into_iter()
            .map(|document| binding.business_value_from_document(document))
            .collect::<Result<Vec<_>>>()?;

        Ok(DbPageResult { values })
    }

    pub async fn find_many_page_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        session: Option<&mut ClientSession>,
    ) -> Result<Vec<RuntimeValue>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let sort = binding.page_sort_document(&options)?;
        if options.limit == Some(0) {
            return Ok(Vec::new());
        }
        let projection_doc = binding.projection_document(projection.as_deref())?;
        let documents = self
            .mongo_executor(&binding.collection_name, session)
            .await?
            .find_many(MongoFindManyPlan {
                filter,
                sort,
                projection: projection_doc,
                limit: options.limit,
                offset: options.offset,
            })
            .await?;
        let read_context = recoverable_read_context(&context);
        documents
            .into_iter()
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn exists_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        session: Option<&mut ClientSession>,
    ) -> Result<bool> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.key_filter(&key)?;
        Ok(self.count_documents(binding, filter, session).await? > 0)
    }

    pub async fn exists_by_query(
        &self,
        type_name: &str,
        query: DbQuery,
        session: Option<&mut ClientSession>,
    ) -> Result<bool> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        Ok(self.count_documents(binding, filter, session).await? > 0)
    }

    pub async fn insert_many_result(
        &self,
        type_name: &str,
        values: Vec<DbDocument>,
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let (documents, _) = binding.documents_from_business_values(values)?;
        let inserted_count = self.insert_many_count(binding, documents, session).await?;
        Ok(DbWriteResult::new(
            serde_json::json!({ "insertedCount": inserted_count }),
        ))
    }

    pub async fn update_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: ServiceDbChange,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none()
            && (!binding.immutable_file_paths_for_change(&change).is_empty()
                || has_matching_lease_guards(binding, lease_guards))
        {
            let mut session = self.start_transaction().await?;
            let result = self
                .update_one_inner(
                    binding,
                    type_name,
                    selector,
                    change,
                    lease_guards,
                    Some(&mut session),
                )
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.update_one_inner(binding, type_name, selector, change, lease_guards, session)
            .await
    }

    async fn update_one_inner(
        &self,
        binding: &DbCollectionMetadata,
        type_name: &str,
        selector: DbOneSelector,
        change: ServiceDbChange,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let cascade_paths = binding.immutable_file_paths_for_change(&change);
        let update = binding.validated_change_update(type_name, change.clone())?;
        if update.is_empty() {
            return self
                .find_one_for_selector(binding, selector, None, session)
                .await;
        }
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_document = if cascade_paths.is_empty() {
            None
        } else {
            executor
                .find_one(MongoFindOnePlan {
                    filter: filter.clone(),
                    sort: sort.clone(),
                    ..Default::default()
                })
                .await?
        };
        let document = executor
            .find_one_and_update(
                MongoOneWritePlan {
                    filter: guarded_filter,
                    sort,
                },
                update,
            )
            .await?;
        if document.is_none() {
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(old_document) = &old_document {
            self.delete_skiff_files_by_plan(
                cascade_plan_for_change(old_document, &change, &cascade_paths),
                executor.session_mut(),
            )
            .await?;
        }
        document
            .map(|document| binding.business_value_from_document(document))
            .transpose()
    }

    pub async fn update_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none() {
            let mut session = self.start_transaction().await?;
            let result = self
                .update_one_runtime_inner(
                    binding,
                    type_name,
                    selector,
                    change,
                    heap,
                    context,
                    lease_guards,
                    Some(&mut session),
                )
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.update_one_runtime_inner(
            binding,
            type_name,
            selector,
            change,
            heap,
            context,
            lease_guards,
            session,
        )
        .await
    }

    async fn update_one_runtime_inner(
        &self,
        binding: &DbCollectionMetadata,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        lease_guards: &[DbLeaseHold],
        mut session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let cascade_paths = binding.immutable_file_paths_for_change(&change.wire_change);
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let update = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.runtime_change_update_document(
                type_name,
                change.clone(),
                heap,
                Some(&mut write_context),
            )?
        };
        if update.is_empty() {
            return self
                .find_one_for_selector_runtime(binding, selector, None, heap, &context, session)
                .await;
        }
        self.persist_recoverable_artifact_retention_roots(
            &root_store.roots,
            session.as_deref_mut(),
        )
        .await?;
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_document = if cascade_paths.is_empty() {
            None
        } else {
            executor
                .find_one(MongoFindOnePlan {
                    filter: filter.clone(),
                    sort: sort.clone(),
                    ..Default::default()
                })
                .await?
        };
        let document = executor
            .find_one_and_update(
                MongoOneWritePlan {
                    filter: guarded_filter,
                    sort,
                },
                update,
            )
            .await?;
        if document.is_none() {
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(old_document) = &old_document {
            self.delete_skiff_files_by_plan(
                cascade_plan_for_change(old_document, &change.wire_change, &cascade_paths),
                executor.session_mut(),
            )
            .await?;
        }
        let read_context = recoverable_read_context(&context);
        document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    pub async fn update_many(
        &self,
        type_name: &str,
        query: DbQuery,
        change: ServiceDbChange,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none()
            && (!binding.immutable_file_paths_for_change(&change).is_empty()
                || has_matching_lease_guards(binding, lease_guards))
        {
            let mut session = self.start_transaction().await?;
            let result = self
                .update_many_inner(
                    binding,
                    type_name,
                    query,
                    change,
                    lease_guards,
                    Some(&mut session),
                )
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.update_many_inner(binding, type_name, query, change, lease_guards, session)
            .await
    }

    async fn update_many_inner(
        &self,
        binding: &DbCollectionMetadata,
        type_name: &str,
        query: DbQuery,
        change: ServiceDbChange,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let filter = binding.query_filter(query)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let cascade_paths = binding.immutable_file_paths_for_change(&change);
        let update = binding.validated_change_update(type_name, change.clone())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_documents = if cascade_paths.is_empty() {
            Vec::new()
        } else {
            executor
                .find_many(MongoFindManyPlan {
                    filter: filter.clone(),
                    ..Default::default()
                })
                .await?
        };
        let result = executor.update_many(guarded_filter, update).await?;
        self.delete_skiff_files_by_plan(
            cascade_plan_for_changed_documents(&old_documents, &change, &cascade_paths),
            executor.session_mut(),
        )
        .await?;
        Ok(DbWriteResult::new(serde_json::json!({
            "matchedCount": result.matched_count,
            "modifiedCount": result.modified_count,
        })))
    }

    pub async fn upsert_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        insert: DbDocument,
        change: ServiceDbChange,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.key_filter(&key)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let insert = binding.upsert_insert_value_with_key(insert, &key)?;
        let (mut insert_document, _) = binding.document_from_business_value(insert)?;
        let create_key = binding.key_from_document(&insert_document)?;
        if create_key != key {
            return Err(ServiceDbError::Decode(format!(
                "db upsert insert value key field {} must match selector key",
                binding.key_field
            )));
        }
        insert_document.remove("_id");
        let change = binding.validated_change(type_name, change)?;
        for field in change.touched_fields() {
            insert_document.remove(skiff_runtime_boundary::db::top_level_field(field));
        }
        let mut update = binding.change_update_document(&change)?;
        update.insert("$setOnInsert", Bson::Document(insert_document));
        let (inserted, value) = {
            let mut executor = self
                .mongo_executor(&binding.collection_name, session)
                .await?;
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
            if executor.has_session() {
                let result = executor
                    .update_one_upsert(guarded_filter.clone(), update)
                    .await?;
                let value = executor
                    .find_one(MongoFindOnePlan {
                        filter: guarded_filter,
                        ..Default::default()
                    })
                    .await?
                    .map(|document| binding.business_value_from_document(document))
                    .transpose()?;
                (result.upserted_id.is_some(), value)
            } else {
                let collection = executor.collection.clone();
                let result = match collection
                    .update_one(guarded_filter.clone(), update.clone())
                    .upsert(true)
                    .await
                {
                    Ok(result) => result,
                    Err(error) if is_mongo_duplicate_key_error(&error) => {
                        if self
                            .find_one_by_key(type_name, key.clone(), None, None)
                            .await?
                            .is_none()
                        {
                            return Err(error.into());
                        }
                        let retry_update = update_without_set_on_insert(&update);
                        if let Some(retry_update) = retry_update {
                            let retry_result = collection
                                .update_one(guarded_filter.clone(), retry_update)
                                .await
                                .map_err(ServiceDbError::from)?;
                            if retry_result.matched_count == 0 {
                                let mut retry_executor =
                                    self.mongo_executor(&binding.collection_name, None).await?;
                                self.assert_lease_guards_live(
                                    binding,
                                    &filter,
                                    lease_guards,
                                    &mut retry_executor,
                                )
                                .await?;
                                return Err(ServiceDbError::Decode(format!(
                                    "db upsert duplicate-key retry did not match {type_name}"
                                )));
                            }
                        }
                        let value = self.find_one_by_key(type_name, key, None, None).await?;
                        let value = value.ok_or_else(|| {
                            ServiceDbError::Decode(format!(
                                "db upsert duplicate-key retry did not materialize {type_name}"
                            ))
                        })?;
                        return Ok(DbWriteResult::new(
                            serde_json::json!({ "value": value, "inserted": false }),
                        ));
                    }
                    Err(error) => return Err(error.into()),
                };
                let value = self.find_one_by_key(type_name, key, None, None).await?;
                (result.upserted_id.is_some(), value)
            }
        };
        let value = value.ok_or_else(|| {
            ServiceDbError::Decode(format!("db upsert did not materialize {type_name}"))
        })?;
        Ok(DbWriteResult::new(
            serde_json::json!({ "value": value, "inserted": inserted }),
        ))
    }

    pub async fn replace_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: DbDocument,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none()
            && (binding.has_immutable_file_cascade()
                || has_matching_lease_guards(binding, lease_guards))
        {
            let mut session = self.start_transaction().await?;
            let result = self
                .replace_one_inner(binding, selector, value, lease_guards, Some(&mut session))
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.replace_one_inner(binding, selector, value, lease_guards, session)
            .await
    }

    async fn replace_one_inner(
        &self,
        binding: &DbCollectionMetadata,
        selector: DbOneSelector,
        value: DbDocument,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let key_selector = match &selector {
            DbOneSelector::Key(key) => Some(key),
            DbOneSelector::Query { .. } => None,
        };
        let mut replacement =
            binding.replacement_document_from_business_value(value, key_selector)?;
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_document = if binding.has_immutable_file_cascade()
            || has_matching_lease_guards(binding, lease_guards)
        {
            executor
                .find_one(MongoFindOnePlan {
                    filter: filter.clone(),
                    sort: sort.clone(),
                    ..Default::default()
                })
                .await?
        } else {
            None
        };
        if let Some(Bson::Document(leases)) = old_document
            .as_ref()
            .and_then(|document| document.get(SKIFF_LEASES_FIELD))
        {
            replacement.insert(SKIFF_LEASES_FIELD, Bson::Document(leases.clone()));
        }
        let document = executor
            .find_one_and_replace(
                MongoOneWritePlan {
                    filter: guarded_filter,
                    sort,
                },
                replacement.clone(),
            )
            .await?;
        if document.is_none() {
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(old_document) = &old_document {
            self.delete_skiff_files_by_plan(
                cascade_plan_for_replacement(
                    old_document,
                    &replacement,
                    &binding.immutable_file_paths,
                ),
                executor.session_mut(),
            )
            .await?;
        }
        document
            .map(|document| binding.business_value_from_document(document))
            .transpose()
    }

    pub async fn replace_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none() {
            let mut session = self.start_transaction().await?;
            let result = self
                .replace_one_runtime_inner(
                    binding,
                    selector,
                    value,
                    heap,
                    context,
                    lease_guards,
                    Some(&mut session),
                )
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.replace_one_runtime_inner(
            binding,
            selector,
            value,
            heap,
            context,
            lease_guards,
            session,
        )
        .await
    }

    async fn replace_one_runtime_inner(
        &self,
        binding: &DbCollectionMetadata,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
        lease_guards: &[DbLeaseHold],
        mut session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let key_selector = match &selector {
            DbOneSelector::Key(key) => Some(key),
            DbOneSelector::Query { .. } => None,
        };
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let mut replacement = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.document_from_runtime_business_value(value, heap, Some(&mut write_context))?
        };
        if let Some(key_selector) = key_selector {
            let create_key = binding.key_from_document(&replacement)?;
            if &create_key != key_selector {
                return Err(ServiceDbError::Decode(format!(
                    "db replace value key field {} must match selected object",
                    binding.key_field
                )));
            }
        }
        self.persist_recoverable_artifact_retention_roots(
            &root_store.roots,
            session.as_deref_mut(),
        )
        .await?;
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_document = if binding.has_immutable_file_cascade()
            || has_matching_lease_guards(binding, lease_guards)
        {
            executor
                .find_one(MongoFindOnePlan {
                    filter: filter.clone(),
                    sort: sort.clone(),
                    ..Default::default()
                })
                .await?
        } else {
            None
        };
        if let Some(Bson::Document(leases)) = old_document
            .as_ref()
            .and_then(|document| document.get(SKIFF_LEASES_FIELD))
        {
            replacement.insert(SKIFF_LEASES_FIELD, Bson::Document(leases.clone()));
        }
        let document = executor
            .find_one_and_replace(
                MongoOneWritePlan {
                    filter: guarded_filter,
                    sort,
                },
                replacement.clone(),
            )
            .await?;
        if document.is_none() {
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(old_document) = &old_document {
            self.delete_skiff_files_by_plan(
                cascade_plan_for_replacement(
                    old_document,
                    &replacement,
                    &binding.immutable_file_paths,
                ),
                executor.session_mut(),
            )
            .await?;
        }
        let read_context = recoverable_read_context(&context);
        document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    pub async fn delete_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<bool> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none()
            && (binding.has_immutable_file_cascade()
                || has_matching_lease_guards(binding, lease_guards))
        {
            let mut session = self.start_transaction().await?;
            let result = self
                .delete_one_inner(binding, selector, lease_guards, Some(&mut session))
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.delete_one_inner(binding, selector, lease_guards, session)
            .await
    }

    async fn delete_one_inner(
        &self,
        binding: &DbCollectionMetadata,
        selector: DbOneSelector,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<bool> {
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let deleted = executor
            .find_one_and_delete(MongoOneWritePlan {
                filter: guarded_filter,
                sort,
            })
            .await?;
        if deleted.is_none() {
            self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
                .await?;
        }
        if let Some(deleted) = &deleted {
            self.delete_skiff_files_by_plan(
                cascade_plan_for_deleted_documents(
                    std::slice::from_ref(deleted),
                    &binding.immutable_file_paths,
                ),
                executor.session_mut(),
            )
            .await?;
        }
        Ok(deleted.is_some())
    }

    pub async fn delete_many(
        &self,
        type_name: &str,
        query: DbQuery,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none()
            && (binding.has_immutable_file_cascade()
                || has_matching_lease_guards(binding, lease_guards))
        {
            let mut session = self.start_transaction().await?;
            let result = self
                .delete_many_inner(binding, query, lease_guards, Some(&mut session))
                .await;
            return self.finish_transaction(session, result, lease_guards).await;
        }
        self.delete_many_inner(binding, query, lease_guards, session)
            .await
    }

    async fn delete_many_inner(
        &self,
        binding: &DbCollectionMetadata,
        query: DbQuery,
        lease_guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<DbWriteResult> {
        let filter = binding.query_filter(query)?;
        let guarded_filter =
            guarded_filter(binding, filter.clone(), lease_guards, service_db_now_ms())?;
        let mut executor = self
            .mongo_executor(&binding.collection_name, session)
            .await?;
        self.assert_lease_guards_live(binding, &filter, lease_guards, &mut executor)
            .await?;
        let old_documents = if binding.has_immutable_file_cascade() {
            executor
                .find_many(MongoFindManyPlan {
                    filter: filter.clone(),
                    ..Default::default()
                })
                .await?
        } else {
            Vec::new()
        };
        let result = executor.delete_many(guarded_filter).await?;
        self.delete_skiff_files_by_plan(
            cascade_plan_for_deleted_documents(&old_documents, &binding.immutable_file_paths),
            executor.session_mut(),
        )
        .await?;
        Ok(DbWriteResult::new(
            serde_json::json!({ "deletedCount": result.deleted_count }),
        ))
    }

    pub async fn insert_skiff_file_record(
        &self,
        record: FileCapabilityRecord,
        session: Option<&mut ClientSession>,
    ) -> Result<()> {
        let document = skiff_file_record_document(record);
        self.mongo_executor(SKIFF_FILE_COLLECTION, session)
            .await?
            .insert_one(document)
            .await
    }

    pub async fn find_skiff_file_by_id(
        &self,
        id: &str,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<FileCapabilityRecord>> {
        let document = self
            .mongo_executor(SKIFF_FILE_COLLECTION, session)
            .await?
            .find_one(MongoFindOnePlan {
                filter: doc! { SKIFF_FILE_ID_FIELD: id },
                ..Default::default()
            })
            .await?;
        document.map(skiff_file_record_from_document).transpose()
    }

    pub async fn delete_skiff_file_by_id(
        &self,
        id: &str,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        self.delete_skiff_files_by_ids([id.to_string()], session)
            .await
    }

    pub async fn create(
        &self,
        type_name: &str,
        value: DbDocument,
        session: Option<&mut ClientSession>,
    ) -> Result<DbDocument> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let (document, materialized) = binding.document_from_business_value(value)?;
        self.mongo_executor(&binding.collection_name, session)
            .await?
            .insert_one(document)
            .await?;
        Ok(materialized)
    }

    pub async fn create_runtime(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
        session: Option<&mut ClientSession>,
    ) -> Result<RuntimeValue> {
        let binding = self.metadata.collection_for_type(type_name)?;
        if session.is_none() {
            let mut transaction = self.start_transaction().await?;
            let result = self
                .create_runtime_inner(binding, value, heap, context, Some(&mut transaction))
                .await;
            return self.finish_transaction(transaction, result, &[]).await;
        }
        self.create_runtime_inner(binding, value, heap, context, session)
            .await
    }

    async fn create_runtime_inner(
        &self,
        binding: &DbCollectionMetadata,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
        mut session: Option<&mut ClientSession>,
    ) -> Result<RuntimeValue> {
        let artifact_store = CurrentRequestRecoverableArtifactStore::new(&context);
        let mut root_store = CollectedRecoverableRootStore::default();
        let document = {
            let mut write_context =
                recoverable_write_context(&context, &artifact_store, &mut root_store);
            binding.document_from_runtime_business_value(value, heap, Some(&mut write_context))?
        };
        self.persist_recoverable_artifact_retention_roots(
            &root_store.roots,
            session.as_deref_mut(),
        )
        .await?;
        self.mongo_executor(&binding.collection_name, session)
            .await?
            .insert_one(document)
            .await?;
        Ok(value.clone())
    }

    pub async fn count(
        &self,
        type_name: &str,
        query: DbQuery,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let filter = binding.query_filter(query)?;
        let count = self
            .mongo_executor(&binding.collection_name, session)
            .await?
            .count_documents(filter)
            .await?;
        Ok(count)
    }

    pub async fn claim_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
        owner: &str,
        request_id: &str,
        now_ms: i64,
    ) -> Result<Option<DbLeaseHandle>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        let lease = binding.lease(slot)?;
        let key_bson = key_bson(binding, &key)?;
        let token = uuid::Uuid::new_v4().to_string();
        let max_expires_at_ms = lease.max_ms.map(|max_ms| add_ms(now_ms, max_ms));
        let expires_at_ms = lease_claim_expires_at_ms(now_ms, lease.ttl_ms, max_expires_at_ms);
        let mut set = Document::new();
        set.insert(lease_field(slot, LEASE_TOKEN_FIELD), token.clone());
        set.insert(lease_field(slot, LEASE_OWNER_FIELD), owner);
        set.insert(lease_field(slot, LEASE_REQUEST_ID_FIELD), request_id);
        set.insert(lease_field(slot, LEASE_CLAIMED_AT_MS_FIELD), now_ms);
        set.insert(lease_field(slot, LEASE_EXPIRES_AT_MS_FIELD), expires_at_ms);
        if let Some(max_expires_at_ms) = max_expires_at_ms {
            set.insert(
                lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD),
                max_expires_at_ms,
            );
        } else {
            set.insert(lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD), Bson::Null);
        }
        let filter = doc! {
            "$and": [
                doc! { "_id": key_bson.clone() },
                lease_available_filter(slot, now_ms),
            ]
        };
        let update = doc! { "$set": set };
        let document = self
            .mongo_executor(&binding.collection_name, None)
            .await?
            .find_one_and_update(MongoOneWritePlan { filter, sort: None }, update)
            .await?;
        let Some(document) = document else {
            return Ok(None);
        };
        let value = binding.business_value_from_document(document)?;
        let type_name = binding
            .canonical_type_name()
            .unwrap_or_else(|| type_name.to_string());
        Ok(Some(DbLeaseHandle {
            hold: DbLeaseHold {
                type_name,
                key,
                slot: slot.to_string(),
                token,
            },
            value,
            ttl_ms: lease.ttl_ms,
        }))
    }

    pub async fn renew_lease(&self, hold: &DbLeaseHold, now_ms: i64) -> Result<bool> {
        let binding = self.metadata.collection_for_type(&hold.type_name)?;
        let lease = binding.lease(&hold.slot)?;
        let key_bson = key_bson(binding, &hold.key)?;
        let max_path = lease_field(&hold.slot, LEASE_MAX_EXPIRES_AT_MS_FIELD);
        let max_expires_at_ms = self
            .read_lease_max_expires_at_ms(binding, &hold.slot, key_bson.clone(), &hold.token)
            .await?;
        let expires_at_ms = max_expires_at_ms
            .map(|max| add_ms(now_ms, lease.ttl_ms).min(max))
            .unwrap_or_else(|| add_ms(now_ms, lease.ttl_ms));
        if max_expires_at_ms.is_some_and(|max| now_ms >= max) {
            return Ok(false);
        }
        let mut max_null = Document::new();
        max_null.insert(max_path.clone(), Bson::Null);
        let mut max_live = Document::new();
        max_live.insert(max_path, doc! { "$gt": now_ms });
        let mut filter = doc! {
            "_id": key_bson,
            "$or": [max_null, max_live],
        };
        filter.insert(
            lease_field(&hold.slot, LEASE_TOKEN_FIELD),
            hold.token.clone(),
        );
        let mut set = Document::new();
        set.insert(
            lease_field(&hold.slot, LEASE_EXPIRES_AT_MS_FIELD),
            expires_at_ms,
        );
        let update = doc! { "$set": set };
        Ok(self
            .mongo_executor(&binding.collection_name, None)
            .await?
            .find_one_and_update(MongoOneWritePlan { filter, sort: None }, update)
            .await?
            .is_some())
    }

    pub async fn release_lease(&self, hold: &DbLeaseHold) -> Result<()> {
        let binding = self.metadata.collection_for_type(&hold.type_name)?;
        let mut filter = doc! { "_id": key_bson(binding, &hold.key)? };
        filter.insert(
            lease_field(&hold.slot, LEASE_TOKEN_FIELD),
            hold.token.clone(),
        );
        let mut unset = Document::new();
        unset.insert(lease_slot_path(&hold.slot), "");
        let update = doc! { "$unset": unset };
        let _ = self
            .mongo_executor(&binding.collection_name, None)
            .await?
            .find_one_and_update(MongoOneWritePlan { filter, sort: None }, update)
            .await?;
        Ok(())
    }

    pub async fn read_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
        now_ms: i64,
    ) -> Result<Option<Value>> {
        let binding = self.metadata.collection_for_type(type_name)?;
        binding.lease(slot)?;
        let filter = binding.key_filter(&key)?;
        let document = self
            .mongo_executor(&binding.collection_name, None)
            .await?
            .find_one(MongoFindOnePlan {
                filter,
                ..Default::default()
            })
            .await?;
        let Some(document) = document else {
            return Ok(None);
        };
        let Some(lease) = lease_document(&document, slot) else {
            return Ok(None);
        };
        let Some(expires_at_ms) = lease_i64(lease, LEASE_EXPIRES_AT_MS_FIELD) else {
            return Ok(None);
        };
        if expires_at_ms <= now_ms
            || lease_i64(lease, LEASE_MAX_EXPIRES_AT_MS_FIELD).is_some_and(|max| max <= now_ms)
        {
            return Ok(None);
        }
        let owner = lease
            .get_str(LEASE_OWNER_FIELD)
            .unwrap_or_default()
            .to_string();
        let request_id = lease
            .get_str(LEASE_REQUEST_ID_FIELD)
            .unwrap_or_default()
            .to_string();
        Ok(Some(json!({
            "owner": owner,
            "requestId": request_id,
            "expiresAt": date_value::format_epoch_millis(expires_at_ms, "db lease expiresAt")?,
        })))
    }

    async fn read_lease_max_expires_at_ms(
        &self,
        binding: &DbCollectionMetadata,
        slot: &str,
        key_bson: Bson,
        token: &str,
    ) -> Result<Option<i64>> {
        let document = self
            .mongo_executor(&binding.collection_name, None)
            .await?
            .find_one(MongoFindOnePlan {
                filter: {
                    let mut filter = doc! { "_id": key_bson };
                    filter.insert(lease_field(slot, LEASE_TOKEN_FIELD), token);
                    filter
                },
                ..Default::default()
            })
            .await?;
        Ok(document
            .as_ref()
            .and_then(|document| lease_document(document, slot))
            .and_then(|lease| lease_i64(lease, LEASE_MAX_EXPIRES_AT_MS_FIELD)))
    }

    async fn assert_lease_guards_live(
        &self,
        binding: &DbCollectionMetadata,
        filter: &Document,
        guards: &[DbLeaseHold],
        executor: &mut MongoSessionExecutor<'_>,
    ) -> Result<()> {
        for guard in matching_lease_guards(binding, guards) {
            let key_bson = key_bson(binding, &guard.key)?;
            let document = executor
                .find_one(MongoFindOnePlan {
                    filter: and_filter(filter.clone(), vec![doc! { "_id": key_bson }]),
                    ..Default::default()
                })
                .await?;
            if document.is_some() {
                self.assert_lease_hold_live(binding, guard, executor)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn assert_lease_holds_live(
        &self,
        guards: &[DbLeaseHold],
        session: Option<&mut ClientSession>,
    ) -> Result<()> {
        let Some(session) = session else {
            for guard in guards {
                let binding = self.metadata.collection_for_type(&guard.type_name)?;
                let mut executor = self.mongo_executor(&binding.collection_name, None).await?;
                self.assert_lease_hold_live(binding, guard, &mut executor)
                    .await?;
            }
            return Ok(());
        };
        for guard in guards {
            let binding = self.metadata.collection_for_type(&guard.type_name)?;
            let mut executor = self
                .mongo_executor(&binding.collection_name, Some(&mut *session))
                .await?;
            self.assert_lease_hold_live(binding, guard, &mut executor)
                .await?;
        }
        Ok(())
    }

    async fn assert_lease_hold_live(
        &self,
        binding: &DbCollectionMetadata,
        guard: &DbLeaseHold,
        executor: &mut MongoSessionExecutor<'_>,
    ) -> Result<()> {
        let key_bson = key_bson(binding, &guard.key)?;
        let filter =
            lease_live_key_filter(&guard.slot, key_bson, &guard.token, service_db_now_ms());
        let update = doc! {
            "$set": {
                lease_field(&guard.slot, LEASE_TOKEN_FIELD): guard.token.clone(),
            },
        };
        if executor
            .find_one_and_update(MongoOneWritePlan { filter, sort: None }, update)
            .await?
            .is_some()
        {
            Ok(())
        } else {
            Err(lease_lost_error(&guard.type_name, &guard.slot))
        }
    }

    async fn collection(&self, collection_name: &str) -> Result<Collection<Document>> {
        let client = self.client().await?;
        Ok(client
            .database(&self.database_name)
            .collection::<Document>(collection_name))
    }

    async fn mongo_executor<'a>(
        &self,
        collection_name: &str,
        session: Option<&'a mut ClientSession>,
    ) -> Result<MongoSessionExecutor<'a>> {
        Ok(MongoSessionExecutor::new(
            self.collection(collection_name).await?,
            session,
        ))
    }

    async fn start_transaction(&self) -> Result<ClientSession> {
        let client = self.client().await?;
        let mut session = client.start_session().await?;
        session.start_transaction().await?;
        Ok(session)
    }

    async fn finish_transaction<T>(
        &self,
        mut session: ClientSession,
        result: Result<T>,
        lease_guards: &[DbLeaseHold],
    ) -> Result<T> {
        match result {
            Ok(value) => {
                if let Err(error) = self
                    .assert_lease_holds_live(lease_guards, Some(&mut session))
                    .await
                {
                    let _ = session.abort_transaction().await;
                    return Err(error);
                }
                if let Err(error) = session.commit_transaction().await {
                    let _ = session.abort_transaction().await;
                    Err(error.into())
                } else {
                    Ok(value)
                }
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                Err(error)
            }
        }
    }

    async fn delete_skiff_files_by_plan(
        &self,
        plan: CascadeFileDeletePlan,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        self.delete_skiff_files_by_ids(plan.file_ids, session).await
    }

    async fn delete_skiff_files_by_ids(
        &self,
        file_ids: impl IntoIterator<Item = String>,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        let file_ids = file_ids.into_iter().collect::<Vec<_>>();
        if file_ids.is_empty() {
            return Ok(0);
        }
        let result = self
            .mongo_executor(SKIFF_FILE_COLLECTION, session)
            .await?
            .delete_many(doc! { SKIFF_FILE_ID_FIELD: { "$in": file_ids } })
            .await?;
        Ok(result.deleted_count)
    }

    async fn find_one_document(
        &self,
        binding: &DbCollectionMetadata,
        filter: Document,
        sort: Option<Document>,
        projection: Option<&[FieldPath]>,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<Document>> {
        let projection = binding.projection_document(projection)?;
        self.mongo_executor(&binding.collection_name, session)
            .await?
            .find_one(MongoFindOnePlan {
                filter,
                sort,
                projection,
            })
            .await
    }

    async fn find_one_for_selector(
        &self,
        binding: &DbCollectionMetadata,
        selector: DbOneSelector,
        projection: Option<&[FieldPath]>,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<DbDocument>> {
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let document = self
            .find_one_document(binding, filter, sort, projection, session)
            .await?;
        document
            .map(|document| binding.business_value_from_document(document))
            .transpose()
    }

    async fn find_one_for_selector_runtime(
        &self,
        binding: &DbCollectionMetadata,
        selector: DbOneSelector,
        projection: Option<&[FieldPath]>,
        heap: &mut RequestHeap,
        context: &DbRecoverableRuntimeContext,
        session: Option<&mut ClientSession>,
    ) -> Result<Option<RuntimeValue>> {
        let (filter, sort) = binding.selector_filter_sort(selector)?;
        let document = self
            .find_one_document(binding, filter, sort, projection, session)
            .await?;
        let read_context = recoverable_read_context(context);
        document
            .map(|document| {
                binding.runtime_business_value_from_document(document, heap, Some(&read_context))
            })
            .transpose()
    }

    async fn persist_recoverable_artifact_retention_roots(
        &self,
        roots: &[RecoverableArtifactRetentionRoot],
        session: Option<&mut ClientSession>,
    ) -> Result<()> {
        if roots.is_empty() {
            return Ok(());
        }
        let now_ms = service_db_now_ms();
        let mut executor = self
            .mongo_executor(SKIFF_RECOVERABLE_ARTIFACT_ROOT_COLLECTION, session)
            .await?;
        for root in roots {
            let filter = doc! { "_id": recoverable_retention_root_id(root) };
            let update = doc! {
                "$set": {
                    "serviceId": root.service_id.clone(),
                    "artifactIdentity": root.artifact_identity.clone(),
                    "buildId": root.build_id.clone(),
                    "boundaryKind": root.boundary_kind.as_str(),
                    "expiresAtEpochMillis": root.expires_at_epoch_millis.map(Bson::Int64).unwrap_or(Bson::Null),
                    "updatedAtEpochMillis": now_ms,
                },
                "$setOnInsert": {
                    "createdAtEpochMillis": now_ms,
                }
            };
            executor.update_one_upsert(filter, update).await?;
        }
        Ok(())
    }

    async fn count_documents(
        &self,
        binding: &DbCollectionMetadata,
        filter: Document,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        let count = self
            .mongo_executor(&binding.collection_name, session)
            .await?
            .count_documents(filter)
            .await?;
        Ok(count)
    }

    async fn insert_many_count(
        &self,
        binding: &DbCollectionMetadata,
        documents: Vec<Document>,
        session: Option<&mut ClientSession>,
    ) -> Result<u64> {
        if documents.is_empty() {
            return Ok(0);
        }
        let result = self
            .mongo_executor(&binding.collection_name, session)
            .await?
            .insert_many(documents)
            .await?;
        Ok(result.inserted_ids.len() as u64)
    }
}

fn service_db_client_cell(mongo_url: &str) -> Arc<OnceCell<Client>> {
    let cells = SERVICE_DB_CLIENT_CELLS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cells = cells
        .lock()
        .expect("service DB Mongo client cache lock should not be poisoned");
    cells.retain(|_, cell| cell.strong_count() > 0);
    if let Some(cell) = cells.get(mongo_url).and_then(Weak::upgrade) {
        return cell;
    }
    let cell = Arc::new(OnceCell::new());
    cells.insert(mongo_url.to_string(), Arc::downgrade(&cell));
    cell
}

fn service_id_storage_database_name(service_id: &str) -> Result<String> {
    validate_publication_id(service_id)?;
    Ok(service_id.replace('.', "~").replace('/', "~~"))
}

fn validate_publication_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 63
        || value == "std"
        || value != value.trim()
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    }
    let Some((authority, local)) = value.split_once('/') else {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    };
    validate_publication_authority(value, authority)?;
    if local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    }
    Ok(())
}

fn validate_publication_authority(publication_id: &str, authority: &str) -> Result<()> {
    let labels = authority.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|label| !is_valid_authority_label(label)) {
        return Err(ServiceDbError::Decode(format!(
            "service id `{publication_id}` must be a publication id"
        )));
    }
    Ok(())
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

fn validate_service_database_name(database_name: &str) -> Result<()> {
    if database_name.is_empty() || database_name.len() >= 64 {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` must project to a Mongo database name of 1-63 bytes"
        )));
    }
    if matches!(database_name, "admin" | "local" | "config") {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` projects to a reserved Mongo database name"
        )));
    }
    if database_name
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || database_name.contains(['.', '/', '\\', '"', '$'])
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` projects to a character forbidden in Mongo database names"
        )));
    }
    Ok(())
}

async fn service_db_client_options(mongo_url: &str) -> mongodb::error::Result<ClientOptions> {
    let mut options = ClientOptions::parse(mongo_url).await?;
    if options.direct_connection == Some(true) {
        options.repl_set_name = None;
    }
    options.retry_writes = Some(false);
    Ok(options)
}

fn skiff_file_record_document(record: FileCapabilityRecord) -> Document {
    let mut document = Document::new();
    document.insert("_id", record.id.clone());
    document.insert(SKIFF_FILE_ID_FIELD, record.id);
    document.insert("sha256", record.sha256);
    document.insert("size", Bson::Int64(record.size));
    if let Some(content_type) = record.content_type {
        document.insert("content_type", content_type);
    }
    if let Some(purpose) = record.purpose {
        document.insert("purpose", purpose);
    }
    document.insert("blob_key", record.blob_key);
    document.insert("created_at", record.created_at);
    document
}

fn skiff_file_record_from_document(document: Document) -> Result<FileCapabilityRecord> {
    let id = document
        .get_str(SKIFF_FILE_ID_FIELD)
        .map(str::to_string)
        .map_err(|error| ServiceDbError::Decode(format!("_skiff_file.id is invalid: {error}")))?;
    let sha256 = document
        .get_str("sha256")
        .map(str::to_string)
        .map_err(|error| {
            ServiceDbError::Decode(format!("_skiff_file.sha256 is invalid: {error}"))
        })?;
    let size = match document.get("size") {
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Int64(value)) => *value,
        Some(Bson::Double(value)) if value.fract() == 0.0 => *value as i64,
        other => {
            return Err(ServiceDbError::Decode(format!(
                "_skiff_file.size is invalid: {other:?}"
            )))
        }
    };
    let content_type = optional_document_string(&document, "content_type")?;
    let purpose = optional_document_string(&document, "purpose")?;
    let blob_key = document
        .get_str("blob_key")
        .map(str::to_string)
        .map_err(|error| {
            ServiceDbError::Decode(format!("_skiff_file.blob_key is invalid: {error}"))
        })?;
    let created_at = document
        .get_str("created_at")
        .map(str::to_string)
        .map_err(|error| {
            ServiceDbError::Decode(format!("_skiff_file.created_at is invalid: {error}"))
        })?;
    Ok(FileCapabilityRecord {
        id,
        sha256,
        size,
        content_type,
        purpose,
        blob_key,
        created_at,
    })
}

fn optional_document_string(document: &Document, field: &str) -> Result<Option<String>> {
    match document.get(field) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::String(value)) => Ok(Some(value.clone())),
        other => Err(ServiceDbError::Decode(format!(
            "_skiff_file.{field} is invalid: {other:?}"
        ))),
    }
}

fn recoverable_write_context<'a>(
    context: &'a DbRecoverableRuntimeContext,
    artifact_store: &'a CurrentRequestRecoverableArtifactStore,
    root_store: &'a mut CollectedRecoverableRootStore,
) -> mapping::DbRecoverableRuntimeWriteContext<'a> {
    mapping::DbRecoverableRuntimeWriteContext {
        behavior_hooks: context.behavior_hooks.as_ref(),
        boundary_context: Some(&context.boundary_context),
        recoverable_expected_override: None,
        recoverable_expected_overrides: Some(context.expected_plans.fields()),
        artifact_store: Some(artifact_store),
        retention_root_store: Some(root_store),
        retention_expires_at_epoch_millis: context.retention_expires_at_epoch_millis,
    }
}

fn recoverable_read_context(
    context: &DbRecoverableRuntimeContext,
) -> mapping::DbRecoverableRuntimeReadContext<'_> {
    mapping::DbRecoverableRuntimeReadContext {
        behavior_hooks: context.behavior_hooks.as_ref(),
        boundary_context: Some(&context.boundary_context),
        recoverable_expected_override: None,
        recoverable_expected_overrides: Some(context.expected_plans.fields()),
    }
}

fn recoverable_retention_root_id(root: &RecoverableArtifactRetentionRoot) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.service_id.as_bytes());
    hasher.update([0]);
    hasher.update(root.artifact_identity.as_bytes());
    hasher.update([0]);
    hasher.update(root.build_id.as_bytes());
    hasher.update([0]);
    hasher.update(root.boundary_kind.as_str().as_bytes());
    hasher.update([0]);
    if let Some(expires_at) = root.expires_at_epoch_millis {
        hasher.update(expires_at.to_be_bytes());
    }
    format!("recoverable-root:sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests;
