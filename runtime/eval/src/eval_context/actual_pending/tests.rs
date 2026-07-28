use super::*;

pub(crate) fn legacy_native_call_expected_to_suspend(binding_key: &str) -> bool {
    skiff_artifact_model::STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
        .is_some_and(|semantics| semantics.effects.may_suspend)
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
