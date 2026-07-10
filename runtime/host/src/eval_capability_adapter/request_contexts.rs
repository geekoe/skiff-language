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
    caller_target: &str,
    execution_budget: Arc<skiff_runtime_request::ExecutionBudget>,
    cancellation: skiff_runtime_request::cancellation::CancellationToken,
    request_heap_limits: RequestHeapLimits,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    outbound_requests: Arc<OutboundRequestRegistry>,
    service_dependencies: &[ServiceDependencyConstraint],
    service_timeout: &ServiceTimeoutConfig,
) -> concrete::OutboundServiceContext {
    concrete::OutboundServiceContext::new(concrete::OutboundServiceContextInput {
        caller_request_id: request.request_id.clone(),
        caller_target: caller_target.to_string(),
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
        cancellation,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_target(target: &str) -> RequestEnvelope {
        RequestEnvelope {
            request_id: "request-parent".to_string(),
            mode: "unary".to_string(),
            target: target.to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: Some("agine.ai/api".to_string()),
            build_id: "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            service_protocol_identity: "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            contract_identity: None,
            activation_identity: None,
            binary_http: None,
            http_adapter: None,
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn outbound_context_uses_canonical_operation_target_as_caller_target() {
        let request = request_with_target("entry.agine~ai~~api.websocket.receive");
        let context = outbound_service_context_from_request(
            &request,
            "AgineService.receive",
            Arc::new(skiff_runtime_request::ExecutionBudget::disabled()),
            skiff_runtime_request::cancellation::CancellationToken::new(),
            RequestHeapLimits::default(),
            None,
            Arc::new(OutboundRequestRegistry::default()),
            &[],
            &ServiceTimeoutConfig::default(),
        );

        let start = context.request_start_control_for_test(
            concrete::OutboundServiceRequestStart {
                service_id: "agine.ai/aihub".to_string(),
                version: "0.1.0".to_string(),
                build_id: "skiff-service-build-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
                service_protocol_identity: "skiff-protocol-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string(),
                operation_abi_id: "operation:aihub:builtin-provider".to_string(),
                selector: "operation:aihub:builtin-provider".to_string(),
                target: "providerCatalog.builtinProvider".to_string(),
                mode: "unary".to_string(),
                timeout_ms: None,
                activation_identity: None,
                test_effect_doubles: HashMap::new(),
            },
            "request-child".to_string(),
        );

        assert_eq!(start.caller.kind, "service");
        assert_eq!(start.caller.target, "AgineService.receive");
    }
}
