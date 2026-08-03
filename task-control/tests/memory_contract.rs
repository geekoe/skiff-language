//! Contract matrix (reference test items 5-14) against the in-memory fake,
//! plus fake-specific deterministic time / failure-injection checks.

mod support;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use skiff_task_control::{MemoryTaskStore, TaskStore, TaskStoreErrorClass};

    use super::support::{contract, fixtures, FakeClock, TestTime};

    const START_MILLIS: i64 = 1_700_000_000_000;

    #[tokio::test]
    async fn contract_matrix_runs_on_memory_store() {
        let clock = Arc::new(FakeClock::new(START_MILLIS));
        let store = MemoryTaskStore::with_clock(clock.clone());
        contract::run_contract(&store, &TestTime::Controlled(clock)).await;
    }

    #[tokio::test]
    async fn transient_store_failures_are_retryable() {
        let clock = Arc::new(FakeClock::new(START_MILLIS));
        let store = MemoryTaskStore::with_clock(clock.clone());
        let time = TestTime::Controlled(clock);

        let record = fixtures::record(201, time.now_millis() + 60_000);
        store.fail_next_transient(1).await;
        let error = store
            .create(record.clone())
            .await
            .expect_err("injected transient failure");
        assert!(error.is_retryable(), "transient errors must be retryable");
        assert_eq!(
            error.class(),
            TaskStoreErrorClass::Transient,
            "driver/store unavailability is the transient class"
        );
        store.create(record).await.expect("retry succeeds");
    }

    #[tokio::test]
    async fn closed_store_rejects_operations() {
        let clock = Arc::new(FakeClock::new(START_MILLIS));
        let store = MemoryTaskStore::with_clock(clock);
        store.close().await.expect("close");
        let error = store
            .create(fixtures::record(202, START_MILLIS + 60_000))
            .await
            .expect_err("closed store");
        assert_eq!(error, skiff_task_control::TaskStoreError::Closed);
    }
}
