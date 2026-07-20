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
    let service_error = interpreter
        .execute_runtime_assembly_addr(
            context.clone(),
            &mut service_heap,
            &fixture.consumer_executable_addr(0),
            Vec::new(),
        )
        .await
        .expect_err("ordinary service call must enter its exact admitted provider target");
    assert!(
        service_error
            .to_string()
            .contains("executable provide missing block entry"),
        "service execution stopped before the admitted provider target: {service_error}"
    );

    let mut package_heap = context.request_heap();
    let package_error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut package_heap,
            &fixture.consumer_executable_addr(1),
            Vec::new(),
        )
        .await
        .expect_err("package direct call must enter its exact admitted package target");
    assert!(
        package_error
            .to_string()
            .contains("executable provide missing block entry"),
        "package execution stopped before the admitted package target: {package_error}"
    );
    assert_eq!(
        fixture.eval_target.activation_context().activation_id(),
        &receiver_activation,
        "provider execution must not replace the receiver activation target"
    );
}
