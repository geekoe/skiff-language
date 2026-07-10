use super::*;

pub(crate) struct RuntimeRequestEvalAdapterInput {
    pub(crate) service: Arc<crate::host::ServiceRuntimeContext>,
    pub(crate) file_source: concrete::FileCapabilitySource,
    pub(crate) http_options: concrete::HttpRuntimeOptions,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) telemetry_context: Option<RequestTelemetryContext>,
    pub(crate) router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
}

pub(crate) fn request_eval_adapter(
    input: RuntimeRequestEvalAdapterInput,
) -> Arc<dyn RequestEvalAdapter> {
    Arc::new(RuntimeRequestEvalAdapter {
        service: input.service,
        file_source: input.file_source,
        http_options: input.http_options,
        outbound_requests: input.outbound_requests,
        telemetry_context: input.telemetry_context,
        router_sender: input.router_sender,
    })
}

pub(crate) fn attach_request_error_diagnostic_frame(
    error: root_error::RuntimeError,
    program: &skiff_runtime_eval::EvalRuntimeProgram,
    operation: &str,
    target: &str,
    build_id: &str,
    addr: &ExecutableAddr,
) -> root_error::RuntimeError {
    root_error::RuntimeError::from(program.attach_request_diagnostic_frame(
        root_error_into_eval(error),
        operation,
        target,
        build_id,
        addr,
    ))
}

#[derive(Clone)]
struct RuntimeRequestEvalAdapter {
    service: Arc<crate::host::ServiceRuntimeContext>,
    file_source: concrete::FileCapabilitySource,
    http_options: concrete::HttpRuntimeOptions,
    outbound_requests: Arc<OutboundRequestRegistry>,
    telemetry_context: Option<RequestTelemetryContext>,
    router_sender: Option<mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
}

impl RequestEvalAdapter for RuntimeRequestEvalAdapter {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_input<'a>(
        &'a self,
        parts: RequestEvalExecutionInputParts<'a>,
        request_context: skiff_runtime_request::RequestPayloadContext<'a>,
    ) -> skiff_runtime_eval::EvalRequestExecutionInput<'a> {
        let RequestEvalExecutionInputParts {
            operation,
            request,
            execution,
            cancellation,
            cancelled,
            execution_budget,
            request_heap_limits,
        } = parts;
        let service = self.service.as_ref();
        let db = db_context(
            service
                .service_db
                .context_for_request(service.runtime_id.clone(), request.request_id.clone()),
        );
        let effects = effects(effect_dispatch_context_from_request(
            request,
            service.http_response_max_bytes,
            execution.cancel_flag(),
            self.telemetry_context.clone(),
            self.http_options.clone(),
        ));
        let websocket = websocket_from_request(
            service.service_id.as_str(),
            request
                .extra
                .get("websocketEntryId")
                .and_then(Value::as_str),
            self.router_sender.as_ref(),
        );
        let actor = actor_from_request(
            service.runtime_id.as_str(),
            service.service_id.as_str(),
            service.service_version(),
            request,
            operation,
            self.router_sender.as_ref(),
            &self.outbound_requests,
            cancelled,
            execution.cancel_flag(),
        );
        let outbound = outbound(outbound_service_context_from_request(
            request,
            operation.target.as_str(),
            execution_budget.clone(),
            cancellation,
            request_heap_limits.clone(),
            self.router_sender.clone(),
            self.outbound_requests.clone(),
            &service.runtime_activation.service_dependencies,
            &service.runtime_activation.timeout,
        ));
        skiff_runtime_eval::EvalRequestExecutionInput {
            request: request_context,
            operation: operation.operation.as_str(),
            capabilities: eval_capabilities::EvalRequestExecutionCapabilities::from_parts(
                execution_control(execution),
                config_context(concrete::ConfigCapabilityContext::new(
                    &service.config,
                    &service.package_configs,
                )),
                db,
                file_source(self.file_source.clone()),
                websocket,
                effects,
                Arc::clone(&service.runtime_activation),
                actor.clone(),
                actor,
                outbound,
            ),
            request_heap_limits,
            http_response_max_bytes: service.http_response_max_bytes,
        }
    }
}
