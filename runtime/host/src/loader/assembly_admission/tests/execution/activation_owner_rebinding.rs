use std::sync::Arc;

use skiff_runtime_eval::program_execution::ActivationExecutionOperation;

use super::scenario::TypedExecutionFixture;

#[tokio::test]
async fn service_call_owner_facts_use_provider_activation_protocol_config_and_image() {
    let fixture = TypedExecutionFixture::admit().await;
    let provider = fixture.resolve_provider();
    let provider_target = fixture
        .eval_target
        .with_request_activation(provider.provider_request().clone())
        .expect("provider target stays in the pinned generation");
    let (activation, protocol, operation) =
        crate::eval_capability_adapter::provider_execution_facts_for_test(
            fixture._active.contexts(),
            fixture._active.candidate().execution_image(),
            &provider_target,
            ActivationExecutionOperation::ServiceCall {
                operation_id: fixture.provider_operation.clone(),
            },
        )
        .expect("provider facts resolve from the pinned context set");

    assert!(Arc::ptr_eq(&activation, provider.provider_activation()));
    assert_eq!(
        protocol,
        provider.contract().service_protocol_identity.as_str()
    );
    assert_eq!(operation, fixture.provider_operation.as_str());
    assert!(Arc::ptr_eq(
        provider_target.execution_image(),
        fixture._active.candidate().execution_image(),
    ));

    let caller_config = fixture
        ._active
        .contexts()
        .config_views(
            &fixture
                .eval_target
                .activation_context()
                .identity()
                .deployment,
        )
        .expect("caller config views");
    let provider_config = fixture
        ._active
        .contexts()
        .config_views(&fixture.provider_deployment)
        .expect("provider config views");
    assert!(
        !Arc::ptr_eq(&caller_config, &provider_config),
        "even an empty fixture must retain activation-owned config-view objects"
    );
}

#[tokio::test]
async fn callback_and_nested_switches_resolve_each_exact_activation_owner() {
    let fixture = TypedExecutionFixture::admit().await;
    let provider = fixture.resolve_provider();
    let provider_target = fixture
        .eval_target
        .with_request_activation(provider.provider_request().clone())
        .expect("provider target");
    let (provider_activation, _, _) =
        crate::eval_capability_adapter::provider_execution_facts_for_test(
            fixture._active.contexts(),
            fixture._active.candidate().execution_image(),
            &provider_target,
            ActivationExecutionOperation::ServiceCall {
                operation_id: fixture.provider_operation.clone(),
            },
        )
        .expect("A to B provider facts");
    let (callback_activation, _, callback_method) =
        crate::eval_capability_adapter::provider_execution_facts_for_test(
            fixture._active.contexts(),
            fixture._active.candidate().execution_image(),
            &fixture.eval_target,
            ActivationExecutionOperation::CallbackMethod {
                method_abi_id: "callback.invoke".to_string(),
            },
        )
        .expect("B to A callback owner facts");

    assert!(Arc::ptr_eq(
        &provider_activation,
        provider.provider_activation()
    ));
    assert!(Arc::ptr_eq(
        &callback_activation,
        fixture.eval_target.activation_context()
    ));
    assert_eq!(callback_method, "callback.invoke");
    assert_ne!(
        provider_activation.activation_id(),
        callback_activation.activation_id(),
        "nested switches must not retain the immediately preceding owner"
    );
}

#[tokio::test]
async fn old_generation_rebinder_rejects_new_generation_target_and_keeps_old_snapshot() {
    let old = TypedExecutionFixture::admit().await;
    let new = TypedExecutionFixture::admit().await;
    let old_provider = old.resolve_provider();
    let new_provider = new.resolve_provider();
    let old_target = old
        .eval_target
        .with_request_activation(old_provider.provider_request().clone())
        .expect("old provider target");
    let new_target = new
        .eval_target
        .with_request_activation(new_provider.provider_request().clone())
        .expect("new provider target");
    crate::eval_capability_adapter::provider_execution_facts_for_test(
        old._active.contexts(),
        old._active.candidate().execution_image(),
        &old_target,
        ActivationExecutionOperation::ServiceCall {
            operation_id: old.provider_operation.clone(),
        },
    )
    .expect("old generation remains usable while its request is alive");
    let error = crate::eval_capability_adapter::provider_execution_facts_for_test(
        old._active.contexts(),
        old._active.candidate().execution_image(),
        &new_target,
        ActivationExecutionOperation::ServiceCall {
            operation_id: old.provider_operation.clone(),
        },
    )
    .expect_err("old rebinder must not consult or accept the latest generation");
    assert!(error.to_string().contains("pinned request execution image"));

    crate::eval_capability_adapter::provider_execution_facts_for_test(
        new._active.contexts(),
        new._active.candidate().execution_image(),
        &new_target,
        ActivationExecutionOperation::ServiceCall {
            operation_id: new.provider_operation.clone(),
        },
    )
    .expect("cold new request uses the new generation context set");
}

#[test]
fn provider_bundle_source_has_no_caller_db_or_owner_fallback() {
    let source =
        include_str!("../../../../eval_capability_adapter/activation_execution_rebinder.rs");
    for required in [
        ".config_views(deployment)",
        ".db_source(facts.activation.activation_id())",
        "file_source(self.input.file_source.clone())",
        "websocket_from_runtime_request(",
        "activation_identity_control(&facts.activation)",
        "telemetry.service_id = Some(deployment.service_id.clone())",
        "telemetry.build_id = Some(",
        "telemetry.activation_identity =",
        "telemetry.target = Some(facts.target.clone())",
    ] {
        assert!(
            source.contains(required),
            "provider bundle must retain exact owner projection: {required}"
        );
    }
    assert!(
        !source.contains("unwrap_or(self.input"),
        "missing provider capabilities must not inherit caller-owned state"
    );
    assert!(
        !source.contains("latest"),
        "generation-pinned rebinding must never consult a latest assembly"
    );
    assert!(
        !source.contains("service_resources"),
        "activation switching must not rebuild or replace static resource projection"
    );
}
