use super::{
    artifacts::TypedExecutionContract, runtime::TypedExecutionRuntime,
    scenario::TypedExecutionFixture,
};
use skiff_runtime_eval::error::RuntimeError;
use skiff_runtime_model::{
    runtime_value::{HeapNode, RuntimeValue},
    service_error::PlatformBuiltinErrorIdentity,
};

#[tokio::test]
async fn typed_execution_callback_native_uses_production_service_materialization() {
    let fixture = TypedExecutionFixture::admit_contract(
        TypedExecutionContract::callback().with_callback_owner_may_suspend(true),
    )
    .await;
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
    let RuntimeError::UserException(exception) = mapping_error else {
        panic!("callback protocol mismatch must remain a typed user exception: {mapping_error}")
    };
    assert_eq!(
        exception.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity())
    );
    let RuntimeValue::Heap(payload_handle) = exception
        .request()
        .local_value()
        .expect("platform protocol error must expose its caller-local payload")
        .value()
    else {
        panic!("platform protocol error payload must remain a record")
    };
    let HeapNode::Object(payload) = heap
        .get(*payload_handle)
        .expect("platform protocol error payload must remain in the caller heap")
    else {
        panic!("platform protocol error payload must remain an object")
    };
    let Some(RuntimeValue::String(message)) = payload.fields().get("message") else {
        panic!("platform protocol error payload must retain its message field")
    };
    assert!(
        message.contains("contract operation different has no same-name local method"),
        "projection should reject the explicit stable-name mismatch: {message}"
    );
    assert!(
        !message.contains("provide missing block entry")
            && !message.contains("CallbackProbe.invoke missing block entry"),
        "mapping mismatch must fail before provider or owner executable entry: {message}"
    );
}
