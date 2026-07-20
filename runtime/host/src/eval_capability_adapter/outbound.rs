use super::*;

pub(super) struct RuntimeOutboundServiceContext(pub(super) concrete::OutboundServiceContext);

/// Exact Phase 05 fence installed in canonical assembly execution contexts.
/// It owns no router sender, outbound registry, route selector, or dependency table.
pub(super) struct RetiredAssemblyOutboundServiceContext {
    cancellation: CancellationToken,
    request_heap_limits: RequestHeapLimits,
}

impl RetiredAssemblyOutboundServiceContext {
    pub(super) fn new(
        cancellation: CancellationToken,
        request_heap_limits: RequestHeapLimits,
    ) -> Self {
        Self {
            cancellation,
            request_heap_limits,
        }
    }

    fn retired_error() -> RuntimeError {
        RuntimeError::ProviderUnavailable {
            target: "canonical-assembly-service-call".to_string(),
            reason: "legacy outbound service relay is retired for assembly execution".to_string(),
        }
    }
}

impl eval_capabilities::OutboundServiceApi for RetiredAssemblyOutboundServiceContext {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        &[]
    }

    fn test_effects_enabled(&self) -> bool {
        false
    }

    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        HashMap::new()
    }

    fn request_heap(&self) -> RequestHeap {
        RequestHeap::new(self.request_heap_limits.clone())
    }

    fn effective_timeout_ms(&self, _operation_timeout_ms: Option<u64>) -> Option<u64> {
        None
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        Self::retired_error()
    }

    fn start_request(
        &self,
        _start: eval_capabilities::OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> Result<eval_capabilities::OutboundStartedRequest> {
        Err(Self::retired_error())
    }

    fn receive_response<'a>(
        &'a self,
        _lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        _target: &'a str,
        _receiver: &'a mut skiff_runtime_capability_context::OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> eval_capabilities::EvalCapabilityFuture<
        'a,
        skiff_runtime_capability_context::OutboundResponse,
    > {
        Box::pin(async { Err(Self::retired_error()) })
    }

    fn cancel_signal(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

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
