use super::*;

pub fn runtime_factory() -> eval_capabilities::EvalRuntimeFactory {
    eval_capabilities::EvalRuntimeFactory::new(RuntimeEvalFactory)
}

pub fn execution_control<'a>(
    execution: skiff_runtime_request::ExecutionControl<'a>,
) -> eval_capabilities::ExecutionControl<'a> {
    capability_contract::ExecutionControl::new(RuntimeExecutionControl(execution.owned()))
}

pub fn config_context<'a>(
    context: concrete::ConfigCapabilityContext<'a>,
) -> eval_capabilities::ConfigCapabilityContext<'a> {
    eval_capabilities::ConfigCapabilityContext::new(RuntimeConfigCapabilityContext(context))
}

pub fn db_context(
    context: concrete::DbCapabilityContext,
) -> eval_capabilities::DbCapabilityContext {
    context
}

pub fn file_source(
    source: concrete::FileCapabilitySource,
) -> eval_capabilities::FileCapabilitySource {
    capability_contract::FileCapabilitySource::new(RuntimeFileCapabilitySource(source))
}

pub fn effects(
    context: concrete::EffectDispatchContext,
) -> eval_capabilities::EffectDispatchContext {
    eval_capabilities::EffectDispatchContext::new(RuntimeEffectDispatchContext(context))
}

pub fn outbound(
    context: concrete::OutboundServiceContext,
) -> eval_capabilities::OutboundServiceContext {
    eval_capabilities::OutboundServiceContext::new(RuntimeOutboundServiceContext(context))
}

pub fn websocket<'a>(
    context: concrete::WebsocketCapabilityContext<'a>,
    owned: RuntimeOwnedWebsocketParts,
) -> eval_capabilities::WebsocketCapabilityContext<'a> {
    eval_capabilities::WebsocketCapabilityContext::new(RuntimeWebsocketCapabilityContext {
        context,
        owned,
    })
}

pub fn websocket_from_request<'a>(
    service_id: &'a str,
    websocket_entry_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
) -> eval_capabilities::WebsocketCapabilityContext<'a> {
    websocket(
        concrete::WebsocketCapabilityContext::with_entry_id(
            service_id,
            websocket_entry_id,
            router_sender,
        ),
        RuntimeOwnedWebsocketParts {
            service_id: service_id.to_string(),
            websocket_entry_id: websocket_entry_id.map(str::to_string),
            router_sender: router_sender.cloned(),
        },
    )
}

pub fn actor_from_request<'a>(
    runtime_id: &'a str,
    service_id: &'a str,
    service_version: &'a str,
    request: &'a RequestEnvelope,
    operation: &'a RuntimeOperation,
    router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    outbound_requests: &'a Arc<OutboundRequestRegistry>,
    cancelled: &'a AtomicBool,
    cancel_flag: Arc<AtomicBool>,
) -> eval_capabilities::ActorCapabilityContext<'a> {
    let invocation = invocation_context_from_request(
        runtime_id,
        service_id,
        service_version,
        request,
        operation,
    );
    let context = concrete::ActorClientContext::new(
        invocation,
        router_sender,
        outbound_requests.as_ref(),
        cancelled,
    );
    actor(
        context,
        RuntimeOwnedActorParts {
            runtime_id: context.runtime_id().to_string(),
            service_id: context.service_id().to_string(),
            service_version: context.service_version().to_string(),
            request_id: context.request_id().to_string(),
            request_target: context.request_target().to_string(),
            request_build_id: context.request_build_id().to_string(),
            request_service_protocol_identity: context
                .request_service_protocol_identity()
                .to_string(),
            operation_service_protocol_identity: context
                .operation_service_protocol_identity()
                .map(str::to_string),
            activation_identity: context.activation_identity().map(str::to_string),
            trace_id: context.trace_id().map(str::to_string),
            router_sender: router_sender.cloned(),
            outbound_requests: outbound_requests.clone(),
            cancel_flag,
        },
    )
}

#[derive(Clone)]
struct RuntimeEvalFactory;

impl eval_capabilities::EvalRuntimeFactoryApi for RuntimeEvalFactory {
    fn stream_runtime(&self) -> eval_capabilities::StreamRuntime {
        capability_contract::StreamRuntime::new(RuntimeStreamRuntime(
            concrete::StreamRuntime::default(),
        ))
    }

    fn reusable_test_effect_doubles(
        &self,
        doubles: HashMap<String, eval_capabilities::TestEffectDouble>,
        stream_runtime: &eval_capabilities::StreamRuntime,
        test_effects_enabled: bool,
    ) -> eval_capabilities::TestEffectDoubleContext {
        eval_capabilities::TestEffectDoubleContext::new(RuntimeTestEffectDoubleContext(
            concrete::TestEffectDoubleContext::reusable(
                doubles
                    .into_iter()
                    .map(|(target, double)| (target, concrete_test_double(double)))
                    .collect(),
                concrete_stream_runtime(stream_runtime).clone(),
                test_effects_enabled,
            ),
        ))
    }

    fn one_shot_test_effect_double_sequences(
        &self,
        doubles: HashMap<String, Vec<eval_capabilities::TestEffectDouble>>,
        stream_runtime: &eval_capabilities::StreamRuntime,
        test_effects_enabled: bool,
    ) -> eval_capabilities::TestEffectDoubleContext {
        eval_capabilities::TestEffectDoubleContext::new(RuntimeTestEffectDoubleContext(
            concrete::TestEffectDoubleContext::one_shot_sequences(
                doubles
                    .into_iter()
                    .map(|(target, doubles)| {
                        (
                            target,
                            doubles.into_iter().map(concrete_test_double).collect(),
                        )
                    })
                    .collect(),
                concrete_stream_runtime(stream_runtime).clone(),
                test_effects_enabled,
            ),
        ))
    }
}
