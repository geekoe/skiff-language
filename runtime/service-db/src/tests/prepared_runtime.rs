use super::*;
use skiff_runtime_capability_context::{DbCapabilityStoreApi, DbRuntimeSetOp};

mod driver;
mod lifecycle;
mod matrix;

fn concrete_service_store() -> ServiceDbStore {
    let runtime = Arc::new(
        ServiceDbRuntime::new(
            service_id("prepared-runtime"),
            inert_mongo_url("prepared-runtime"),
            &object_metadata_for_type("PreparedItem"),
        )
        .expect("prepared runtime fixture should build"),
    );
    ServiceDbStore::new(
        runtime,
        Arc::new(TokioMutex::new(DbRequestState::default())),
    )
}

fn concrete_store() -> ServiceDbCapabilityStore {
    ServiceDbCapabilityStore::new(concrete_service_store())
}

fn context() -> DbRecoverableRuntimeContext {
    production_runtime_context(Arc::new(ThreadSafeTestDbBehaviorHooks::default()))
}

fn input_value(heap: &mut RequestHeap) -> RuntimeValue {
    runtime_object(
        heap,
        [
            ("id", RuntimeValue::String("item-1".to_string())),
            ("title", RuntimeValue::String("first".to_string())),
        ],
    )
}

fn input_change() -> DbRuntimeChange {
    DbRuntimeChange {
        wire_change: ServiceDbChange::default(),
        set_ops: vec![DbRuntimeSetOp {
            field: "title".to_string(),
            value: RuntimeValue::String("changed".to_string()),
        }],
    }
}

#[test]
fn concrete_provider_overrides_all_six_prepared_runtime_entries() {
    let store = concrete_store();
    let mut heap = RequestHeap::default();
    let value = input_value(&mut heap);

    drop(
        store
            .prepare_find_one_by_key_runtime(
                "PreparedItem",
                db_key(json!("item-1")),
                None,
                &mut heap,
                context(),
            )
            .expect("find-one-by-key prepare should use the concrete provider"),
    );
    drop(
        store
            .prepare_find_one_by_query_runtime(
                "PreparedItem",
                db_query(Value::Null),
                Vec::new(),
                None,
                &mut heap,
                context(),
            )
            .expect("find-one-by-query prepare should use the concrete provider"),
    );
    drop(
        store
            .prepare_find_many_page_runtime(
                "PreparedItem",
                db_query(Value::Null),
                ServiceDbFindOptions::default(),
                None,
                &mut heap,
                context(),
            )
            .expect("find-many prepare should use the concrete provider"),
    );
    drop(
        store
            .prepare_create_runtime("PreparedItem", &value, &mut heap, context())
            .expect("create prepare should use the concrete provider"),
    );
    drop(
        store
            .prepare_update_one_runtime(
                "PreparedItem",
                DbOneSelector::Key(db_key(json!("item-1"))),
                input_change(),
                &mut heap,
                context(),
            )
            .expect("update prepare should use the concrete provider"),
    );
    drop(
        store
            .prepare_replace_one_runtime(
                "PreparedItem",
                DbOneSelector::Key(db_key(json!("item-1"))),
                &value,
                &mut heap,
                context(),
            )
            .expect("replace prepare should use the concrete provider"),
    );
}
