use super::{
    artifacts::TypedExecutionContract, runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};

#[tokio::test]
async fn typed_execution_callback_native_uses_production_service_materialization() {
    let fixture = TypedExecutionFixture::admit_contract(TypedExecutionContract::callback()).await;
    let receiver_activation_id = fixture
        .eval_target
        .activation_context()
        .activation_id()
        .clone();
    let provider_activation_id = fixture
        .resolve_provider()
        .provider_activation()
        .activation_id()
        .clone();
    assert_ne!(receiver_activation_id, provider_activation_id);

    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let owner_error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err(
            "consumer InterfaceBox must cross the production service hook, enter provider, and dispatch back to its owner method",
        );
    assert!(
        owner_error
            .to_string()
            .contains("CallbackProbe.invoke missing block entry"),
        "callback should execute the exact admitted owner method after the provider context switch: {owner_error}"
    );
    assert_eq!(
        fixture.eval_target.activation_context().activation_id(),
        &receiver_activation_id,
        "callback unwind must preserve the receiver activation target"
    );
}

#[tokio::test]
async fn typed_execution_callback_native_rejects_wrong_mapping_before_provider_or_owner() {
    let fixture = TypedExecutionFixture::admit_contract(
        TypedExecutionContract::callback_with_operation_key("different"),
    )
    .await;
    let runtime = TypedExecutionRuntime::new(
        &fixture
            .eval_target
            .activation_context()
            .identity()
            .deployment
            .service_id,
    );
    let interpreter = runtime.interpreter();
    let context = runtime.context(&interpreter, &fixture.eval_target);
    let mut heap = context.request_heap();

    let mapping_error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("same-count but wrong-name callback mappings must fail closed");
    let message = mapping_error.to_string();
    assert!(
        message.contains("contract operation different has no same-name local method"),
        "projection should reject the explicit stable-name mismatch: {mapping_error}"
    );
    assert!(
        !message.contains("provide missing block entry")
            && !message.contains("CallbackProbe.invoke missing block entry"),
        "mapping mismatch must fail before provider or owner executable entry: {mapping_error}"
    );
}
