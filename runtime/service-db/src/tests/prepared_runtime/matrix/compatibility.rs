use std::sync::Arc;

use skiff_runtime_capability_context::DbCapabilityStoreApi;

use crate::prepared_runtime::PreparedRuntimeTestKind;

use super::super::{driver::TestDriver, *};

fn store_with_driver(driver: TestDriver) -> ServiceDbCapabilityStore {
    ServiceDbCapabilityStore::new(
        concrete_service_store().with_prepared_runtime_test_driver(Arc::new(driver)),
    )
}

fn assert_title(value: &RuntimeValue, heap: &RequestHeap, expected: &str) {
    let RuntimeValue::Heap(handle) = value else {
        panic!("prepared DB result should be a heap object");
    };
    let HeapNode::Object(object) = heap.get(*handle).expect("result handle should resolve") else {
        panic!("prepared DB result should be an object");
    };
    assert_eq!(
        object.fields().get("title"),
        Some(&RuntimeValue::String(expected.to_string()))
    );
}

#[tokio::test]
async fn prepared_and_legacy_entries_share_the_same_six_concrete_waits() {
    let driver = TestDriver::ready();
    let store = store_with_driver(driver.clone());
    let mut heap = RequestHeap::default();
    let value = input_value(&mut heap);

    let by_key = store
        .prepare_find_one_by_key_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_key(json!("item-1")),
            None,
            &mut heap,
            context(),
        )
        .expect("find-one-by-key prepare")
        .into_wait()
        .await
        .expect("find-one-by-key wait")
        .finalize(&mut heap)
        .expect("find-one-by-key finalize")
        .expect("find-one-by-key value");
    assert_title(&by_key, &heap, "from-provider");

    let by_query = store
        .prepare_find_one_by_query_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            Vec::new(),
            None,
            &mut heap,
            context(),
        )
        .expect("find-one-by-query prepare")
        .into_wait()
        .await
        .expect("find-one-by-query wait")
        .finalize(&mut heap)
        .expect("find-one-by-query finalize")
        .expect("find-one-by-query value");
    assert_title(&by_query, &heap, "from-provider");

    let many = store
        .prepare_find_many_page_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            &mut heap,
            context(),
        )
        .expect("find-many prepare")
        .into_wait()
        .await
        .expect("find-many wait")
        .finalize(&mut heap)
        .expect("find-many finalize");
    assert_eq!(many.len(), 1);
    assert_title(&many[0], &heap, "from-provider");

    let created = store
        .prepare_create_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            &value,
            &mut heap,
            context(),
        )
        .expect("create prepare")
        .into_wait()
        .await
        .expect("create wait")
        .finalize(&mut heap)
        .expect("create finalize");
    assert_title(&created, &heap, "first");

    let updated = store
        .prepare_update_one_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            DbOneSelector::Key(db_key(json!("item-1"))),
            input_change(),
            &mut heap,
            context(),
        )
        .expect("update prepare")
        .into_wait()
        .await
        .expect("update wait")
        .finalize(&mut heap)
        .expect("update finalize")
        .expect("update value");
    assert_title(&updated, &heap, "from-provider");

    let replaced = store
        .prepare_replace_one_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            DbOneSelector::Key(db_key(json!("item-1"))),
            &value,
            &mut heap,
            context(),
        )
        .expect("replace prepare")
        .into_wait()
        .await
        .expect("replace wait")
        .finalize(&mut heap)
        .expect("replace finalize")
        .expect("replace value");
    assert_title(&replaced, &heap, "from-provider");

    let legacy_by_key = store
        .find_one_by_key_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_key(json!("item-1")),
            None,
            &mut heap,
            context(),
        )
        .await
        .expect("legacy find-one-by-key")
        .expect("legacy find-one-by-key value");
    assert_title(&legacy_by_key, &heap, "from-provider");

    let legacy_by_query = store
        .find_one_by_query_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            Vec::new(),
            None,
            &mut heap,
            context(),
        )
        .await
        .expect("legacy find-one-by-query")
        .expect("legacy find-one-by-query value");
    assert_title(&legacy_by_query, &heap, "from-provider");

    let legacy_many = store
        .find_many_page_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            &mut heap,
            context(),
        )
        .await
        .expect("legacy find-many");
    assert_eq!(legacy_many.len(), 1);
    assert_title(&legacy_many[0], &heap, "from-provider");

    let legacy_created = store
        .create_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            &value,
            &mut heap,
            context(),
        )
        .await
        .expect("legacy create");
    assert_title(&legacy_created, &heap, "first");

    let legacy_updated = store
        .update_one_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            DbOneSelector::Key(db_key(json!("item-1"))),
            input_change(),
            &mut heap,
            context(),
        )
        .await
        .expect("legacy update")
        .expect("legacy update value");
    assert_title(&legacy_updated, &heap, "from-provider");

    let legacy_replaced = store
        .replace_one_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            DbOneSelector::Key(db_key(json!("item-1"))),
            &value,
            &mut heap,
            context(),
        )
        .await
        .expect("legacy replace")
        .expect("legacy replace value");
    assert_title(&legacy_replaced, &heap, "from-provider");

    let one_round = [
        PreparedRuntimeTestKind::FindOneByKey,
        PreparedRuntimeTestKind::FindOneByQuery,
        PreparedRuntimeTestKind::FindMany,
        PreparedRuntimeTestKind::Create,
        PreparedRuntimeTestKind::Update,
        PreparedRuntimeTestKind::Replace,
    ];
    assert_eq!(driver.kinds(), [one_round, one_round].concat());
    assert_eq!(driver.starts(), 12);
    assert_eq!(driver.completions(), 12);
    assert_eq!(driver.pending_drops(), 0);
}

#[tokio::test]
async fn create_finalizer_uses_the_owned_prepare_document_not_the_input_handle() {
    let driver = TestDriver::ready();
    let store = store_with_driver(driver);
    let mut heap = RequestHeap::default();
    let value = input_value(&mut heap);
    let RuntimeValue::Heap(input_handle) = value else {
        panic!("create input should be a heap object");
    };
    let operation = store
        .prepare_create_runtime(
            test_db_target(0, "", "PreparedItem").lookup_key(),
            &RuntimeValue::Heap(input_handle),
            &mut heap,
            context(),
        )
        .expect("create prepare should encode an owned document");

    heap.set_object_field(
        input_handle,
        "title".to_string(),
        RuntimeValue::String("changed-after-prepare".to_string()),
    )
    .expect("input mutation should succeed");
    let finalizer = operation
        .into_wait()
        .await
        .expect("create provider wait should succeed");
    let created = finalizer
        .finalize(&mut heap)
        .expect("create finalizer should materialize the owned document");

    assert_title(&created, &heap, "first");
    assert_title(
        &RuntimeValue::Heap(input_handle),
        &heap,
        "changed-after-prepare",
    );
}
