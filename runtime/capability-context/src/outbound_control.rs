pub use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorActivationSnapshotControl, ActorControlDeadline,
    ActorFindControlRequest, ActorGetOrCreateControlRequest, ActorKeyControlMetadata,
    ActorMethodTaskTargetControl, ActorRemoveControlRequest, ActorReplaceControlRequest,
    ConnectionRequestCancelControl, ConnectionRequestControl, ConnectionSendControl,
    OutboundControlMessage, RequestCancelControl, RuntimeClientSessionControl,
    RuntimeDeadlineControl, TaskCallerKind, TaskCancelControlRequest, TaskCancelControlResponse,
    TaskStatusControlRequest, TaskStatusControlResponse, TaskSubmitControlRequest,
    TaskSubmitResponseControl, TaskSubmitTimingControl, WebSocketConnectionPolicyControl,
    WebSocketConnectionPolicyOverflowControl,
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

/// Closed failure namespace for one runtime WebSocket stream-frame write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouterWriteFailure {
    WebSocketWrite { message: String },
    SessionClosed,
}

impl std::fmt::Display for RouterWriteFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WebSocketWrite { message } => {
                write!(formatter, "runtime WebSocket write failed: {message}")
            }
            Self::SessionClosed => {
                formatter.write_str("runtime Router session closed before stream-frame flush")
            }
        }
    }
}

impl std::error::Error for RouterWriteFailure {}

/// Sole receipt for one non-cloneable stream-frame write.
#[must_use = "server-stream production must await the WebSocket flush receipt"]
pub struct RouterStreamFrameFlush {
    receiver: tokio::sync::oneshot::Receiver<Result<(), RouterWriteFailure>>,
}

impl RouterStreamFrameFlush {
    /// Resolves only after the WebSocket sink flushes the exact frame, or to
    /// the closed writer/session failure that prevented that flush.
    pub async fn wait(self) -> Result<(), RouterWriteFailure> {
        match self.receiver.await {
            Ok(result) => result,
            Err(_) => Err(RouterWriteFailure::SessionClosed),
        }
    }
}

#[derive(Debug)]
pub enum RouterWriterMessage {
    Binary(Vec<u8>),
    /// One server-stream frame whose producer must not advance until the
    /// runtime WebSocket writer has flushed these exact bytes.
    ///
    /// The acknowledgement sender deliberately makes this message
    /// non-cloneable. Dropping the queued message closes the one-shot and is
    /// therefore observable as an error by the producer.
    StreamFrame {
        bytes: Vec<u8>,
        flush_ack: tokio::sync::oneshot::Sender<Result<(), RouterWriteFailure>>,
    },
    Control(skiff_runtime_request_contract::OutboundControlMessage),
    TaskSubmit(TaskSubmitControlMessage),
}

impl RouterWriterMessage {
    /// Creates one non-cloneable server-stream frame and its sole flush
    /// acknowledgement receiver.
    pub fn stream_frame(bytes: Vec<u8>) -> (Self, RouterStreamFrameFlush) {
        let (flush_ack, receiver) = tokio::sync::oneshot::channel();
        (
            Self::StreamFrame { bytes, flush_ack },
            RouterStreamFrameFlush { receiver },
        )
    }
}
