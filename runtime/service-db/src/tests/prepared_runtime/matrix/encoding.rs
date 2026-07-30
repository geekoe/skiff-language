use std::sync::Arc;

use skiff_runtime_capability_context::DbCapabilityStoreApi;

use super::super::*;

#[test]
fn recoverable_and_encrypted_writes_finish_owned_encoding_during_prepare() {
    let recoverable_runtime = Arc::new(
        ServiceDbRuntime::new(
            "skiff.run/preparedrecoverable".to_string(),
            inert_mongo_url("prepared-recoverable"),
            &provider_metadata_from_ir(recoverable_provider_metadata_value()),
        )
        .expect("recoverable prepared runtime fixture should build"),
    );
    let recoverable_store = ServiceDbCapabilityStore::new(ServiceDbStore::new(
        recoverable_runtime,
        Arc::new(TokioMutex::new(DbRequestState::default())),
    ));
    let hooks = Arc::new(ThreadSafeTestDbBehaviorHooks::default());
    let recoverable_context = production_runtime_context(hooks);
    let mut recoverable_heap = RequestHeap::default();
    let provider = local_provider_runtime_value(&mut recoverable_heap, "openai");
    let recoverable_value = runtime_object(
        &mut recoverable_heap,
        [
            ("id", RuntimeValue::String("binding-1".to_string())),
            ("provider", provider),
        ],
    );

    drop(
        recoverable_store
            .prepare_create_runtime(
                test_db_target(0, "", "ProviderBinding").lookup_key(),
                &recoverable_value,
                &mut recoverable_heap,
                recoverable_context.clone(),
            )
            .expect("recoverable create should encode during prepare"),
    );
    drop(
        recoverable_store
            .prepare_update_one_runtime(
                test_db_target(0, "", "ProviderBinding").lookup_key(),
                DbOneSelector::Key(db_key(json!("binding-1"))),
                DbRuntimeChange {
                    wire_change: ServiceDbChange::default(),
                    set_ops: vec![DbRuntimeSetOp {
                        field: "provider".to_string(),
                        value: local_provider_runtime_value(&mut recoverable_heap, "anthropic"),
                    }],
                },
                &mut recoverable_heap,
                recoverable_context.clone(),
            )
            .expect("recoverable update should encode during prepare"),
    );
    drop(
        recoverable_store
            .prepare_replace_one_runtime(
                test_db_target(0, "", "ProviderBinding").lookup_key(),
                DbOneSelector::Key(db_key(json!("binding-1"))),
                &recoverable_value,
                &mut recoverable_heap,
                recoverable_context,
            )
            .expect("recoverable replace should encode during prepare"),
    );

    let encrypted_runtime = Arc::new(
        ServiceDbRuntime::new_with_config(
            "skiff.run/prepencrypted".to_string(),
            ServiceDbConfig {
                mongo_url: inert_mongo_url("prepared-encrypted"),
                encryption_cipher: Some(test_encryption_keyring().cipher()),
            },
            &provider_metadata_from_ir(encrypted_metadata("string", "string", json!([]))),
        )
        .expect("encrypted prepared runtime fixture should build"),
    );
    let encrypted_store = ServiceDbCapabilityStore::new(ServiceDbStore::new(
        encrypted_runtime,
        Arc::new(TokioMutex::new(DbRequestState::default())),
    ));
    let mut encrypted_heap = RequestHeap::default();
    let encrypted_value = runtime_object(
        &mut encrypted_heap,
        [
            ("id", RuntimeValue::String("credential-1".to_string())),
            ("apiKey", RuntimeValue::String("secret-value".to_string())),
            ("label", RuntimeValue::String("primary".to_string())),
        ],
    );

    drop(
        encrypted_store
            .prepare_create_runtime(
                test_db_target(0, "internal.credential", "Credential").lookup_key(),
                &encrypted_value,
                &mut encrypted_heap,
                context(),
            )
            .expect("encrypted create should encode during prepare"),
    );
    drop(
        encrypted_store
            .prepare_update_one_runtime(
                test_db_target(0, "internal.credential", "Credential").lookup_key(),
                DbOneSelector::Key(db_key(json!("credential-1"))),
                DbRuntimeChange {
                    wire_change: ServiceDbChange::default(),
                    set_ops: vec![DbRuntimeSetOp {
                        field: "apiKey".to_string(),
                        value: RuntimeValue::String("rotated-secret".to_string()),
                    }],
                },
                &mut encrypted_heap,
                context(),
            )
            .expect("encrypted update should encode during prepare"),
    );
    drop(
        encrypted_store
            .prepare_replace_one_runtime(
                test_db_target(0, "internal.credential", "Credential").lookup_key(),
                DbOneSelector::Key(db_key(json!("credential-1"))),
                &encrypted_value,
                &mut encrypted_heap,
                context(),
            )
            .expect("encrypted replace should encode during prepare"),
    );
}
