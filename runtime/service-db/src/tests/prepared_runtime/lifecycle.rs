use std::sync::Arc;

use mongodb::bson::doc;
use skiff_runtime_capability_context::DbCapabilityStoreApi;
use skiff_runtime_model::request_heap::RequestHeapLimits;

use super::{driver::TestDriver, *};

fn store_with_driver(driver: TestDriver) -> ServiceDbCapabilityStore {
    ServiceDbCapabilityStore::new(
        concrete_service_store().with_prepared_runtime_test_driver(Arc::new(driver)),
    )
}

#[test]
fn unpolled_drop_never_starts_the_provider_wait() {
    let driver = TestDriver::pending();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let operation = store
        .prepare_find_one_by_key_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_key(json!("item-1")),
            None,
            &mut heap,
            context(),
        )
        .expect("prepare should succeed");

    drop(operation);

    assert_eq!(driver.starts(), 0);
    assert_eq!(driver.completions(), 0);
    assert_eq!(driver.pending_drops(), 0);
    assert!(heap.is_empty());
}

#[test]
fn prepare_failure_never_builds_or_starts_a_provider_wait() {
    let driver = TestDriver::pending();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();

    let missing_type = match store.prepare_find_one_by_key_runtime(
        test_db_target(1, "", "MissingItem").lookup_key(),
        db_key(json!("item-1")),
        None,
        &mut heap,
        context(),
    ) {
        Ok(_) => panic!("missing metadata should fail during prepare"),
        Err(error) => error,
    };
    let scalar = RuntimeValue::String("not-an-object".to_string());
    let invalid_create = match store.prepare_create_runtime(
        test_db_target(0, "", "PreparedItem").lookup_key(),
        &scalar,
        &mut heap,
        context(),
    ) {
        Ok(_) => panic!("invalid create value should fail during prepare"),
        Err(error) => error,
    };

    assert!(missing_type
        .to_string()
        .contains("does not declare the exact DB target"));
    assert!(invalid_create
        .to_string()
        .contains("db write value must be an object"));
    assert_eq!(driver.starts(), 0);
    assert_eq!(driver.completions(), 0);
    assert_eq!(driver.pending_drops(), 0);
    assert!(heap.is_empty());
}

#[tokio::test]
async fn zero_limit_skips_projection_compilation_and_provider_wait() {
    let driver = TestDriver::pending();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();
    let operation = store
        .prepare_find_many_page_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            ServiceDbFindOptions {
                order: vec![DbOrderEntry {
                    field: FieldPath {
                        text: "title".to_string(),
                        segments: vec!["title".to_string()],
                    },
                    direction: DbOrderDirection::Asc,
                }],
                limit: Some(0),
                ..ServiceDbFindOptions::default()
            },
            Some(vec![FieldPath {
                text: "missing.nested".to_string(),
                segments: vec!["missing".to_string(), "nested".to_string()],
            }]),
            &mut heap,
            context(),
        )
        .expect("zero limit should skip invalid projection compilation");

    let finalizer = operation
        .into_wait()
        .await
        .expect("zero limit should complete without a provider");
    let values = finalizer
        .finalize(&mut heap)
        .expect("zero-limit finalizer should materialize an empty page");

    assert!(values.is_empty());
    assert_eq!(driver.starts(), 0);
    assert_eq!(driver.completions(), 0);
    assert_eq!(driver.pending_drops(), 0);
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);
}

#[tokio::test]
async fn pending_wait_releases_caller_heap_until_one_shot_finalize() {
    let driver = TestDriver::pending();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let operation = store
        .prepare_find_one_by_key_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_key(json!("item-1")),
            None,
            &mut heap,
            context(),
        )
        .expect("prepare should succeed");

    let wait_task = tokio::spawn(operation.into_wait());
    driver.wait_until_started().await;
    let existing = runtime_object(
        &mut heap,
        [("local", RuntimeValue::String("unchanged".to_string()))],
    );
    let before_finalize = heap.checkpoint();
    let before_finalize_stats = heap.stats();

    driver.release();
    let finalizer = wait_task
        .await
        .expect("wait task should join")
        .expect("provider wait should succeed");

    assert_eq!(heap.checkpoint(), before_finalize);
    assert_eq!(heap.stats(), before_finalize_stats);
    assert!(matches!(existing, RuntimeValue::Heap(_)));
    let value = finalizer
        .finalize(&mut heap)
        .expect("finalizer should materialize provider document")
        .expect("provider document should exist");

    assert!(matches!(value, RuntimeValue::Heap(_)));
    assert_eq!(driver.starts(), 1);
    assert_eq!(driver.completions(), 1);
    assert_eq!(driver.pending_drops(), 0);
    assert_eq!(heap.len(), 2);
}

#[tokio::test]
async fn pending_drop_does_not_restart_or_complete_the_provider_wait() {
    let driver = TestDriver::pending();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let checkpoint = heap.checkpoint();
    let operation = store
        .prepare_find_one_by_query_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            Vec::new(),
            None,
            &mut heap,
            context(),
        )
        .expect("prepare should succeed");

    let wait_task = tokio::spawn(operation.into_wait());
    driver.wait_until_started().await;
    wait_task.abort();
    let _ = wait_task.await;
    tokio::task::yield_now().await;

    assert_eq!(driver.starts(), 1);
    assert_eq!(driver.completions(), 0);
    assert_eq!(driver.pending_drops(), 1);
    assert_eq!(heap.checkpoint(), checkpoint);
}

#[tokio::test]
async fn provider_error_returns_no_finalizer_and_preserves_heap() {
    let driver = TestDriver::pending();
    driver.fail();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let existing = runtime_object(
        &mut heap,
        [("local", RuntimeValue::String("unchanged".to_string()))],
    );
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();
    let operation = store
        .prepare_find_many_page_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            &mut heap,
            context(),
        )
        .expect("prepare should succeed");

    let error = match operation.into_wait().await {
        Ok(_) => panic!("provider error must not return a finalizer"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("prepared runtime test provider failure"));
    assert!(matches!(existing, RuntimeValue::Heap(_)));
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);
    assert_eq!(driver.starts(), 1);
    assert_eq!(driver.completions(), 0);
    assert_eq!(driver.pending_drops(), 0);
}

#[tokio::test]
async fn multi_node_finalizer_failure_rolls_back_every_allocation() {
    let driver = TestDriver::ready_with_document(doc! {
        "_id": "item-1",
        "title": {
            "nested": ["one", "two"],
        },
    });
    let store = store_with_driver(driver);
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    });
    let _existing = runtime_object(
        &mut heap,
        [("local", RuntimeValue::String("unchanged".to_string()))],
    );
    let operation = store
        .prepare_find_one_by_key_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_key(json!("item-1")),
            None,
            &mut heap,
            context(),
        )
        .expect("prepare should succeed");
    let finalizer = operation
        .into_wait()
        .await
        .expect("provider wait should succeed");
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();

    let error = finalizer
        .finalize(&mut heap)
        .expect_err("nested result should exceed the node budget");

    assert!(error.to_string().contains("max heap nodes"), "{error}");
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);
}
