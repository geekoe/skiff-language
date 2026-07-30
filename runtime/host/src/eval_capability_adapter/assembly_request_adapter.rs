use skiff_runtime_eval::program_execution::ProgramExecutionContext;
use skiff_runtime_request::{
    RuntimeHttpGatewayEvalAdapter, RuntimeHttpGatewayEvalExecutionInputParts,
    RuntimeSpawnEvalAdapter, RuntimeSpawnEvalExecutionInputParts,
    RuntimeWebSocketConnectEvalAdapter, RuntimeWebSocketConnectEvalExecutionInputParts,
    RuntimeWebSocketJsonRpcEvalAdapter, RuntimeWebSocketJsonRpcEvalExecutionInputParts,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestClientSessionFrameHeader,
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader, RuntimeAssemblySpawnRequestStartFrameHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
};

use super::assembly_execution_context::{
    RuntimeAssemblyEvalAdapterContextInput, RuntimeAssemblyExecutionContext,
    RuntimeAssemblyRequestMetadata,
};
use super::*;

pub(crate) struct RuntimeHttpGatewayEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyRequestStartFrameHeader,
}

pub(crate) fn http_gateway_eval_adapter(
    input: RuntimeHttpGatewayEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeHttpGatewayEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        input
            .header
            .test_effects_enabled
            .then_some(input.header.http_request.url),
        input.header.test_case_capability,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeWebSocketConnectEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
}

pub(crate) fn websocket_connect_eval_adapter(
    input: RuntimeWebSocketConnectEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeWebSocketConnectEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        None,
        None,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeWebSocketJsonRpcEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
}

pub(crate) fn websocket_jsonrpc_eval_adapter(
    input: RuntimeWebSocketJsonRpcEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeWebSocketJsonRpcEvalAdapter>> {
    let metadata = request_metadata(
        input.header.request_id,
        input.header.mode,
        &input.header.caller,
        input.header.client_session.as_ref(),
        input.header.deadline.as_ref(),
        &input.header.trace,
        input.header.test_effects_enabled,
        None,
        None,
    )?;
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

pub(crate) struct RuntimeSpawnEvalAdapterInput {
    pub(crate) context: RuntimeAssemblyEvalAdapterContextInput,
    pub(crate) header: RuntimeAssemblySpawnRequestStartFrameHeader,
}

pub(crate) fn spawn_eval_adapter(
    input: RuntimeSpawnEvalAdapterInput,
) -> anyhow::Result<Arc<dyn RuntimeSpawnEvalAdapter>> {
    let metadata = RuntimeAssemblyRequestMetadata {
        request_id: input.header.request_id,
        mode: input.header.mode,
        caller: serde_json::to_value(input.header.caller)?,
        client_session: None,
        deadline: input
            .header
            .deadline
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?,
        trace: serde_json::to_value(input.header.trace)?,
        test_effects_enabled: input.header.test_effects_enabled,
        test_ingress_url: None,
        test_case_capability: input.header.test_case_capability,
    };
    Ok(Arc::new(RuntimeAssemblyExecutionContext::new(
        input.context,
        metadata,
    )?))
}

fn request_metadata(
    request_id: String,
    mode: String,
    caller: &RuntimeAssemblyRequestCallerFrameHeader,
    client_session: Option<&RuntimeAssemblyRequestClientSessionFrameHeader>,
    deadline: Option<&RuntimeAssemblyRequestDeadlineFrameHeader>,
    trace: &RuntimeAssemblyRequestTraceFrameHeader,
    test_effects_enabled: bool,
    test_ingress_url: Option<String>,
    test_case_capability: Option<String>,
) -> anyhow::Result<RuntimeAssemblyRequestMetadata> {
    Ok(RuntimeAssemblyRequestMetadata {
        request_id,
        mode,
        caller: serde_json::to_value(caller)?,
        client_session: client_session.map(serde_json::to_value).transpose()?,
        deadline: deadline.map(serde_json::to_value).transpose()?,
        trace: serde_json::to_value(trace)?,
        test_effects_enabled,
        test_ingress_url,
        test_case_capability,
    })
}

impl RuntimeSpawnEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn begin_test_effect_execution(
        &self,
    ) -> skiff_runtime_request::RequestResult<
        Option<skiff_runtime_request::RuntimeSpawnTestEffectExecution>,
    > {
        let Some(capability) = self.test_case_capability.as_deref() else {
            return Ok(None);
        };
        let lease = self
            .test_http_entries
            .begin_derived(capability, self.request.request_id.clone())
            .map_err(|error| skiff_runtime_request::RequestError::Unsupported(error.to_string()))?;
        Ok(Some(
            skiff_runtime_request::RuntimeSpawnTestEffectExecution::new(lease.effects(), lease),
        ))
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeSpawnEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        target: &'a skiff_runtime_request::RuntimeAssemblySpawnTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeSpawnEvalExecutionInputParts {
            request: _,
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            target.eval(),
        )
    }
}

impl RuntimeHttpGatewayEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn begin_test_effect_execution(
        &self,
    ) -> skiff_runtime_request::RequestResult<
        Option<skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution>,
    > {
        let activation_id = self.activation.activation_id().as_str();
        if let Some(capability) = self.test_case_capability.as_deref() {
            if !self.request.test_effects_enabled {
                return Err(skiff_runtime_request::RequestError::Unsupported(
                    "test case capability cannot be used when test effects are disabled"
                        .to_string(),
                ));
            }
            let ingress_url = self.test_ingress_url.as_deref().ok_or_else(|| {
                skiff_runtime_request::RequestError::Unsupported(
                    "test HTTP ingress is missing its trusted ingress URL".to_string(),
                )
            })?;
            let lease = self
                .test_http_entries
                .begin_root_case(
                    capability,
                    self.request.request_id.clone(),
                    activation_id.to_string(),
                    ingress_url,
                    self.activation.identity().deployment.clone(),
                )
                .map_err(|error| {
                    skiff_runtime_request::RequestError::Unsupported(error.to_string())
                })?;
            let effects = lease.effects();
            return Ok(Some(
                skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution::root(
                    effects,
                    lease.finalize(),
                ),
            ));
        }
        if self.request.test_effects_enabled {
            return Err(skiff_runtime_request::RequestError::Unsupported(
                "test HTTP ingress is missing its opaque test case capability".to_string(),
            ));
        }
        let execution = self
            .test_http_entries
            .begin_nested_http(activation_id, self.request.request_id.clone())
            .map_err(|error| skiff_runtime_request::RequestError::Unsupported(error.to_string()))?;
        Ok(execution.map(|lease| {
            skiff_runtime_request::RuntimeHttpGatewayTestEffectExecution::nested(
                lease.effects(),
                lease,
            )
        }))
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeHttpGatewayEvalExecutionInputParts<'a>,
        _request_context: skiff_runtime_request::RequestPayloadContext<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeHttpGatewayEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

impl RuntimeWebSocketConnectEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketConnectEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let RuntimeWebSocketConnectEvalExecutionInputParts {
            execution,
            cancellation,
            cancelled: _,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}

impl RuntimeWebSocketJsonRpcEvalAdapter for RuntimeAssemblyExecutionContext {
    fn runtime_factory(&self) -> eval_capabilities::EvalRuntimeFactory {
        runtime_factory()
    }

    fn execution_context<'a>(
        &'a self,
        parts: RuntimeWebSocketJsonRpcEvalExecutionInputParts<'a>,
        interpreter: &'a skiff_runtime_eval::Interpreter,
        eval_target: &'a skiff_runtime_eval::RuntimeAssemblyEvalTarget,
    ) -> skiff_runtime_eval::program_execution::ProgramExecutionContext<'a> {
        let RuntimeWebSocketJsonRpcEvalExecutionInputParts {
            execution,
            cancellation,
            execution_budget: _,
            request_heap_limits,
        } = parts;
        self.program_execution_context(
            execution,
            cancellation,
            request_heap_limits,
            interpreter,
            eval_target,
        )
    }
}
