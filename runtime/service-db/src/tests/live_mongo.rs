use super::{super::*, recoverable_support::*, support::*};

#[tokio::test]
#[ignore = "requires a local MongoDB replica set and real network resources"]
async fn service_db_runtime_create_and_find_runtime_roundtrips_local_interface() {
    let service_id = format!(
        "skiff.run/p5dbprodtest-{}-{}",
        std::process::id(),
        service_db_now_ms()
    );
    let runtime = ServiceDbRuntime::new(
        test_environment(),
        service_id,
        "mongodb://127.0.0.1:27017/?directConnection=true".to_string(),
        &provider_metadata_from_ir(recoverable_provider_metadata_value()),
    )
    .expect("service DB runtime should build");
    let database_name = runtime.database_name_for_test();
    let client = runtime
        .client()
        .await
        .expect("local Mongo service DB should be available for production-path DB test");
    client
        .database(&database_name)
        .drop()
        .await
        .expect("test database should drop before run");

    let mut heap = RequestHeap::default();
    let provider = local_provider_runtime_value(&mut heap, "openai");
    let value = runtime_object(
        &mut heap,
        [
            ("id", RuntimeValue::String("binding-1".to_string())),
            ("provider", provider),
        ],
    );
    let hooks = Arc::new(TestDbBehaviorHooks::default());
    let context = production_runtime_context(hooks.clone());

    runtime
        .create_runtime("ProviderBinding", &value, &mut heap, context.clone(), None)
        .await
        .expect("production service DB runtime create should encode local interface");

    let mut read_heap = RequestHeap::default();
    let read = runtime
        .find_one_by_key_runtime(
            "ProviderBinding",
            db_key(json!("binding-1")),
            None,
            &mut read_heap,
            context.clone(),
            None,
        )
        .await
        .expect("production service DB runtime read should decode local interface")
        .expect("created provider binding should exist");

    assert_decoded_provider_runtime_value(&read, &read_heap, "binding-1", "openai");

    let plain_find_many_error = runtime
        .find_many_page(
            "ProviderBinding",
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            None,
        )
        .await
        .expect_err("plain find many should not decode behavior recoverable envelope fields");
    assert!(
        plain_find_many_error
            .to_string()
            .contains("recoverable-envelope DB field decode failed"),
        "{plain_find_many_error}"
    );

    let mut page_heap = RequestHeap::default();
    let page = runtime
        .find_many_page_runtime(
            "ProviderBinding",
            db_query(Value::Null),
            ServiceDbFindOptions::default(),
            None,
            &mut page_heap,
            context.clone(),
            None,
        )
        .await
        .expect("production service DB runtime find many should decode local interface");
    assert_eq!(page.len(), 1);
    assert_decoded_provider_runtime_value(&page[0], &page_heap, "binding-1", "openai");

    let replaced = runtime
        .replace_one_runtime(
            "ProviderBinding",
            DbOneSelector::Key(db_key(json!("binding-1"))),
            &read,
            &mut read_heap,
            context.clone(),
            &[],
            None,
        )
        .await
        .expect(
            "production service DB runtime replace should re-encode the decoded local interface",
        )
        .expect("created provider binding should be replaced");

    assert_decoded_provider_runtime_value(&replaced, &read_heap, "binding-1", "openai");

    let mut reread_heap = RequestHeap::default();
    let reread = runtime
        .find_one_by_key_runtime(
            "ProviderBinding",
            db_key(json!("binding-1")),
            None,
            &mut reread_heap,
            context,
            None,
        )
        .await
        .expect("production service DB runtime read should decode replaced local interface")
        .expect("replaced provider binding should exist");

    assert_decoded_provider_runtime_value(&reread, &reread_heap, "binding-1", "anthropic");
    assert!(hooks.restore_calls() >= 1);
    client
        .database(&database_name)
        .drop()
        .await
        .expect("test database should drop after run");
}
