use super::*;
use skiff_runtime_model::type_plan::RuntimeTypePlan;

pub(super) struct RuntimeHttpClientCapabilityContext(
    pub(super) concrete::HttpClientCapabilityContext,
);

impl capability_contract::HttpClientCapabilityApi for RuntimeHttpClientCapabilityContext {
    fn with_stream_runtime(
        &self,
        stream_runtime: capability_contract::StreamRuntime,
    ) -> capability_contract::HttpClientCapabilityContext {
        capability_contract::HttpClientCapabilityContext::new(Self(
            self.0.with_stream_runtime(stream_runtime),
        ))
    }

    fn is_test_http_self_ingress(
        &self,
        input: &Value,
    ) -> capability_contract::CapabilityResult<bool> {
        Ok(self.0.is_test_http_self_ingress(input))
    }

    fn dispatch_http_request<'a>(
        &'a self,
        input: &'a Value,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            root_result_into_capability(
                self.0
                    .dispatch_http_request_with_current_scope(input, execution_control)
                    .await,
            )
            .await
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            root_result_into_capability(
                self.0
                    .dispatch_http_stream_with_current_scope(
                        input,
                        expected_body_item_type,
                        execution_control,
                    )
                    .await,
            )
            .await
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
        execution_control: capability_contract::OwnedExecutionControl,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            root_result_into_capability(
                self.0
                    .dispatch_http_sse_with_current_scope(
                        input,
                        expected_item_type,
                        execution_control,
                    )
                    .await,
            )
            .await
        })
    }
}

#[derive(Clone)]
pub(super) struct RuntimeTelemetryCapabilityContext(
    pub(super) concrete::TelemetryCapabilityContext,
);

impl capability_contract::TelemetryCapabilityApi for RuntimeTelemetryCapabilityContext {
    fn emit_native(
        &self,
        target: &str,
        args: &[Value],
    ) -> capability_contract::CapabilityResult<Value> {
        self.0
            .emit_native(target, args)
            .map_err(ordinary_root_error_into_capability)
    }
}

impl capability_contract::RestrictedServiceDiagnosticSink for RuntimeTelemetryCapabilityContext {
    fn submit(
        &self,
        diagnostic: &capability_contract::RestrictedServiceDiagnostic,
    ) -> capability_contract::CapabilityResult<()> {
        if self.0.emit_restricted_service_diagnostic(diagnostic) {
            Ok(())
        } else {
            Err(capability_contract::CapabilityError::provider_unavailable(
                "restricted-service-diagnostic",
                "request telemetry emitter did not accept the diagnostic",
            ))
        }
    }
}
