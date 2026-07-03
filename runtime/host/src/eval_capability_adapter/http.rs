use super::*;
use skiff_runtime_model::type_plan::RuntimeTypePlan;

pub(super) struct RuntimeHttpClientCapabilityContext(
    pub(super) concrete::HttpClientCapabilityContext,
);

impl capability_contract::HttpClientCapabilityApi for RuntimeHttpClientCapabilityContext {
    fn dispatch_http_request<'a>(
        &'a self,
        input: &'a Value,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .dispatch_http_request(input)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .dispatch_http_stream(input, expected_body_item_type)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> capability_contract::HttpCapabilityFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .dispatch_http_sse(input, expected_item_type)
                .await
                .map_err(capability_contract::CapabilityError::opaque)
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
            .map_err(capability_contract::CapabilityError::opaque)
    }
}
