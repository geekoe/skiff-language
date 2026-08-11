#![allow(clippy::large_enum_variant)]
#![allow(clippy::result_large_err)]

// RequestError intentionally carries eval and boundary payloads by value; the
// large-error lints are advisory in the normal workspace gate.

mod assembly_ingress;
mod assembly_seam;
mod bytecode_ingress;
pub mod cancellation;
mod context;
mod effect_context;
mod envelope;
mod error;
mod eval_adapter;
pub mod execution_budget;
mod execution_control;
mod gateway_ingress;
mod http_gateway_execution;
mod http_gateway_target;
mod http_ingress;
mod ingress;
#[path = "eval_invocation_builder.rs"]
mod invocation_builder;
mod operation_context;
mod outbound;
mod response_event;
mod response_stream_writer;
mod response_writer;
mod runner;
mod runtime_ingress;
mod task_execution;
pub mod vm_heap;
mod websocket_connect_execution;
mod websocket_connect_target;
mod websocket_connection_closed_execution;
mod websocket_connection_closed_target;
mod websocket_jsonrpc_execution;
mod websocket_jsonrpc_target;

pub use assembly_ingress::{
    execute_runtime_assembly_request, AssemblyRequestExecutionHandles,
    AssemblyRequestExecutionInput,
};
pub use assembly_seam::{RuntimeAssemblyRequestSeamError, RuntimeAssemblyRequestTarget};
pub use bytecode_ingress::{
    execute_runtime_bytecode_request, execute_runtime_bytecode_request_with_ports,
    BytecodeChildExecutor, BytecodeChildStart, BytecodeHandoff, BytecodeInvocationHandoff,
    BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput, BytecodeRequestExecutionPorts,
    BytecodeRequestTarget, BytecodeRequestTargetError, BytecodeSchedulerError,
    BytecodeSchedulerPorts, BytecodeStreamSupervisor, SuspendedTrampoline,
};
pub use context::{
    invocation_context_from_request, request_payload_context_from_request, RequestPayloadContext,
};
pub use effect_context::request_deadline_ms;
pub use envelope::{
    BinaryHttpRequest, BinaryHttpRequestMetadata, GatewayAdapterArg, GatewayAdapterSource,
    HttpAdapter, HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestCancel,
    RequestEffectDouble, RequestEnvelope,
};
pub use error::{OrdinaryRequestError, RequestError, RequestResult};
pub use eval_adapter::{
    AssemblyRequestEvalAdapter, RequestEvalAdapter, RequestEvalExecutionInputParts,
};
pub use execution_budget::ExecutionBudget;
pub use execution_control::{ExecutionControl, OwnedExecutionControl};
pub use gateway_ingress::{
    RuntimeGatewayIngressPin, RuntimeHttpGatewayRequest, RuntimeWebSocketConnectIngress,
    RuntimeWebSocketConnectionClosedIngress,
};
pub use http_gateway_execution::{
    execute_runtime_http_gateway_request, RuntimeHttpGatewayEvalAdapter,
    RuntimeHttpGatewayEvalExecutionInputParts, RuntimeHttpGatewayExecutionHandles,
    RuntimeHttpGatewayExecutionInput, RuntimeHttpGatewayTestEffectExecution,
};
pub use http_gateway_target::{
    RuntimeAssemblyHttpGatewayTarget, RuntimeAssemblyHttpGatewayTargetError,
};
pub use operation_context::{
    RequestOperationContext, RequestServiceMetadata, RuntimeOperation, RuntimeOperationParameter,
};
pub use outbound::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorKeyControlMetadata, ActorRemoveControlRequest, ActorReplaceControlRequest,
    ConnectionSendControl, OutboundControlMessage, OutboundRequestLease, OutboundRequestRegistry,
    OutboundResponse, OutboundResponseReceiver, RequestCancelControl, RouterWriterMessage,
    RuntimeDeadlineControl, TaskCallerKind, TaskCancelControlRequest, TaskCancelControlResponse,
    TaskStatusControlRequest, TaskStatusControlResponse, TaskSubmitControlMessage,
    TaskSubmitControlRequest, TaskSubmitTimingControl,
};
pub use response_event::{
    BoundaryResponse, FixedServiceResponseFailure, HttpResponseMetadata, ResponseEnd,
    ResponseError, ResponseEvent, ResponseStreamEvent,
};
pub use response_writer::ResponseEventSink;
pub use runner::{
    execute_runtime_request, execution_budget_trace_attrs, response_error_to_telemetry_map,
    RequestExecutionError, RequestExecutionHandles, RequestExecutionInput, RequestExecutionResult,
    RuntimeResponse,
};
pub use skiff_runtime_eval::{
    RuntimeWebSocketJsonRpcExecutionOutcome, RuntimeWebSocketJsonRpcExecutionTerminal,
};
pub use task_execution::{
    execute_runtime_task_request, RuntimeAssemblyTaskTarget, RuntimeTaskEvalAdapter,
    RuntimeTaskEvalExecutionInputParts, RuntimeTaskExecutionHandles, RuntimeTaskExecutionInput,
    RuntimeTaskRequest, RuntimeTaskTestEffectExecution,
};
pub use vm_heap::RequestVmHeap;
pub use websocket_connect_execution::{
    execute_runtime_websocket_connect, RuntimeWebSocketConnectEvalAdapter,
    RuntimeWebSocketConnectEvalExecutionInputParts, RuntimeWebSocketConnectExecutionHandles,
    RuntimeWebSocketConnectExecutionInput,
};
pub use websocket_connect_target::{
    RuntimeAssemblyWebSocketConnectTarget, RuntimeAssemblyWebSocketConnectTargetError,
};
pub use websocket_connection_closed_execution::{
    execute_runtime_websocket_connection_closed, RuntimeWebSocketConnectionClosedEvalAdapter,
    RuntimeWebSocketConnectionClosedEvalExecutionInputParts,
    RuntimeWebSocketConnectionClosedExecutionHandles,
    RuntimeWebSocketConnectionClosedExecutionInput,
};
pub use websocket_connection_closed_target::{
    RuntimeAssemblyWebSocketConnectionClosedTarget,
    RuntimeAssemblyWebSocketConnectionClosedTargetError,
};
pub use websocket_jsonrpc_execution::{
    execute_runtime_websocket_jsonrpc, RuntimeWebSocketJsonRpcEvalAdapter,
    RuntimeWebSocketJsonRpcEvalExecutionInputParts, RuntimeWebSocketJsonRpcExecutionHandles,
    RuntimeWebSocketJsonRpcExecutionInput,
};
pub use websocket_jsonrpc_target::{
    RuntimeAssemblyWebSocketJsonRpcPhysicalRoute, RuntimeAssemblyWebSocketJsonRpcTarget,
    RuntimeAssemblyWebSocketJsonRpcTargetError,
};
