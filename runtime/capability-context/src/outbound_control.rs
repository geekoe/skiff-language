pub use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorControlDeadline, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorKeyControlMetadata, ActorMethodSpawnTargetControl,
    ActorRemoveControlRequest, ActorReplaceControlRequest, ConnectionRequestCancelControl,
    ConnectionRequestControl, ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RuntimeClientSessionControl, RuntimeDeadlineControl, SpawnCallerKind,
    SpawnSubmitControlRequest, WebSocketConnectionPolicyControl,
    WebSocketConnectionPolicyOverflowControl,
};

/// Canonical `spawn.submit.request` writer message after H-spawn-parent-cut.
///
/// The typed `caller_kind` is the closed parent-kind namespace; the driver
/// encodes it into `SpawnSubmitRequestFrameHeaderV2`. The legacy
/// `OutboundControlMessage::SpawnSubmit` shape (no `callerKind`) is rejected
/// by the driver with no compatible reader.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnSubmitControlMessage {
    pub request: SpawnSubmitControlRequest,
    pub payload: Vec<u8>,
    pub caller_kind: SpawnCallerKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouterWriterMessage {
    Binary(Vec<u8>),
    Control(skiff_runtime_request_contract::OutboundControlMessage),
    SpawnSubmit(SpawnSubmitControlMessage),
}
