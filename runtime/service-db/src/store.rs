use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_capability_context::{
    DbDocument, DbKey, DbOneSelector, DbOrderEntry, DbPageResult, DbQuery, DbWriteResult,
    FieldPath, FileCapabilityRecord, ServiceDbChange, ServiceDbFindOptions,
};
use tokio::sync::Mutex;

use crate::{DbRecoverableRuntimeContext, DbRuntimeChange, Result, ServiceDbError};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::{
    service_db_now_ms, DbLeaseHandle, DbLeaseHold, DbRequestState, DbTransactionState,
    ServiceDbRuntime,
};

#[derive(Clone)]
pub struct ServiceDbStore {
    runtime: Arc<ServiceDbRuntime>,
    request_state: Arc<Mutex<DbRequestState>>,
}

impl ServiceDbStore {
    pub fn new(runtime: Arc<ServiceDbRuntime>, request_state: Arc<Mutex<DbRequestState>>) -> Self {
        Self {
            runtime,
            request_state,
        }
    }

    pub async fn begin_transaction(&self) -> Result<()> {
        {
            let state = self.request_state.lock().await;
            state.ensure_lease_live()?;
            if state.transaction.is_some() {
                return Err(nested_transaction_error());
            }
        }

        let client = self.runtime.client().await?;
        let mut session = client.start_session().await?;
        session.start_transaction().await?;

        let mut state = self.request_state.lock().await;
        if state.transaction.is_some() {
            drop(state);
            let _ = session.abort_transaction().await;
            return Err(nested_transaction_error());
        }
        state.transaction = Some(DbTransactionState { session });
        Ok(())
    }

    pub async fn commit_transaction(&self) -> Result<()> {
        let (mut transaction, leases) = {
            let mut state = self.request_state.lock().await;
            if let Err(error) = state.ensure_lease_live() {
                let transaction = state.transaction.take();
                drop(state);
                if let Some(mut transaction) = transaction {
                    let _ = transaction.session.abort_transaction().await;
                }
                return Err(error);
            }
            (
                state
                    .transaction
                    .take()
                    .ok_or_else(missing_transaction_error)?,
                state.leases.clone(),
            )
        };

        if let Err(error) = self
            .runtime
            .assert_lease_holds_live(&leases, Some(&mut transaction.session))
            .await
        {
            let _ = transaction.session.abort_transaction().await;
            if matches!(error, ServiceDbError::LeaseLost(_)) {
                self.mark_lease_lost().await;
            }
            return Err(error);
        }
        if let Err(error) = transaction.session.commit_transaction().await {
            let _ = transaction.session.abort_transaction().await;
            return Err(error.into());
        }
        Ok(())
    }

    pub async fn abort_transaction(&self) {
        let transaction = {
            let mut state = self.request_state.lock().await;
            state.transaction.take()
        };
        if let Some(mut transaction) = transaction {
            let _ = transaction.session.abort_transaction().await;
        }
    }

