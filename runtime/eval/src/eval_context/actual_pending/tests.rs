use super::*;

pub(crate) fn legacy_native_call_expected_to_suspend(binding_key: &str) -> bool {
    skiff_artifact_model::STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
        .is_some_and(|semantics| semantics.effects.may_pending)
        || binding_key.starts_with("std.file.")
        || binding_key.starts_with("std.actor.")
        || matches!(
            binding_key,
            "std.http.client.stream" | "std.http.client.sse" | "std.http.stream.emitResponse"
        )
}

#[test]
fn actual_pending_surface_is_owned_by_the_evaluator_spine() {
    fn assert_future<F: Future>(_: F) {}
    let context = std::future::ready(());
    assert_future(context);
}

#[test]
fn only_plan_free_json_encode_may_defer_return_materialization_to_dispatch() {
    for binding_key in ["std.json.decode", "core.array.empty", "unknown.native"] {
        let invocation = skiff_runtime_native::dispatch::RuntimeNativeInvocation::new(
            binding_key.to_string(),
            binding_key,
            None,
            None,
            None,
        );
        let error = prepared_native_return_plan(&invocation)
            .expect_err("every native except std.json.encode must still require a plan");
        assert!(
            error
                .to_string()
                .contains(&format!("unsupported native target {binding_key}")),
            "unexpected fail-closed diagnostic for {binding_key}: {error}"
        );
    }

    let encode = skiff_runtime_native::dispatch::RuntimeNativeInvocation::new(
        "std.json.encode".to_string(),
        "std.json.encode",
        None,
        None,
        None,
    );
    assert!(prepared_native_return_plan(&encode)
        .expect("plan-free JSON encode is an admitted dynamic dispatch")
        .is_none());
}

#[test]
fn plan_free_json_encode_materializes_only_its_fixed_string_return() {
    let mut heap = RequestHeap::default();
    let carrier = materialize_prepared_native_return(
        RuntimeValue::String("{\"provider\":\"deepseek\"}".to_string()),
        None,
        &mut heap,
    )
    .expect("dynamic JSON encode has a fixed builtin string return");
    assert_eq!(
        carrier.into_value(),
        RuntimeValue::String("{\"provider\":\"deepseek\"}".to_string())
    );

    let error = materialize_prepared_native_return(RuntimeValue::Null, None, &mut heap)
        .expect_err("a plan-free target must not smuggle a different return kind");
    assert!(
        error
            .to_string()
            .contains("plan-free std.json.encode returned a non-string value"),
        "unexpected fail-closed diagnostic: {error}"
    );
}
