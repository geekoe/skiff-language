#![allow(clippy::large_enum_variant)]

mod bytecode_ingress;
pub mod cancellation;
mod continuation_handoff;
mod envelope;
mod error;
pub mod execution_budget;
mod execution_control;
mod failure_projection;
mod http_executor;
mod outbound;
mod response_event;
mod response_stream_writer;
mod response_writer;
mod runner;
pub mod vm_heap;

pub use bytecode_ingress::{
    start_runtime_bytecode_request, BytecodeRequestExecution, BytecodeRequestExecutionHandles,
    BytecodeRequestExecutionInput,
};
pub use envelope::{
    BinaryHttpRequest, BinaryHttpRequestMetadata, GatewayAdapterArg, GatewayAdapterSource,
    HttpAdapter, HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestCancel,
    RequestEffectDouble, RequestEnvelope,
};
pub use error::{OrdinaryRequestError, RequestError, RequestResult};
pub use execution_budget::ExecutionBudget;
pub use execution_control::{ExecutionControl, OwnedExecutionControl};
pub use failure_projection::{
    ActiveCallSiteKey, ActiveRequestCallSite, AdmittedCallSiteProjection, ContinuationLaneId,
    ContinuationProjectionGuard, FailureProjectionError, FailureSite, RequestGeneration,
    ResumeOwnerKey, ResumeOwnerKind,
};
pub use http_executor::{
    BytecodeHttpExecutor, BytecodeHttpStream, BytecodeHttpStreamEvent, BytecodeSelfIngressContext,
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
pub use runner::{execution_budget_trace_attrs, response_error_to_telemetry_map};
pub use skiff_runtime_scheduler::{
    RequestExecutionOwnerInventory, RequestExecutionOwnerInventorySnapshot,
};
pub use vm_heap::RequestVmHeap;
