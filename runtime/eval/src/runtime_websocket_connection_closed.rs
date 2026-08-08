use serde_json::Value;
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayAdapterPlan, GatewayAdapterSource, GatewayEntryIdentity,
    GatewayEntryKey, GatewayEntryProtocolSurface, GatewayProtocolSurface, PackageCallableId,
    PackageCallableSignature, WebSocketEntryId,
};
use skiff_runtime_linked_program::{ExecutableAddr, ExecutableKind, LinkedExecutable};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    error::{Result, RuntimeError},
    heap_access::HeapAccess,
    program_execution::ProgramExecutionContext,
    runtime_ops::runtime_from_wire_required_plan,
    Interpreter, RuntimeAssemblyEvalTarget,
};

/// One WebSocket connection-close notification delivered to a service.
///
/// The close handler is a notification: it receives scalar adapter sources and
/// returns void. No response frame is produced by the runtime for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWebSocketConnectionClosedRequest {
    pub connection_id: String,
    pub websocket_entry_id: WebSocketEntryId,
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub business_identity: Option<String>,
    pub close_code: Option<u16>,
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeWebSocketConnectionClosedCallable<'a> {
    pub callable_id: &'a PackageCallableId,
    pub signature: &'a PackageCallableSignature,
    pub addr: &'a ExecutableAddr,
}

pub trait RuntimeWebSocketConnectionClosedExecutionTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget;
    fn gateway_entry_key(&self) -> &GatewayEntryKey;
    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity;
    fn websocket_entry_id(&self) -> &WebSocketEntryId;
    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface;
    fn close_adapter_plan(&self) -> &GatewayAdapterPlan;
    fn close_handler(&self) -> RuntimeWebSocketConnectionClosedCallable<'_>;
}

impl Interpreter {
    pub async fn execute_runtime_websocket_connection_closed(
        &self,
        context: ProgramExecutionContext<'_>,
        request: &RuntimeWebSocketConnectionClosedRequest,
        target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
    ) -> Result<()> {
        validate_execution_pin(&context, target)?;
        validate_request(request, target)?;
        let mut heap = HeapAccess::private(context.request_heap());
        let handler = target.close_handler();
        let args = handler_args(request, target, handler, heap.heap_mut())?;
        self.execute_runtime_assembly_addr(context, &mut heap, handler.addr, args)
            .await?;
        // The close handler is a notification: its void return is not decoded
        // and no response is ever produced.
        Ok(())
    }
}

fn validate_execution_pin(
    context: &ProgramExecutionContext<'_>,
    target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
) -> Result<()> {
    let pinned = context.runtime_assembly_target()?;
    let expected = target.eval_target();
    if !std::sync::Arc::ptr_eq(pinned.execution_image(), expected.execution_image())
        || !std::sync::Arc::ptr_eq(pinned.activation_context(), expected.activation_context())
        || pinned.request_activation().generation() != expected.request_activation().generation()
    {
        return Err(protocol_error(
            target,
            "websocket connection closed execution context does not match the exact eval target",
        ));
    }
    expected.ensure_execution_ready()?;
    Ok(())
}

fn validate_request(
    request: &RuntimeWebSocketConnectionClosedRequest,
    target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
) -> Result<()> {
    if !matches!(
        target.protocol_surface().protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ) || target.close_adapter_plan().kind != GatewayAdapterKind::WebSocketConnectionClosed
    {
        return Err(protocol_error(
            target,
            "websocket connection closed execution requires the fixed connect protocol surface",
        ));
    }
    if request.websocket_entry_id != *target.websocket_entry_id()
        || request.gateway_entry_identity != *target.gateway_entry_identity()
    {
        return Err(protocol_error(
            target,
            "websocket connection closed request does not match the exact admitted entry identities",
        ));
    }
    Ok(())
}

fn handler_args(
    request: &RuntimeWebSocketConnectionClosedRequest,
    target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
    handler: RuntimeWebSocketConnectionClosedCallable<'_>,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let executable = callable_executable(target, handler)?;
    if executable.params.len() != target.close_adapter_plan().args.len() {
        return Err(protocol_error(
            target,
            "websocket connection closed adapter plan does not cover the exact handler signature",
        ));
    }
    let mut values = Vec::with_capacity(executable.params.len());
    for parameter in &executable.params {
        let arg = target
            .close_adapter_plan()
            .args
            .iter()
            .find(|arg| arg.param == parameter.name)
            .ok_or_else(|| {
                protocol_error(
                    target,
                    format!(
                        "websocket connection closed adapter plan is missing parameter {}",
                        parameter.name
                    ),
                )
            })?;
        let wire = websocket_connection_closed_source_wire(request, target, arg.source)?;
        let plan = RuntimeTypePlan::from_linked_nested_ref(
            &parameter.ty,
            &PlanContext::from_type_view(
                target.eval_target().execution_projection().type_view(),
                handler.addr,
            ),
        )?;
        values.push(runtime_from_wire_required_plan(
            &wire,
            Some(&plan),
            "websocket connection closed adapter argument",
            heap,
        )?);
    }
    Ok(values)
}

fn websocket_connection_closed_source_wire(
    request: &RuntimeWebSocketConnectionClosedRequest,
    target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
    source: GatewayAdapterSource,
) -> Result<Value> {
    match source {
        GatewayAdapterSource::WebSocketConnectionId => Ok(Value::String(request.connection_id.clone())),
        GatewayAdapterSource::WebSocketCloseCode => Ok(request
            .close_code
            .map_or(Value::Null, Value::from)),
        GatewayAdapterSource::WebSocketCloseReason => Ok(request
            .close_reason
            .clone()
            .map_or(Value::Null, Value::String)),
        GatewayAdapterSource::WebSocketBusinessIdentity => Ok(request
            .business_identity
            .clone()
            .map_or(Value::Null, Value::String)),
        GatewayAdapterSource::HttpRequest
        | GatewayAdapterSource::HttpBody
        | GatewayAdapterSource::HttpContext => Err(protocol_error(
            target,
            "websocket connection closed execution refuses HTTP adapter sources",
        )),
        GatewayAdapterSource::WebSocketConnectRequest
        | GatewayAdapterSource::WebSocketJsonRpcParams => Err(protocol_error(
            target,
            "websocket connection closed execution refuses WebSocket connect/JSON-RPC-only adapter sources",
        )),
    }
}

fn callable_executable<'a>(
    target: &'a impl RuntimeWebSocketConnectionClosedExecutionTarget,
    callable: RuntimeWebSocketConnectionClosedCallable<'a>,
) -> Result<&'a LinkedExecutable> {
    let resolved = target
        .eval_target()
        .execution_projection()
        .resolve_executable(callable.addr)?;
    if resolved.addr != *callable.addr
        || resolved.executable.kind != ExecutableKind::Function
        || resolved.executable.self_type.is_some()
        || resolved.executable.return_type.is_none()
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
        return Err(protocol_error(
            target,
            format!(
                "websocket connection closed callable {} does not match its exact linked signature",
                callable.callable_id
            ),
        ));
    }
    Ok(resolved.executable)
}

fn protocol_error(
    target: &impl RuntimeWebSocketConnectionClosedExecutionTarget,
    message: impl Into<String>,
) -> RuntimeError {
    RuntimeError::Protocol {
        target: target.gateway_entry_key().as_str().to_string(),
        message: message.into(),
    }
}
