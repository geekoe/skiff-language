mod prepared;
mod raw_read_api;
mod raw_write_api;
mod state;

use std::sync::Arc;

use tokio::sync::oneshot;

use crate::db::DbCapabilityStore;

pub(super) use state::{runtime_context, wait_until_started, TestStoreState};

pub(super) fn prepared_store(
    gate: Option<oneshot::Receiver<()>>,
) -> (DbCapabilityStore, Arc<TestStoreState>) {
    let state = Arc::new(TestStoreState::new(gate));
    let store = DbCapabilityStore::new(prepared::PreparedFakeStore::new(Arc::clone(&state)));
    (store, state)
}

pub(super) fn default_prepared_store() -> (DbCapabilityStore, Arc<TestStoreState>) {
    let state = Arc::new(TestStoreState::new(None));
    let store = DbCapabilityStore::new(prepared::DefaultPreparedStore::new(Arc::clone(&state)));
    (store, state)
}
