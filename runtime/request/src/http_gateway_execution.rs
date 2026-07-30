use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crate::{
    response_stream_writer::ResponseStreamWriter, BoundaryResponse, ExecutionBudget,
    ExecutionControl, HttpNameValue, HttpResponseMetadata, RequestError, RequestResult,
    ResponseEventSink, RuntimeAssemblyHttpGatewayTarget, RuntimeHttpGatewayRequest,
};
use skiff_artifact_model::{
    AssemblyIdentity, GatewayAdapterKind, GatewayDispatchMode, GatewayEntryIdentity,
    GatewayProtocolSurface,
};
use skiff_runtime_capability_context::{
    BinaryHttpRequestContext, CancellationToken, HttpNameValueContext, RequestPayloadContext,
};
use skiff_runtime_eval::{
    capabilities::EvalRuntimeFactory, error::RuntimeError,
    program_execution::ProgramExecutionContext, Interpreter, RuntimeAssemblyEvalTarget,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;

pub struct RuntimeHttpGatewayExecutionInput {
    pub target: RuntimeAssemblyHttpGatewayTarget,
    pub request: RuntimeHttpGatewayRequest,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: RuntimeHttpGatewayExecutionHandles,
}

pub struct RuntimeHttpGatewayExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub eval_adapter: Arc<dyn RuntimeHttpGatewayEvalAdapter>,
    pub response_events: Arc<dyn ResponseEventSink>,
}

/// Host capability-context builder for the already-validated gateway request.
///
/// The adapter receives no callable/schema/adapter facts to interpret; those remain inside the
/// exact target and eval execution seam.
pub trait RuntimeHttpGatewayEvalAdapter: Send + Sync {
    fn runtime_factory(&self) -> EvalRuntimeFactory;

    fn begin_test_effect_execution(
        &self,
    ) -> RequestResult<Option<RuntimeHttpGatewayTestEffectExecution>>;

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeHttpGatewayEvalExecutionInputParts<'a>,
        request_context: RequestPayloadContext<'a>,
        interpreter: &'a Interpreter,
        eval_target: &'a RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a>;
}

trait RuntimeHttpGatewayTestEffectLease: Send + Sync {}

impl<T> RuntimeHttpGatewayTestEffectLease for T where T: Send + Sync {}

/// Internal runtime ownership for a parent test request or one exact nested
/// HTTP ingress borrowing that parent's inline-effect registry.
#[doc(hidden)]
pub struct RuntimeHttpGatewayTestEffectExecution {
    context: skiff_runtime_eval::TestEffectCaseContext,
    owner: RuntimeHttpGatewayTestEffectOwner,
}

enum RuntimeHttpGatewayTestEffectOwner {
    Nested(Box<dyn RuntimeHttpGatewayTestEffectLease>),
    Root(Pin<Box<dyn Future<Output = skiff_runtime_eval::error::Result<()>> + Send + 'static>>),
}

impl RuntimeHttpGatewayTestEffectExecution {
    #[doc(hidden)]
    pub fn nested(
        context: skiff_runtime_eval::TestEffectCaseContext,
        lease: impl Send + Sync + 'static,
    ) -> Self {
        Self {
            context,
            owner: RuntimeHttpGatewayTestEffectOwner::Nested(Box::new(lease)),
        }
    }

    #[doc(hidden)]
    pub fn root(
        context: skiff_runtime_eval::TestEffectCaseContext,
        finalization: impl Future<Output = skiff_runtime_eval::error::Result<()>> + Send + 'static,
    ) -> Self {
        Self {
            context,
            owner: RuntimeHttpGatewayTestEffectOwner::Root(Box::pin(finalization)),
        }
    }
}

pub struct RuntimeHttpGatewayEvalExecutionInputParts<'a> {
    pub execution: ExecutionControl<'a>,
    pub cancellation: CancellationToken,
    pub cancelled: &'a AtomicBool,
    pub execution_budget: Arc<ExecutionBudget>,
    pub request_heap_limits: RequestHeapLimits,
}

pub async fn execute_runtime_http_gateway_request(
    input: RuntimeHttpGatewayExecutionInput,
) -> RequestResult<BoundaryResponse> {
    let RuntimeHttpGatewayExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;
    let lifecycle = RuntimeHttpGatewayRequestLifecycle::new(target, Arc::clone(&cancelled));
    let target = lifecycle.target();
    validate_request(target, &request)?;
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;
    let mut test_effect_execution = handles.eval_adapter.begin_test_effect_execution()?;
    let interpreter = match test_effect_execution.as_ref() {
        Some(test_effect_execution) => {
            Interpreter::for_runtime_assembly_with_test_effect_case_context(
                test_effect_execution.context.clone(),
                handles.eval_adapter.runtime_factory(),
            )
        }
        None if request.test_effects_enabled => {
            return Err(RequestError::Unsupported(
                "test HTTP ingress did not establish an exact test-effect execution".to_string(),
            ))
        }
        None => Interpreter::for_runtime_assembly(handles.eval_adapter.runtime_factory()),
    };
    let request_context = request_context(target, &request);
    let context = handles.eval_adapter.execution_context(
        RuntimeHttpGatewayEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: cancelled.as_ref(),
            execution_budget: Arc::clone(&execution_budget),
            request_heap_limits: handles.request_heap_limits,
        },
        request_context.clone(),
        &interpreter,
        target.eval(),
    );
    let GatewayProtocolSurface::Http(http) = &target.protocol_surface().protocol else {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "HTTP gateway execution requires an HTTP protocol surface",
        ));
    };
    let body_result = match http.dispatch_mode {
        GatewayDispatchMode::Unary => interpreter
            .execute_runtime_http_gateway_unary(context, request_context, target)
            .await
            .map(|response| {
                BoundaryResponse::http(
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
                )
            })
            .map_err(RequestError::from),
        GatewayDispatchMode::ServerStream => {
            let mut writer = ResponseStreamWriter::new(
                request.request_id.clone(),
                Arc::clone(&handles.response_events),
            );
            interpreter
                .execute_runtime_http_gateway_server_stream(
                    context,
                    request_context,
                    target,
                    |event| {
                        writer.send_binary_http_event(event).map_err(|error| {
                            RuntimeError::Protocol {
                                target: target.gateway_entry_key().as_str().to_string(),
                                message: error.to_string(),
                            }
                        })
                    },
                )
                .await
                .map_err(RequestError::from)
                .and_then(|()| writer.require_exact_http_terminal())
                .map(|()| BoundaryResponse::StreamSent)
        }
    };
    let finalization_result = match test_effect_execution
        .take()
        .map(|execution| execution.owner)
    {
        Some(RuntimeHttpGatewayTestEffectOwner::Root(finalization)) => {
            finalization.await.map_err(RequestError::from)
        }
        Some(RuntimeHttpGatewayTestEffectOwner::Nested(lease)) => {
            drop(lease);
            Ok(())
        }
        None => Ok(()),
    };
    match (body_result, finalization_result) {
        (Err(body_error), _) => Err(body_error),
        (Ok(_), Err(finalization_error)) => Err(finalization_error),
        (Ok(response), Ok(())) => Ok(response),
    }
}

