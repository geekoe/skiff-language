pub use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorControlDeadline, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorKeyControlMetadata, ActorMethodSpawnTargetControl,
    ActorRemoveControlRequest, ActorReplaceControlRequest, ConnectionRequestCancelControl,
    ConnectionRequestControl, ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RuntimeClientSessionControl, RuntimeDeadlineControl, SpawnSubmitControlRequest,
    WebSocketConnectionPolicyControl, WebSocketConnectionPolicyOverflowControl,
};

#[derive(Debug, Clone, PartialEq)]
pub enum RouterWriterMessage {
    Binary(Vec<u8>),
    Control(skiff_runtime_request_contract::OutboundControlMessage),
}
