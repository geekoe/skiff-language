use super::*;

use super::{
    actor::TestActorCapability,
    file::{NoTelemetry, TestFileBundle},
    http::{NoHttpResponseStream, TestHttpClient},
    websocket::TestWebsocketCapability,
};

type PreparedTestCapabilityContexts = NativeCapabilityContexts<
    TestActorCapability,
    TestFileBundle,
    CountingTimeContext,
    TestHttpClient,
    NoHttpResponseStream,
    TestWebsocketCapability,
    NoTelemetry,
    (),
>;

#[test]
fn core_prepare_keeps_sync_routes_ready_and_required_context_fail_closed() {
    let mut heap = RequestHeap::default();
    let bytes_target = "core.bytes.fromUtf8";
    let invocation = RuntimeNativeInvocation::new(
        bytes_target.to_string(),
        bytes_target,
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static(bytes_target),
            vec![scalar_plan("string", RuntimeTypeNode::String)],
            scalar_plan("bytes", RuntimeTypeNode::Bytes),
            NativeRequiredContext::None,
        )),
        None,
        None,
    );
    let prepared = crate::dispatch::core::prepare_resolved_native_call(
        PreparedTestCapabilityContexts::None,
        invocation,
        vec![RuntimeValue::String("ready".to_string())],
        &mut heap,
    )
    .expect("bytes helper should prepare synchronously");
    assert!(
        matches!(prepared, PreparedNativeCall::Ready(RuntimeValue::Heap(_))),
        "bytes/json/registry-style synchronous routes must not be external waits"
    );

    let error = match crate::dispatch::core::prepare_resolved_native_call(
        PreparedTestCapabilityContexts::None,
        sleep_invocation(),
        vec![RuntimeValue::Number(1.0)],
        &mut heap,
    ) {
        Ok(_) => panic!("time call with no capability must fail closed"),
        Err(error) => error,
    };
    assert!(
        matches!(error, RuntimeError::InvalidArtifact(_)),
        "unexpected mismatch error: {error}"
    );
    assert!(
        error.to_string().contains("expected Time"),
        "unexpected mismatch diagnostic: {error}"
    );
}
