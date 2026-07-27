use serde_json::json;
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::fake_store::{default_prepared_store, prepared_store, runtime_context};
use crate::db::{
    DbCapabilityResult, DbKey, DbOneSelector, DbQuery, DbRuntimeChange, PreparedDbRuntimeOperation,
    ServiceDbFindOptions,
};

fn assert_unavailable<T>(result: DbCapabilityResult<PreparedDbRuntimeOperation<T>>) {
    let error = match result {
        Ok(_) => panic!("default prepared runtime operation must fail closed"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("prepared DB runtime operation is unavailable"),
        "{error}"
    );
}

#[tokio::test]
async fn prepared_db_typed_results_cover_all_runtime_paths_without_confusion() {
    let (store, state) = prepared_store(None);
    let mut heap = RequestHeap::default();

    let key_value: Option<RuntimeValue> = store
        .prepare_find_one_by_key_runtime(
            "Item",
            DbKey::new(json!("one")),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare key find")
        .into_wait()
        .await
        .expect("key wait")
        .finalize(&mut heap)
        .expect("key finalize");
    assert_eq!(key_value, Some(RuntimeValue::String("key".to_string())));

    let query_value: Option<RuntimeValue> = store
        .prepare_find_one_by_query_runtime(
            "Item",
            DbQuery::new(json!({})),
            Vec::new(),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare query find")
        .into_wait()
        .await
        .expect("query wait")
        .finalize(&mut heap)
        .expect("query finalize");
    assert_eq!(query_value, Some(RuntimeValue::String("query".to_string())));

    let many_value: Vec<RuntimeValue> = store
        .prepare_find_many_page_runtime(
            "Item",
            DbQuery::new(json!({})),
            ServiceDbFindOptions::default(),
            None,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare many")
        .into_wait()
        .await
        .expect("many wait")
        .finalize(&mut heap)
        .expect("many finalize");
    assert_eq!(
        many_value,
        vec![RuntimeValue::Number(1.0), RuntimeValue::Number(2.0)]
    );

    let created_value: RuntimeValue = store
        .prepare_create_runtime("Item", &RuntimeValue::Null, &mut heap, runtime_context())
        .expect("prepare create")
        .into_wait()
        .await
        .expect("create wait")
        .finalize(&mut heap)
        .expect("create finalize");
    assert!(matches!(created_value, RuntimeValue::Heap(_)));

    let updated_value: Option<RuntimeValue> = store
        .prepare_update_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            DbRuntimeChange::default(),
            &mut heap,
            runtime_context(),
        )
        .expect("prepare update")
        .into_wait()
        .await
        .expect("update wait")
        .finalize(&mut heap)
        .expect("update finalize");
    assert_eq!(updated_value, Some(RuntimeValue::Bool(true)));

    let replaced_value: Option<RuntimeValue> = store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            &RuntimeValue::Null,
            &mut heap,
            runtime_context(),
        )
        .expect("prepare replace")
        .into_wait()
        .await
        .expect("replace wait")
        .finalize(&mut heap)
        .expect("replace finalize");
    assert_eq!(replaced_value, None);
    assert_eq!(state.wait_starts(), 6);
    assert_eq!(state.finalize_calls(), 6);
}

#[test]
fn prepared_db_default_implementation_fails_closed_without_legacy_fallback() {
    let (store, state) = default_prepared_store();
    let mut heap = RequestHeap::default();
    let value = RuntimeValue::Null;

    assert_unavailable(store.prepare_find_one_by_key_runtime(
        "Item",
        DbKey::new(json!("one")),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_find_one_by_query_runtime(
        "Item",
        DbQuery::new(json!({})),
        Vec::new(),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_find_many_page_runtime(
        "Item",
        DbQuery::new(json!({})),
        ServiceDbFindOptions::default(),
        None,
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_create_runtime("Item", &value, &mut heap, runtime_context()));
    assert_unavailable(store.prepare_update_one_runtime(
        "Item",
        DbOneSelector::key(json!("one")),
        DbRuntimeChange::default(),
        &mut heap,
        runtime_context(),
    ));
    assert_unavailable(store.prepare_replace_one_runtime(
        "Item",
        DbOneSelector::key(json!("one")),
        &value,
        &mut heap,
        runtime_context(),
    ));

    assert_eq!(
        state.legacy_runtime_calls(),
        0,
        "prepared defaults must never call heap-borrowing runtime methods"
    );
}

#[tokio::test]
async fn prepared_db_addition_preserves_raw_transaction_and_lease_forwarding() {
    let (store, state) = default_prepared_store();
    store.begin_transaction().await.expect("begin transaction");
    let raw = store
        .find_one_by_key("Item", DbKey::new(json!("raw-1")), None)
        .await
        .expect("raw find")
        .expect("raw document");
    assert_eq!(raw.as_value(), &json!({ "id": "raw-1" }));
    let lease = store
        .claim_lease("Item", DbKey::new(json!("raw-1")), "worker")
        .await
        .expect("claim lease")
        .expect("lease");
    assert_eq!(lease.value.as_value(), &json!({ "lease": "value" }));
    let read = store
        .read_lease("Item", DbKey::new(json!("raw-1")), "worker")
        .await
        .expect("read lease");
    assert_eq!(read, Some(json!({ "lease": "value" })));
    store
        .release_lease(&lease.hold)
        .await
        .expect("release lease");
    store
        .commit_transaction()
        .await
        .expect("commit transaction");
    store.abort_transaction().await;
    assert_eq!(state.raw_calls(), 7);
    assert_eq!(state.legacy_runtime_calls(), 0);
}
