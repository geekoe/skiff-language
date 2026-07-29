use super::*;

pub fn effect_dispatch_context_from_request(
    request: &RequestEnvelope,
    response_max_bytes: usize,
    cancellation: CancellationToken,
    telemetry: Option<RequestTelemetryContext>,
    http_options: concrete::HttpRuntimeOptions,
) -> concrete::EffectDispatchContext {
    concrete::EffectDispatchContext::new(
        concrete::HttpEffectContext::new(
            request_deadline_ms(request),
            response_max_bytes,
            cancellation,
        ),
        concrete::TelemetryCapabilityContext::new(telemetry),
        http_options,
    )
}