fn validate_request(
    target: &RuntimeAssemblyHttpGatewayTarget,
    request: &RuntimeHttpGatewayRequest,
) -> RequestResult<()> {
    let GatewayProtocolSurface::Http(http) = &target.protocol_surface().protocol else {
        return Err(RequestError::protocol(
            target.gateway_entry_key().as_str(),
            "HTTP gateway request validation requires an HTTP protocol surface",
        ));
    };
    validate_request_facts(
        HttpGatewayRequestValidationFacts {
            gateway_entry_key: target.gateway_entry_key().as_str(),
            assembly_identity: target.eval().execution_image().assembly_identity(),
            assembly_generation: target
                .eval()
                .activation_context()
                .identity()
                .assembly_generation,
            deployment: target.owner(),
            gateway_entry_identity: target.gateway_entry_identity(),
            dispatch_mode: http.dispatch_mode,
            surface_adapter_kind: http.adapter_kind,
            plan_adapter_kind: target.adapter_plan().kind,
        },
        request,
    )
}

struct HttpGatewayRequestValidationFacts<'a> {
    gateway_entry_key: &'a str,
    assembly_identity: &'a AssemblyIdentity,
    assembly_generation: u64,
    deployment: &'a skiff_artifact_model::ServiceDeploymentRef,
    gateway_entry_identity: &'a GatewayEntryIdentity,
    dispatch_mode: GatewayDispatchMode,
    surface_adapter_kind: GatewayAdapterKind,
    plan_adapter_kind: GatewayAdapterKind,
}

fn validate_request_facts(
    target: HttpGatewayRequestValidationFacts<'_>,
    request: &RuntimeHttpGatewayRequest,
) -> RequestResult<()> {
    if request.pin.assembly_identity != *target.assembly_identity
        || request.pin.assembly_generation != target.assembly_generation
        || &request.pin.deployment != target.deployment
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request does not match the pinned assembly activation",
        ));
    }
    if request.pin.gateway_entry_identity != *target.gateway_entry_identity {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request identity does not match the exact linked entry",
        ));
    }
    if request.ingress_method != request.http_request.method
        || request.ingress_path != request.http_request.path
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway routing metadata and binary HTTP context disagree",
        ));
    }
    if request.dispatch_mode != target.dispatch_mode
        || (target.surface_adapter_kind == GatewayAdapterKind::TypedJson
            && target.dispatch_mode != GatewayDispatchMode::Unary)
        || target.plan_adapter_kind != target.surface_adapter_kind
    {
        return Err(RequestError::protocol(
            target.gateway_entry_key,
            "HTTP gateway request mode does not match the exact linked adapter plan",
        ));
    }
    Ok(())
}

fn request_context<'a>(
    target: &'a RuntimeAssemblyHttpGatewayTarget,
    request: &'a RuntimeHttpGatewayRequest,
) -> RequestPayloadContext<'a> {
    RequestPayloadContext::new(
        target.gateway_entry_key().as_str(),
        &request.body,
        Some(BinaryHttpRequestContext::new(
            request.http_request.method.as_str(),
            request.http_request.url.as_str(),
            request.http_request.path.as_str(),
            request
                .http_request
                .query
                .iter()
                .map(|item| HttpNameValueContext::new(&item.name, &item.value))
                .collect(),
            request
                .http_request
                .headers
                .iter()
                .map(|item| HttpNameValueContext::new(&item.name, &item.value))
                .collect(),
            &request.body,
        )),
    )
}

struct RuntimeHttpGatewayRequestLifecycle {
    target: RuntimeAssemblyHttpGatewayTarget,
    cancelled: Arc<AtomicBool>,
}

impl RuntimeHttpGatewayRequestLifecycle {
    fn new(target: RuntimeAssemblyHttpGatewayTarget, cancelled: Arc<AtomicBool>) -> Self {
        Self { target, cancelled }
    }

    fn target(&self) -> &RuntimeAssemblyHttpGatewayTarget {
        &self.target
    }
}

impl Drop for RuntimeHttpGatewayRequestLifecycle {
    fn drop(&mut self) {
        let request_activation = self.target.eval().request_activation();
        if self.cancelled.load(Ordering::Acquire) {
            request_activation.cancel();
        }
        request_activation.end_request();
    }
}

#[cfg(test)]
#[path = "http_gateway_execution/tests.rs"]
mod tests;
