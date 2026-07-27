use std::{collections::BTreeSet, time::Instant};

use serde_json::Value;
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayDispatchMode,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface, PackageCallableId,
    PackageCallableSignature,
};
use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, LinkedExecutable};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    error::{BudgetReason, RuntimeError},
    program_execution::ProgramExecutionContext,
    runtime_ops::{runtime_from_wire_required_plan, runtime_to_wire_required_plan},
    Interpreter, RuntimeAssemblyEvalTarget,
};

/// The runtime-side payload ceiling shared by inbound params and encoded results.
///
/// Transport independently enforces the same one-mebibyte ceiling. The typed kernel repeats the
/// check because it is a trust boundary of its own and can be called without a wire decoder.
pub const RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Response-producing outcomes exposed to the later Host attachment.
///
/// Cancellation is deliberately absent. It is represented by
/// [`RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled`] so a Host cannot accidentally encode it
/// as an ordinary response outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeWebSocketJsonRpcExecutionOutcome {
    Success { payload: Vec<u8> },
    InvalidParams,
    InternalError,
    DeadlineExceeded,
}

impl RuntimeWebSocketJsonRpcExecutionOutcome {
    pub fn payload(&self) -> Option<&[u8]> {
        match self {
            Self::Success { payload } => Some(payload),
            Self::InvalidParams | Self::InternalError | Self::DeadlineExceeded => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeWebSocketJsonRpcExecutionTerminal {
    Response(RuntimeWebSocketJsonRpcExecutionOutcome),
    Cancelled,
}

impl RuntimeWebSocketJsonRpcExecutionTerminal {
    fn success(payload: Vec<u8>) -> Self {
        Self::Response(RuntimeWebSocketJsonRpcExecutionOutcome::Success { payload })
    }

    fn invalid_params() -> Self {
        Self::Response(RuntimeWebSocketJsonRpcExecutionOutcome::InvalidParams)
    }

    fn internal_error() -> Self {
        Self::Response(RuntimeWebSocketJsonRpcExecutionOutcome::InternalError)
    }

    fn deadline_exceeded() -> Self {
        Self::Response(RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeWebSocketJsonRpcRequest<'a> {
    pub params: &'a [u8],
    pub connection_id: &'a str,
    pub business_identity: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeWebSocketJsonRpcCallable<'a> {
    pub callable_id: &'a PackageCallableId,
    pub signature: &'a PackageCallableSignature,
    pub addr: &'a ExecutableAddr,
}

/// Exact eval-facing facts supplied by the generation-pinned request target.
pub trait RuntimeWebSocketJsonRpcExecutionTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget;
    fn gateway_entry_key(&self) -> &GatewayEntryKey;
    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface;
    fn adapter_plan(&self) -> &GatewayAdapterPlan;
    fn handler(&self) -> RuntimeWebSocketJsonRpcCallable<'_>;
}

impl Interpreter {
    /// Execute one generation-pinned WebSocket JSON-RPC method.
    ///
    /// The method consumes only already-linked target facts. It never performs a name lookup,
    /// current-assembly lookup, artifact read, or transport projection.
    pub async fn execute_runtime_websocket_jsonrpc(
        &self,
        context: ProgramExecutionContext<'_>,
        request: RuntimeWebSocketJsonRpcRequest<'_>,
        target: &impl RuntimeWebSocketJsonRpcExecutionTarget,
        cancellation: CancellationToken,
        deadline: Option<Instant>,
    ) -> RuntimeWebSocketJsonRpcExecutionTerminal {
        let execution = self.execute_runtime_websocket_jsonrpc_kernel(context, request, target);
        tokio::pin!(execution);
        let cancel_wait = cancellation.clone();

        match deadline {
            Some(deadline) => {
                tokio::select! {
                    biased;
                    _ = cancel_wait.wait_cancelled() => {
                        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
                    }
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                        cancellation.cancel();
                        RuntimeWebSocketJsonRpcExecutionTerminal::deadline_exceeded()
                    }
                    terminal = &mut execution => {
                        prefer_cancel_then_deadline(
                            terminal,
                            &cancellation,
                            Some(deadline),
                        )
                    }
                }
            }
            None => {
                tokio::select! {
                    biased;
                    _ = cancel_wait.wait_cancelled() => {
                        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
                    }
                    terminal = &mut execution => {
                        prefer_cancel_then_deadline(terminal, &cancellation, None)
                    }
                }
            }
        }
    }

    async fn execute_runtime_websocket_jsonrpc_kernel(
        &self,
        context: ProgramExecutionContext<'_>,
        request: RuntimeWebSocketJsonRpcRequest<'_>,
        target: &impl RuntimeWebSocketJsonRpcExecutionTarget,
    ) -> RuntimeWebSocketJsonRpcExecutionTerminal {
        if validate_execution_pin(&context, target).is_err() {
            return RuntimeWebSocketJsonRpcExecutionTerminal::internal_error();
        }
        let (handler, executable) = match validate_target(target) {
            Ok(validated) => validated,
            Err(()) => return RuntimeWebSocketJsonRpcExecutionTerminal::internal_error(),
        };
        let params = match decode_params_json(request.params) {
            Ok(params) => params,
            Err(()) => return RuntimeWebSocketJsonRpcExecutionTerminal::invalid_params(),
        };
        let mut heap = context.request_heap();
        let args = match handler_args(request, &params, target, handler, executable, &mut heap) {
            Ok(args) => args,
            Err(HandlerArgumentError::InvalidParams) => {
                return RuntimeWebSocketJsonRpcExecutionTerminal::invalid_params()
            }
            Err(HandlerArgumentError::InvalidTarget) => {
                return RuntimeWebSocketJsonRpcExecutionTerminal::internal_error()
            }
        };
        let value = match self
            .execute_runtime_assembly_addr(context, &mut heap, handler.addr, args)
            .await
        {
            Ok(value) => value,
            Err(error) => return terminal_from_runtime_error(&error),
        };
        let return_plan = match callable_return_plan(target, handler, executable) {
            Ok(plan) => plan,
            Err(()) => return RuntimeWebSocketJsonRpcExecutionTerminal::internal_error(),
        };
        match encode_result(&value, &return_plan, &mut heap) {
            Ok(payload) => RuntimeWebSocketJsonRpcExecutionTerminal::success(payload),
            Err(()) => RuntimeWebSocketJsonRpcExecutionTerminal::internal_error(),
        }
    }
}

fn prefer_cancel_then_deadline(
    terminal: RuntimeWebSocketJsonRpcExecutionTerminal,
    cancellation: &CancellationToken,
    deadline: Option<Instant>,
) -> RuntimeWebSocketJsonRpcExecutionTerminal {
    if cancellation.is_cancelled() {
        return RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        cancellation.cancel();
        return RuntimeWebSocketJsonRpcExecutionTerminal::deadline_exceeded();
    }
    terminal
}

fn validate_execution_pin(
    context: &ProgramExecutionContext<'_>,
    target: &impl RuntimeWebSocketJsonRpcExecutionTarget,
) -> Result<(), ()> {
    let pinned = context.runtime_assembly_target().map_err(|_| ())?;
    let expected = target.eval_target();
    if !std::sync::Arc::ptr_eq(pinned.execution_image(), expected.execution_image())
        || !std::sync::Arc::ptr_eq(pinned.activation_context(), expected.activation_context())
        || pinned.request_activation().generation() != expected.request_activation().generation()
    {
        return Err(());
    }
    expected.ensure_execution_ready().map_err(|_| ())
}

fn validate_target<'a>(
    target: &'a impl RuntimeWebSocketJsonRpcExecutionTarget,
) -> Result<(RuntimeWebSocketJsonRpcCallable<'a>, &'a LinkedExecutable), ()> {
    let GatewayProtocolSurface::WebSocketJsonRpc(surface) = &target.protocol_surface().protocol
    else {
        return Err(());
    };
    if surface.dispatch_mode != GatewayDispatchMode::Unary
        || target.adapter_plan().kind != GatewayAdapterKind::WebSocketJsonRpc
    {
        return Err(());
    }
    let handler = target.handler();
    let executable = callable_executable(target, handler)?;
    let plan = target.adapter_plan();
    if executable.params.len() != plan.args.len()
        || executable
            .params
            .iter()
            .zip(&plan.args)
            .any(|(parameter, argument)| parameter.name != argument.param)
    {
        return Err(());
    }

    let formal_names = executable
        .params
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<BTreeSet<_>>();
    if formal_names.len() != executable.params.len() {
        return Err(());
    }
    let plan_names = plan
        .args
        .iter()
        .map(|argument| argument.param.as_str())
        .collect::<BTreeSet<_>>();
    if plan_names.len() != plan.args.len() || plan_names != formal_names {
        return Err(());
    }

    let mut params_count = 0usize;
    let mut connection_count = 0usize;
    let mut business_identity_count = 0usize;
    let mut plan_sources = BTreeSet::new();
    for argument in &plan.args {
        plan_sources.insert(argument.source);
        match argument.source {
            GatewayAdapterSource::WebSocketJsonRpcParams => params_count += 1,
            GatewayAdapterSource::WebSocketConnectionId => connection_count += 1,
            GatewayAdapterSource::WebSocketBusinessIdentity => business_identity_count += 1,
            GatewayAdapterSource::HttpRequest
            | GatewayAdapterSource::HttpBody
            | GatewayAdapterSource::HttpContext
            | GatewayAdapterSource::WebSocketConnectRequest => return Err(()),
        }
    }
    let surface_sources = surface
        .external_sources
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if params_count != 1
        || connection_count > 1
        || business_identity_count > 1
        || plan_sources.len() != plan.args.len()
        || surface_sources.len() != surface.external_sources.len()
        || plan_sources != surface_sources
    {
        return Err(());
    }
    Ok((handler, executable))
}

fn callable_executable<'a>(
    target: &'a impl RuntimeWebSocketJsonRpcExecutionTarget,
    callable: RuntimeWebSocketJsonRpcCallable<'a>,
) -> Result<&'a LinkedExecutable, ()> {
    let resolved = target
        .eval_target()
        .execution_projection()
        .resolve_executable(callable.addr)
        .map_err(|_| ())?;
    if resolved.addr != *callable.addr
        || resolved.executable.kind != ExecutableKind::Function
        || resolved.executable.self_type.is_some()
        || resolved.executable.return_type.is_none()
        || !callable.signature.type_params.is_empty()
        || resolved.executable.type_params != callable.signature.type_params
        || resolved.executable.may_suspend != callable.signature.may_suspend
        || resolved.executable.params.len() != callable.signature.parameters.len()
        || resolved
            .executable
            .params
            .iter()
            .zip(&callable.signature.parameters)
            .any(|(linked, declared)| linked.name != declared.name)
    {
        return Err(());
    }
    Ok(resolved.executable)
}

