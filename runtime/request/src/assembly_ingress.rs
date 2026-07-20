use std::sync::{atomic::AtomicBool, Arc};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    dispatch_ingress_via_in_process_boundary, InProcessBoundaryIngressResponse, Interpreter,
};

use crate::{
    request_payload_context_from_request, AssemblyRequestEvalAdapter, BoundaryResponse,
    ExecutionBudget, ExecutionControl, HttpNameValue, HttpResponseMetadata, RequestEnvelope,
    RequestError, RequestEvalExecutionInputParts, RequestResult, RuntimeAssemblyRequestTarget,
    RuntimeOperation,
};

pub struct AssemblyRequestExecutionInput {
    pub target: RuntimeAssemblyRequestTarget,
    pub request: RequestEnvelope,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: AssemblyRequestExecutionHandles,
}

pub struct AssemblyRequestExecutionHandles {
    pub request_heap_limits: skiff_runtime_model::request_heap::RequestHeapLimits,
    pub eval_adapter: Arc<dyn AssemblyRequestEvalAdapter>,
}

/// Executes a canonical ingress through the production in-process boundary dispatcher.
pub async fn execute_runtime_assembly_request(
    input: AssemblyRequestExecutionInput,
) -> RequestResult<BoundaryResponse> {
    let AssemblyRequestExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    let lifecycle = AssemblyRequestLifecycle::new(target, Arc::clone(&cancelled));
    let target = lifecycle.target();
    validate_assembly_ingress_request(&request)?;
    target
        .ensure_execution_ready()
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let adapter = &handles.eval_adapter;
    let interpreter = Interpreter::for_runtime_assembly(adapter.runtime_factory());
    let operation = canonical_runtime_operation(target, &request);
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let request_context = request_payload_context_from_request(&request);
    let context = adapter.execution_context(
        RequestEvalExecutionInputParts {
            operation: &operation,
            request: &request,
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits.clone(),
        },
        request_context.clone(),
        &interpreter,
        target,
    );
    let mut heap = context.request_heap();
    let result = dispatch_ingress_via_in_process_boundary(
        &interpreter,
        context,
        &mut heap,
        target.boundary().clone(),
        &request_context,
    )
    .await;
    match result.map_err(RequestError::from)? {
        InProcessBoundaryIngressResponse::RuntimePayload(payload) => {
            Ok(BoundaryResponse::end(payload, None, None))
        }
        InProcessBoundaryIngressResponse::BinaryHttp(response) => Ok(BoundaryResponse::end(
            response.body,
            Some(HttpResponseMetadata::new(
                response.status,
                response
                    .headers
                    .into_iter()
                    .map(|header| HttpNameValue {
                        name: header.name,
                        value: header.value,
                    })
                    .collect(),
            )),
            None,
        )),
    }
}

struct AssemblyRequestLifecycle {
    target: RuntimeAssemblyRequestTarget,
    cancelled: Arc<AtomicBool>,
}

impl AssemblyRequestLifecycle {
    fn new(target: RuntimeAssemblyRequestTarget, cancelled: Arc<AtomicBool>) -> Self {
        Self { target, cancelled }
    }

    fn target(&self) -> &RuntimeAssemblyRequestTarget {
        &self.target
    }
}

impl Drop for AssemblyRequestLifecycle {
    fn drop(&mut self) {
        let request_activation = self.target.eval().request_activation();
        if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            request_activation.cancel();
        }
        request_activation.end_request();
    }
}

fn validate_assembly_ingress_request(request: &RequestEnvelope) -> RequestResult<()> {
    if request.ingress_selector.is_none() {
        return Err(RequestError::Unsupported(
            "request.start canonical ingress selector is required".to_string(),
        ));
    }
    if request.mode != "unary" {
        return Err(RequestError::Unsupported(format!(
            "canonical assembly ingress only supports unary request.start, got {}",
            request.mode
        )));
    }
    if request.http_adapter.is_some() {
        return Err(RequestError::Unsupported(
            "legacy HTTP callable adapter metadata is not accepted by canonical assembly ingress"
                .to_string(),
        ));
    }
    Ok(())
}

fn canonical_runtime_operation(
    target: &RuntimeAssemblyRequestTarget,
    request: &RequestEnvelope,
) -> RuntimeOperation {
    let descriptor = target.boundary().descriptor();
    RuntimeOperation {
        operation_abi_id: None,
        operation: descriptor.operation_id.as_str().to_string(),
        target: descriptor.operation_id.as_str().to_string(),
        mode: request.mode.clone(),
        parameters: Vec::new(),
        service_protocol_identity: Some(
            target
                .boundary()
                .contract()
                .service_protocol_identity
                .as_str()
                .to_string(),
        ),
        extra: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};

    use super::validate_assembly_ingress_request;
    use crate::RequestEnvelope;

    #[test]
    fn assembly_ingress_ignores_legacy_target_fields_but_requires_canonical_selector() {
        let mut request = request();
        request.build_id = "mutated-build".to_string();
        request.operation_abi_id = Some("mutated-operation-abi".to_string());
        request.selector = Some("mutated-display-selector".to_string());
        request.target = "mutated-display-target".to_string();
        assert!(validate_assembly_ingress_request(&request).is_ok());

        request.ingress_selector = None;
        let error = validate_assembly_ingress_request(&request)
            .expect_err("missing canonical selector must fail closed");
        assert!(error.to_string().contains("canonical ingress selector"));
    }

    #[test]
    fn assembly_ingress_rejects_legacy_callable_adapter_before_dispatch() {
        let mut request = request();
        request.mode = "serverStream".to_string();
        assert!(validate_assembly_ingress_request(&request).is_err());
    }

    fn request() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "assembly-ingress-request".to_string(),
            mode: "unary".to_string(),
            target: "display-only".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            build_id: "legacy-build".to_string(),
            service_protocol_identity: "legacy-protocol".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: Some(IngressSelector {
                protocol: IngressProtocol::Http,
                host: "example.test".to_string(),
                method: Some("POST".to_string()),
                path: "/entry".to_string(),
            }),
            binary_http: None,
            http_adapter: None,
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}