    pub async fn find_one_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
    ) -> Result<Option<DbDocument>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_one_by_key(type_name, key, projection, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime
            .find_one_by_key(type_name, key, projection, None)
            .await
    }

    pub async fn find_one_by_key_runtime(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<Option<RuntimeValue>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_one_by_key_runtime(
                    type_name,
                    key,
                    projection,
                    heap,
                    context,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .find_one_by_key_runtime(type_name, key, projection, heap, context, None)
            .await
    }

    pub async fn find_one_by_query(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
    ) -> Result<Option<DbDocument>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_one_by_query(
                    type_name,
                    query,
                    order,
                    projection,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .find_one_by_query(type_name, query, order, projection, None)
            .await
    }

    pub async fn find_one_by_query_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<Option<RuntimeValue>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_one_by_query_runtime(
                    type_name,
                    query,
                    order,
                    projection,
                    heap,
                    context,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .find_one_by_query_runtime(type_name, query, order, projection, heap, context, None)
            .await
    }

    pub async fn find_many_page(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
    ) -> Result<DbPageResult> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_many_page(
                    type_name,
                    query,
                    options,
                    projection,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .find_many_page(type_name, query, options, projection, None)
            .await
    }

    pub async fn find_many_page_runtime(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<Vec<RuntimeValue>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_many_page_runtime(
                    type_name,
                    query,
                    options,
                    projection,
                    heap,
                    context,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .find_many_page_runtime(type_name, query, options, projection, heap, context, None)
            .await
    }

    pub async fn exists_by_key(&self, type_name: &str, key: DbKey) -> Result<bool> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .exists_by_key(type_name, key, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.exists_by_key(type_name, key, None).await
    }

    pub async fn exists_by_query(&self, type_name: &str, query: DbQuery) -> Result<bool> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .exists_by_query(type_name, query, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.exists_by_query(type_name, query, None).await
    }

    pub async fn insert_many_result(
        &self,
        type_name: &str,
        values: Vec<DbDocument>,
    ) -> Result<DbWriteResult> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .insert_many_result(type_name, values, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime
            .insert_many_result(type_name, values, None)
            .await
    }

    pub async fn update_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: ServiceDbChange,
    ) -> Result<Option<DbDocument>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .update_one(
                    type_name,
                    selector,
                    change,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .update_one(type_name, selector, change, &leases, None)
                .await,
        )
        .await
    }

    pub async fn update_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<Option<RuntimeValue>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .update_one_runtime(
                    type_name,
                    selector,
                    change,
                    heap,
                    context,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .update_one_runtime(type_name, selector, change, heap, context, &leases, None)
                .await,
        )
        .await
    }

    pub async fn update_many(
        &self,
        type_name: &str,
        query: DbQuery,
        change: ServiceDbChange,
    ) -> Result<DbWriteResult> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .update_many(
                    type_name,
                    query,
                    change,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .update_many(type_name, query, change, &leases, None)
                .await,
        )
        .await
    }

    pub async fn upsert_by_key(
        &self,
        type_name: &str,
        key: DbKey,
        insert: DbDocument,
        change: ServiceDbChange,
    ) -> Result<DbWriteResult> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .upsert_by_key(
                    type_name,
                    key,
                    insert,
                    change,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .upsert_by_key(type_name, key, insert, change, &leases, None)
                .await,
        )
        .await
    }

    pub async fn replace_one(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: DbDocument,
    ) -> Result<Option<DbDocument>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .replace_one(
                    type_name,
                    selector,
                    value,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .replace_one(type_name, selector, value, &leases, None)
                .await,
        )
        .await
    }

    pub async fn replace_one_runtime(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<Option<RuntimeValue>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .replace_one_runtime(
                    type_name,
                    selector,
                    value,
                    heap,
                    context,
                    &leases,
                    Some(&mut transaction.session),
                )
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .replace_one_runtime(type_name, selector, value, heap, context, &leases, None)
                .await,
        )
        .await
    }

    pub async fn delete_one(&self, type_name: &str, selector: DbOneSelector) -> Result<bool> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .delete_one(type_name, selector, &leases, Some(&mut transaction.session))
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .delete_one(type_name, selector, &leases, None)
                .await,
        )
        .await
    }

    pub async fn delete_many(&self, type_name: &str, query: DbQuery) -> Result<DbWriteResult> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = self
                .runtime
                .delete_many(type_name, query, &leases, Some(&mut transaction.session))
                .await;
            if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        self.record_lease_result(
            self.runtime
                .delete_many(type_name, query, &leases, None)
                .await,
        )
        .await
    }

    pub async fn create(&self, type_name: &str, value: DbDocument) -> Result<DbDocument> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .create(type_name, value, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.create(type_name, value, None).await
    }

    pub async fn create_runtime(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &RequestHeap,
        context: DbRecoverableRuntimeContext,
    ) -> Result<RuntimeValue> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .create_runtime(
                    type_name,
                    value,
                    heap,
                    context,
                    Some(&mut transaction.session),
                )
                .await;
        }
        drop(state);
        self.runtime
            .create_runtime(type_name, value, heap, context, None)
            .await
    }

    pub async fn insert_skiff_file_record(&self, record: FileCapabilityRecord) -> Result<()> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .insert_skiff_file_record(record, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.insert_skiff_file_record(record, None).await
    }

    pub async fn find_skiff_file_by_id(&self, id: &str) -> Result<Option<FileCapabilityRecord>> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .find_skiff_file_by_id(id, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.find_skiff_file_by_id(id, None).await
    }

    pub async fn delete_skiff_file_by_id(&self, id: &str) -> Result<u64> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .delete_skiff_file_by_id(id, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.delete_skiff_file_by_id(id, None).await
    }

    pub async fn count(&self, type_name: &str, query: DbQuery) -> Result<u64> {
        let mut state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        if let Some(transaction) = state.transaction.as_mut() {
            return self
                .runtime
                .count(type_name, query, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        self.runtime.count(type_name, query, None).await
    }

    pub async fn claim_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> Result<Option<DbLeaseHandle>> {
        let (owner, request_id) = {
            let state = self.request_state.lock().await;
            state.ensure_lease_live()?;
            if state.transaction.is_some() {
                return Err(ServiceDbError::Decode(
                    "db claim is not allowed inside active db transaction".to_string(),
                ));
            }
            if state
                .leases
                .iter()
                .any(|hold| hold.type_name == type_name && hold.key == key && hold.slot == slot)
            {
                return Err(ServiceDbError::Decode(
                    "db claim cannot re-enter a lease already held by this request".to_string(),
                ));
            }
            (state.owner.clone(), state.request_id.clone())
        };
        let handle = self
            .runtime
            .claim_lease(
                type_name,
                key,
                slot,
                &owner,
                &request_id,
                service_db_now_ms(),
            )
            .await?;
        if let Some(handle) = &handle {
            let mut state = self.request_state.lock().await;
            state.ensure_lease_live()?;
            state.leases.push(handle.hold.clone());
        }
        Ok(handle)
    }

    pub async fn renew_lease(&self, hold: &DbLeaseHold) -> Result<bool> {
        match self.runtime.renew_lease(hold, service_db_now_ms()).await {
            Ok(true) => Ok(true),
            Ok(false) => {
                self.mark_lease_lost().await;
                Ok(false)
            }
            Err(error) => {
                self.mark_lease_lost().await;
                Err(error)
            }
        }
    }

    pub async fn release_lease(&self, hold: &DbLeaseHold) -> Result<()> {
        self.runtime.release_lease(hold).await?;
        let mut state = self.request_state.lock().await;
        state.leases.retain(|candidate| candidate != hold);
        Ok(())
    }

    pub async fn read_lease(
        &self,
        type_name: &str,
        key: DbKey,
        slot: &str,
    ) -> Result<Option<Value>> {
        let state = self.request_state.lock().await;
        state.ensure_lease_live()?;
        drop(state);
        self.runtime
            .read_lease(type_name, key, slot, service_db_now_ms())
            .await
    }

    pub async fn mark_lease_lost(&self) {
        let mut state = self.request_state.lock().await;
        state.lease_lost = true;
    }

    pub async fn lease_lost(&self) -> bool {
        self.request_state.lock().await.lease_lost
    }

    async fn record_lease_result<T>(&self, result: Result<T>) -> Result<T> {
        if matches!(result, Err(ServiceDbError::LeaseLost(_))) {
            self.mark_lease_lost().await;
        }
        result
    }
}

fn nested_transaction_error() -> ServiceDbError {
    ServiceDbError::Decode("nested db.transaction is not supported".to_string())
}

fn missing_transaction_error() -> ServiceDbError {
    ServiceDbError::Decode("db.transaction commit without active transaction".to_string())
}
