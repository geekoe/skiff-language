use super::*;

pub fn effect_dispatch_context_from_request(
    request: &RequestEnvelope,
    response_max_bytes: usize,
    request_cancelled: Arc<AtomicBool>,
    telemetry: Option<RequestTelemetryContext>,
    http_options: concrete::HttpRuntimeOptions,
) -> concrete::EffectDispatchContext {
    concrete::EffectDispatchContext::new(
        concrete::HttpEffectContext::new(
            request_deadline_ms(request),
            response_max_bytes,
            request_cancelled,
        ),
        concrete::TelemetryCapabilityContext::new(telemetry),
        http_options,
    )
}

pub fn outbound_service_context_from_request(
    request: &RequestEnvelope,
    execution_budget: Arc<skiff_runtime_request::ExecutionBudget>,
    cancel_flag: Arc<AtomicBool>,
    request_heap_limits: RequestHeapLimits,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    outbound_requests: Arc<OutboundRequestRegistry>,
    service_dependencies: &[ServiceDependencyConstraint],
    service_timeout: &ServiceTimeoutConfig,
) -> concrete::OutboundServiceContext {
    concrete::OutboundServiceContext::new(concrete::OutboundServiceContextInput {
        caller_request_id: request.request_id.clone(),
        caller_target: request.target.clone(),
        client_session: request
            .extra
            .get("clientSession")
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        caller_deadline: concrete::OutboundCallerDeadline::from_extra(&request.extra),
        service_timeout: service_timeout.clone(),
        trace: concrete::OutboundTraceMetadata::from_extra(
            &request.request_id,
            request.extra.get("trace"),
        ),
        service_dependencies: service_dependencies.to_vec(),
        test_effects_enabled: request.test_effects_enabled,
        test_effect_doubles: request_effect_doubles_to_control(&request.test_effect_doubles),
        execution_budget,
        cancel_flag,
        request_heap_limits,
        router_sender,
        outbound_requests,
    })
}

fn request_effect_doubles_to_control(
    doubles: &HashMap<String, Vec<RequestEffectDouble>>,
) -> HashMap<String, Vec<skiff_runtime_capability_context::RequestEffectDoubleControl>> {
    doubles
        .iter()
        .map(|(target, sequence)| {
            (
                target.clone(),
                sequence
                    .iter()
                    .map(
                        |double| skiff_runtime_capability_context::RequestEffectDoubleControl {
                            expect_request: double.expect_request.clone(),
                            response: double.response.clone(),
                        },
                    )
                    .collect(),
            )
        })
        .collect()
}
