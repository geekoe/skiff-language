use super::*;

pub(super) fn concrete_stream_runtime(
    stream_runtime: &capability_contract::StreamRuntime,
) -> &concrete::StreamRuntime {
    &stream_runtime
        .downcast_ref::<RuntimeStreamRuntime>()
        .expect("eval stream runtime came from runtime adapter")
        .0
}

pub(super) fn concrete_actor_context_from_owned(
    parts: &RuntimeOwnedActorParts,
) -> concrete::ActorCapabilityContext<'_> {
    concrete::ActorCapabilityContext::from_parts(
        &parts.runtime_id,
        &parts.service_id,
        &parts.service_version,
        &parts.request_id,
        &parts.request_target,
        &parts.request_build_id,
        &parts.request_service_protocol_identity,
        parts.operation_service_protocol_identity.as_deref(),
        parts.activation_identity.as_deref(),
        parts.trace_id.as_deref(),
        parts.router_sender.as_ref(),
        parts.outbound_requests.as_ref(),
        parts.cancellation.clone(),
    )
}

pub(super) fn concrete_db_context(
    db_context: &eval_capabilities::DbCapabilityContext,
) -> &concrete::DbCapabilityContext {
    db_context
}

pub(super) fn concrete_test_effect_double_context(
    context: &eval_capabilities::TestEffectDoubleContext,
) -> &concrete::TestEffectDoubleContext {
    &context
        .downcast_ref::<RuntimeTestEffectDoubleContext>()
        .expect("eval test effect double context came from runtime adapter")
        .0
}

pub(super) fn concrete_stream_cancel_signals(
    signals: &[capability_contract::StreamCancelSignal],
) -> StreamRuntimeResult<Vec<concrete::StreamCancelSignal>> {
    signals
        .iter()
        .map(|signal| {
            signal
                .downcast_ref::<RuntimeStreamCancelSignal>()
                .map(|signal| signal.0.clone())
                .ok_or_else(|| {
                    StreamRuntimeError::decode("stream cancel signal came from a different adapter")
                })
        })
        .collect()
}

pub(super) fn concrete_test_double(
    double: eval_capabilities::TestEffectDouble,
) -> concrete::TestEffectDouble {
    concrete::TestEffectDouble {
        expect_request: double.expect_request,
        response: double.response,
    }
}

pub(super) fn eval_test_double(
    double: concrete::TestEffectDouble,
) -> eval_capabilities::TestEffectDouble {
    eval_capabilities::TestEffectDouble {
        expect_request: double.expect_request,
        response: double.response,
    }
}
