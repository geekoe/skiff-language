use std::{future::Future, pin::Pin, sync::Arc};

use serde_json::{json, Value};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use crate::db::*;

use super::{
    raw_read_api::impl_raw_read_api,
    raw_write_api::impl_raw_write_api,
    state::{test_hold, TestStoreState},
};

pub(super) struct PreparedFakeStore {
    state: Arc<TestStoreState>,
}

impl PreparedFakeStore {
    pub(super) fn new(state: Arc<TestStoreState>) -> Self {
        Self { state }
    }

    fn ready<T>(&self, value: T) -> PreparedDbRuntimeOperation<T>
    where
        T: Send + 'static,
    {
        let wait_state = Arc::clone(&self.state);
        PreparedDbRuntimeOperation::new(async move {
            wait_state.record_wait_start();
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |_heap| {
                finalize_state.record_finalize();
                Ok(value)
            }))
        })
    }
}

impl DbCapabilityStoreApi for PreparedFakeStore {
    impl_raw_read_api!();
    impl_raw_write_api!();

    fn prepare_find_one_by_key_runtime(
        &self,
        _type_name: &str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::String("key".to_string()))))
    }

    fn prepare_find_one_by_query_runtime(
        &self,
        _type_name: &str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::String("query".to_string()))))
    }

    fn prepare_find_many_page_runtime(
        &self,
        _type_name: &str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbManyRuntimeOperation> {
        Ok(self.ready(vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)]))
    }

    fn prepare_create_runtime(
        &self,
        _type_name: &str,
        value: &RuntimeValue,
        heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        if let RuntimeValue::Heap(handle) = value {
            heap.get(*handle)
                .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
        }
        let gate = self.state.take_create_gate();
        let wait_state = Arc::clone(&self.state);
        Ok(PreparedDbRuntimeOperation::new(async move {
            wait_state.record_wait_start();
            if let Some(gate) = gate {
                gate.await
                    .map_err(|_| DbCapabilityError::decode("create gate dropped"))?;
            }
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |heap| {
                finalize_state.record_finalize();
                if finalize_state.create_finalize_fails() {
                    heap.alloc_array(Vec::new())
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    heap.alloc_array(Vec::new())
                        .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                    unreachable!("second allocation must exceed the test heap limit");
                }
                let handle = heap
                    .alloc_array(vec![RuntimeValue::String("created".to_string())])
                    .map_err(|error| DbCapabilityError::decode(error.to_string()))?;
                Ok(RuntimeValue::Heap(handle))
            }))
        }))
    }

    fn prepare_update_one_runtime(
        &self,
        _type_name: &str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        Ok(self.ready(Some(RuntimeValue::Bool(true))))
    }

    fn prepare_replace_one_runtime(
        &self,
        _type_name: &str,
        _selector: DbOneSelector,
        _value: &RuntimeValue,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbOptionalRuntimeOperation> {
        let wait_state = Arc::clone(&self.state);
        Ok(PreparedDbRuntimeOperation::new(async move {
            wait_state.record_wait_start();
            if wait_state.replace_wait_fails() {
                return Err(DbCapabilityError::decode("prepared replace failed"));
            }
            let finalize_state = Arc::clone(&wait_state);
            Ok(DbRuntimeFinalizer::new(move |_heap| {
                finalize_state.record_finalize();
                Ok(None)
            }))
        }))
    }
}

pub(super) struct DefaultPreparedStore {
    state: Arc<TestStoreState>,
}

impl DefaultPreparedStore {
    pub(super) fn new(state: Arc<TestStoreState>) -> Self {
        Self { state }
    }
}

impl DbCapabilityStoreApi for DefaultPreparedStore {
    impl_raw_read_api!();
    impl_raw_write_api!();
}
