use skiff_runtime_capability_context::{
    DbCapabilityResult, DbKey, DbOneSelector, DbOrderEntry, DbQuery, DbRuntimeChange, FieldPath,
    PreparedDbManyRuntimeOperation, PreparedDbOptionalRuntimeOperation,
    PreparedDbValueRuntimeOperation, ServiceDbFindOptions,
};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

#[cfg(test)]
use super::PreparedRuntimeTestKind;
use super::{
    capability_error,
    create::{CompletedCreate, PreparedCreate},
    read::{CompletedFindMany, CompletedFindOne, PreparedFindMany, PreparedFindOne},
    replace::{is_lease_lost as replace_lease_lost, CompletedReplace, PreparedReplace},
    update::{is_lease_lost as update_lease_lost, CompletedUpdate, PreparedUpdate},
};
use crate::{Result, ServiceDbStore};

impl ServiceDbStore {
    pub(crate) fn prepare_find_one_by_key_runtime_operation(
        &self,
        type_name: &str,
        key: DbKey,
        projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_find_one_by_key_runtime_command(type_name, key, projection, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbOptionalRuntimeOperation::new(async move {
            let completion = store
                .wait_find_one_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) fn prepare_find_one_by_query_runtime_operation(
        &self,
        type_name: &str,
        query: DbQuery,
        order: Vec<DbOrderEntry>,
        projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_find_one_by_query_runtime_command(type_name, query, order, projection, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbOptionalRuntimeOperation::new(async move {
            let completion = store
                .wait_find_one_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) fn prepare_find_many_page_runtime_operation(
        &self,
        type_name: &str,
        query: DbQuery,
        options: ServiceDbFindOptions,
        projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbManyRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_find_many_page_runtime_command(type_name, query, options, projection, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbManyRuntimeOperation::new(async move {
            let completion = store
                .wait_find_many_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) fn prepare_create_runtime_operation(
        &self,
        type_name: &str,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_create_runtime_command(type_name, value, heap, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbValueRuntimeOperation::new(async move {
            let completion = store
                .wait_create_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) fn prepare_update_one_runtime_operation(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        change: DbRuntimeChange,
        heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_update_one_runtime_command(type_name, selector, change, heap, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbOptionalRuntimeOperation::new(async move {
            let completion = store
                .wait_update_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) fn prepare_replace_one_runtime_operation(
        &self,
        type_name: &str,
        selector: DbOneSelector,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        context: crate::DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let runtime = self.runtime_owner();
        let command = runtime
            .prepare_replace_one_runtime_command(type_name, selector, value, heap, context)
            .map_err(capability_error)?;
        let store = self.clone();
        Ok(PreparedDbOptionalRuntimeOperation::new(async move {
            let completion = store
                .wait_replace_runtime(command)
                .await
                .map_err(capability_error)?;
            Ok(completion.into_finalizer(runtime))
        }))
    }

    pub(crate) async fn wait_find_one_runtime(
        &self,
        command: PreparedFindOne,
    ) -> Result<CompletedFindOne> {
        let runtime = self.runtime_owner();
        let request_state = self.request_state_owner();
        let mut state = request_state.lock().await;
        state.ensure_lease_live()?;
        #[cfg(test)]
        if let Some(driver) = self.prepared_runtime_test_driver() {
            let kind = command.test_kind();
            return command.complete_for_test(driver.wait(kind).await?);
        }
        if let Some(transaction) = state.transaction.as_mut() {
            return command
                .execute(&runtime, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        command.execute(&runtime, None).await
    }

    pub(crate) async fn wait_find_many_runtime(
        &self,
        command: PreparedFindMany,
    ) -> Result<CompletedFindMany> {
        let runtime = self.runtime_owner();
        let request_state = self.request_state_owner();
        let mut state = request_state.lock().await;
        state.ensure_lease_live()?;
        if !command.requires_provider() {
            return command.execute(&runtime, None).await;
        }
        #[cfg(test)]
        if let Some(driver) = self.prepared_runtime_test_driver() {
            return command
                .complete_for_test(driver.wait(PreparedRuntimeTestKind::FindMany).await?);
        }
        if let Some(transaction) = state.transaction.as_mut() {
            return command
                .execute(&runtime, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        command.execute(&runtime, None).await
    }

    pub(crate) async fn wait_create_runtime(
        &self,
        command: PreparedCreate,
    ) -> Result<CompletedCreate> {
        let runtime = self.runtime_owner();
        let request_state = self.request_state_owner();
        let mut state = request_state.lock().await;
        state.ensure_lease_live()?;
        #[cfg(test)]
        if let Some(driver) = self.prepared_runtime_test_driver() {
            return command.complete_for_test(driver.wait(PreparedRuntimeTestKind::Create).await?);
        }
        if let Some(transaction) = state.transaction.as_mut() {
            return command
                .execute(&runtime, Some(&mut transaction.session))
                .await;
        }
        drop(state);
        command.execute(&runtime, None).await
    }

    pub(crate) async fn wait_update_runtime(
        &self,
        command: PreparedUpdate,
    ) -> Result<CompletedUpdate> {
        let runtime = self.runtime_owner();
        let request_state = self.request_state_owner();
        let mut state = request_state.lock().await;
        state.ensure_lease_live()?;
        #[cfg(test)]
        if let Some(driver) = self.prepared_runtime_test_driver() {
            return command.complete_for_test(driver.wait(PreparedRuntimeTestKind::Update).await?);
        }
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = command
                .execute(&runtime, &leases, Some(&mut transaction.session))
                .await;
            if update_lease_lost(&result) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        let result = command.execute(&runtime, &leases, None).await;
        if update_lease_lost(&result) {
            request_state.lock().await.lease_lost = true;
        }
        result
    }

    pub(crate) async fn wait_replace_runtime(
        &self,
        command: PreparedReplace,
    ) -> Result<CompletedReplace> {
        let runtime = self.runtime_owner();
        let request_state = self.request_state_owner();
        let mut state = request_state.lock().await;
        state.ensure_lease_live()?;
        #[cfg(test)]
        if let Some(driver) = self.prepared_runtime_test_driver() {
            return command.complete_for_test(driver.wait(PreparedRuntimeTestKind::Replace).await?);
        }
        let leases = state.leases.clone();
        if let Some(transaction) = state.transaction.as_mut() {
            let result = command
                .execute(&runtime, &leases, Some(&mut transaction.session))
                .await;
            if replace_lease_lost(&result) {
                state.lease_lost = true;
            }
            return result;
        }
        drop(state);
        let result = command.execute(&runtime, &leases, None).await;
        if replace_lease_lost(&result) {
            request_state.lock().await.lease_lost = true;
        }
        result
    }
}
