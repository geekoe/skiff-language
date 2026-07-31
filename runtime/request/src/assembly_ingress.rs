use std::sync::{atomic::AtomicBool, Arc};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_eval::{
    dispatch_ingress_via_in_process_boundary, InProcessBoundaryIngressResponse, Interpreter,
    TestEffectDouble,
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
    let interpreter = if request.test_effects_enabled || !request.test_effect_doubles.is_empty() {
        Interpreter::for_runtime_assembly_with_test_effect_double_sequences(
            request
                .test_effect_doubles
                .iter()
                .map(|(target, sequence)| {
                    (
                        target.clone(),
                        sequence
                            .iter()
                            .map(|double| TestEffectDouble {
                                expect_request: double.expect_request.clone(),
                                response: double.response.clone(),
                            })
                            .collect(),
                    )
                })
                .collect(),
            adapter.runtime_factory(),
        )
    } else {
        Interpreter::for_runtime_assembly(adapter.runtime_factory())
    };
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
    let body_result = dispatch_ingress_via_in_process_boundary(
        &interpreter,
        context,
        &mut heap,
        target.boundary().clone(),
        &request_context,
    )
    .await
    .map_err(RequestError::from)
    .map(|result| match result {
        InProcessBoundaryIngressResponse::RuntimePayload(payload) => {
            BoundaryResponse::payload(payload)
        }
        InProcessBoundaryIngressResponse::BinaryHttp(response) => BoundaryResponse::http(
            response.body,
            HttpResponseMetadata::new(
                response.status,
                response
                    .headers
                    .into_iter()
                    .map(|header| HttpNameValue {
                        name: header.name,
                        value: header.value,
                    })
                    .collect(),
            ),
        ),
    });
    let finalization_result = interpreter.finalize_test_case().map_err(RequestError::from);
    match (body_result, finalization_result) {
        (Err(body_error), _) => Err(body_error),
        (Ok(_), Err(finalization_error)) => Err(finalization_error),
        (Ok(response), Ok(())) => Ok(response),
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
    let selector = request
        .ingress_selector
        .as_ref()
        .expect("canonical selector checked above");
    match selector.protocol {
        skiff_artifact_model::IngressProtocol::Http => {
            if request.extra.contains_key("websocketEntryId") {
                return Err(RequestError::Unsupported(
                    "canonical HTTP ingress does not accept websocketEntryId".to_string(),
                ));
            }
        }
        skiff_artifact_model::IngressProtocol::WebSocket => {
            return Err(RequestError::Unsupported(
                "webSocket connect uses the dedicated runtimeAssembly request path".to_string(),
            ))
        }
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
mod tests;