fn decode_params_json(payload: &[u8]) -> Result<Value, ()> {
    if payload.is_empty() || payload.len() > RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES {
        return Err(());
    }
    let text = std::str::from_utf8(payload).map_err(|_| ())?;
    let value: Value = serde_json::from_str(text).map_err(|_| ())?;
    if !value.is_object() && !value.is_array() {
        return Err(());
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandlerArgumentError {
    InvalidParams,
    InvalidTarget,
}

fn handler_args(
    request: RuntimeWebSocketJsonRpcRequest<'_>,
    params: &Value,
    target: &impl RuntimeWebSocketJsonRpcExecutionTarget,
    handler: RuntimeWebSocketJsonRpcCallable<'_>,
    executable: &LinkedExecutable,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>, HandlerArgumentError> {
    let plan_context = PlanContext::from_type_view(
        target.eval_target().execution_projection().type_view(),
        handler.addr,
    );
    let mut values = Vec::with_capacity(executable.params.len());
    for (parameter, argument) in executable.params.iter().zip(&target.adapter_plan().args) {
        if parameter.name != argument.param {
            return Err(HandlerArgumentError::InvalidTarget);
        }
        let plan = RuntimeTypePlan::from_linked_nested_ref(&parameter.ty, &plan_context)
            .map_err(|_| HandlerArgumentError::InvalidTarget)?;
        let wire = match argument.source {
            GatewayAdapterSource::WebSocketJsonRpcParams => params.clone(),
            GatewayAdapterSource::WebSocketConnectionId => {
                Value::String(request.connection_id.to_string())
            }
            GatewayAdapterSource::WebSocketBusinessIdentity => request
                .business_identity
                .map_or(Value::Null, |identity| Value::String(identity.to_string())),
            GatewayAdapterSource::HttpRequest
            | GatewayAdapterSource::HttpBody
            | GatewayAdapterSource::HttpContext
            | GatewayAdapterSource::WebSocketConnectRequest => {
                return Err(HandlerArgumentError::InvalidTarget)
            }
        };
        let value = runtime_from_wire_required_plan(
            &wire,
            Some(&plan),
            "websocket json-rpc adapter argument",
            heap,
        )
        .map_err(|_| {
            if argument.source == GatewayAdapterSource::WebSocketJsonRpcParams {
                HandlerArgumentError::InvalidParams
            } else {
                HandlerArgumentError::InvalidTarget
            }
        })?;
        values.push(value);
    }
    Ok(values)
}

fn callable_return_plan(
    target: &impl RuntimeWebSocketJsonRpcExecutionTarget,
    handler: RuntimeWebSocketJsonRpcCallable<'_>,
    executable: &LinkedExecutable,
) -> Result<RuntimeTypePlan, ()> {
    let return_type = executable.return_type.as_ref().ok_or(())?;
    RuntimeTypePlan::from_linked(
        return_type,
        &PlanContext::from_type_view(
            target.eval_target().execution_projection().type_view(),
            handler.addr,
        ),
    )
    .map_err(|_| ())
}

fn encode_result(
    value: &RuntimeValue,
    return_plan: &RuntimeTypePlan,
    heap: &mut RequestHeap,
) -> Result<Vec<u8>, ()> {
    let wire =
        runtime_to_wire_required_plan(value, Some(return_plan), "websocket json-rpc result", heap)
            .map_err(|_| ())?;
    if return_plan.named_type_name() == Some("void") && wire != Value::Null {
        return Err(());
    }
    let wire = if return_plan.named_type_name() == Some("void") {
        Value::Null
    } else {
        wire
    };
    let payload = serde_json::to_vec(&wire).map_err(|_| ())?;
    if payload.is_empty()
        || payload.len() > RUNTIME_WEBSOCKET_JSONRPC_MAX_PAYLOAD_BYTES
        || std::str::from_utf8(&payload).is_err()
    {
        return Err(());
    }
    Ok(payload)
}

fn terminal_from_runtime_error(error: &RuntimeError) -> RuntimeWebSocketJsonRpcExecutionTerminal {
    if error.is_cancellation_terminal() {
        return RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
    }
    if runtime_error_is_deadline(error) {
        return RuntimeWebSocketJsonRpcExecutionTerminal::deadline_exceeded();
    }
    RuntimeWebSocketJsonRpcExecutionTerminal::internal_error()
}

fn runtime_error_is_deadline(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::ExecutionBudgetExceeded { reason, .. } => {
            *reason == BudgetReason::DeadlineExceeded
        }
        RuntimeError::WithSource { error, .. }
        | RuntimeError::WithDiagnosticFrame { error, .. } => runtime_error_is_deadline(error),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
