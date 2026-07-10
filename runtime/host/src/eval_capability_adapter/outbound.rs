use super::*;

pub(super) struct RuntimeOutboundServiceContext(pub(super) concrete::OutboundServiceContext);

impl eval_capabilities::OutboundServiceApi for RuntimeOutboundServiceContext {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        self.0.service_dependencies()
    }

    fn test_effects_enabled(&self) -> bool {
        self.0.test_effects_enabled()
    }

    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        self.0.test_effect_doubles()
    }

    fn request_heap(&self) -> RequestHeap {
        self.0.request_heap()
    }

    fn effective_timeout_ms(&self, operation_timeout_ms: Option<u64>) -> Option<u64> {
        self.0.effective_timeout_ms(operation_timeout_ms)
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        root_error_into_eval(self.0.outbound_deadline_error())
    }

    fn start_request(
        &self,
        start: eval_capabilities::OutboundServiceRequestStart,
        payload: Vec<u8>,
    ) -> Result<eval_capabilities::OutboundStartedRequest> {
        self.0.start_request(start, payload).into_eval_result()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn request_start_control_for_test(
        &self,
        start: eval_capabilities::OutboundServiceRequestStart,
        request_id: String,
    ) -> skiff_runtime_capability_context::RequestStartControl {
        self.0.request_start_control_for_test(start, request_id)
    }

    fn receive_response<'a>(
        &'a self,
        lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        target: &'a str,
        receiver: &'a mut skiff_runtime_capability_context::OutboundResponseReceiver,
        timeout_ms: Option<u64>,
    ) -> eval_capabilities::EvalCapabilityFuture<
        'a,
        skiff_runtime_capability_context::OutboundResponse,
    > {
        Box::pin(async move {
            self.0
                .receive_response(lease, target, receiver, timeout_ms)
                .await
                .into_eval_result()
        })
    }

    fn cancel_signal(&self) -> CancellationToken {
        self.0.cancel_signal()
    }
}
