pub use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorControlDeadline, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorKeyControlMetadata, ActorMethodTaskTargetControl,
    ActorRemoveControlRequest, ActorReplaceControlRequest, ConnectionRequestCancelControl,
    ConnectionRequestControl, ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RuntimeClientSessionControl, RuntimeDeadlineControl, TaskCallerKind, TaskCancelControlRequest,
    TaskCancelControlResponse, TaskStatusControlRequest, TaskStatusControlResponse,
    TaskSubmitControlRequest, TaskSubmitResponseControl, TaskSubmitTimingControl,
    WebSocketConnectionPolicyControl, WebSocketConnectionPolicyOverflowControl,
};

/// Canonical `task.submit.request` writer message after H-task-parent-cut.
///
/// The typed `caller_kind` is the closed parent-kind namespace; the driver
/// encodes it into `TaskSubmitRequestFrameHeaderV2`. The legacy
/// `OutboundControlMessage::TaskSubmit` shape (no `callerKind`) is rejected
/// by the driver with no compatible reader.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSubmitControlMessage {
    pub request: TaskSubmitControlRequest,
    pub payload: Vec<u8>,
    pub caller_kind: TaskCallerKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouterWriterMessage {
    Binary(Vec<u8>),
    Control(skiff_runtime_request_contract::OutboundControlMessage),
    TaskSubmit(TaskSubmitControlMessage),
}
