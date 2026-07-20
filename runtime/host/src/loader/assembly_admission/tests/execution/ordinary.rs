use super::{runtime::TypedExecutionRuntime, scenario::TypedExecutionFixture};

#[tokio::test]
async fn typed_execution_ordinary() {
    let fixture = TypedExecutionFixture::admit().await;
    let receiver_activation = fixture
        .eval_target
        .activation_context()
        .activation_id()
        .clone();
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

    let mut service_heap = context.request_heap();
    let service_result = interpreter
        .execute_runtime_assembly_addr(
            context.clone(),
            &mut service_heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect("ordinary service call must return from its exact admitted provider target");
    assert_eq!(
        service_result,
        skiff_runtime_model::runtime_value::RuntimeValue::Bool(true),
        "service execution must propagate the detached provider result"
    );

    let mut package_heap = context.request_heap();
    let package_result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut package_heap,
            &fixture.consumer_executable_addr(1),
            Vec::new(),
        )
        .await
        .expect("package direct call must return from its exact admitted package target");
    assert_eq!(
        package_result,
        skiff_runtime_model::runtime_value::RuntimeValue::Bool(true),
        "package execution must propagate the same-heap provider result"
    );
    assert_eq!(
        fixture.eval_target.activation_context().activation_id(),
        &receiver_activation,
        "provider execution must not replace the receiver activation target"
    );
}
