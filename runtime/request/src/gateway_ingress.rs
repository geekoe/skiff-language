use skiff_artifact_model::{
    AssemblyIdentity, GatewayDispatchMode, GatewayEntryIdentity, ServiceDeploymentRef,
    WebSocketEntryId,
};

use crate::{BinaryHttpRequestMetadata, HttpNameValue};

/// Exact activation facts retained after Host admits a gateway transport frame.
///
/// This is a request-layer projection, not a protocol frame. Host constructs it from the
/// already-decoded wire header and request execution uses it only to keep the resolved target
/// pinned to the admitted activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeGatewayIngressPin {
    pub assembly_identity: AssemblyIdentity,
    pub assembly_generation: u64,
    pub deployment: ServiceDeploymentRef,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

/// Request-owned HTTP ingress produced by the Host transport adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHttpGatewayRequest {
    pub request_id: String,
    pub dispatch_mode: GatewayDispatchMode,
    pub pin: RuntimeGatewayIngressPin,
    pub ingress_method: String,
    pub ingress_path: String,
    pub http_request: BinaryHttpRequestMetadata,
    pub body: Vec<u8>,
    pub test_effects_enabled: bool,
}

/// Request-owned WebSocket connect ingress produced by the Host transport adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWebSocketConnectIngress {
    pub request_id: String,
    pub pin: RuntimeGatewayIngressPin,
    pub ingress_path: String,
    pub connection_id: String,
    pub url: String,
    pub query: Vec<HttpNameValue>,
    pub headers: Vec<HttpNameValue>,
    pub cookies: Vec<HttpNameValue>,
    pub version: Option<String>,
    pub websocket_entry_id: WebSocketEntryId,
    pub connect_gateway_entry_identity: GatewayEntryIdentity,
    pub test_effects_enabled: bool,
}

/// Request-owned WebSocket connection-close notification ingress produced by
/// the Host transport adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeWebSocketConnectionClosedIngress {
    pub request_id: String,
    pub pin: RuntimeGatewayIngressPin,
    pub ingress_path: String,
    pub connection_id: String,
    pub websocket_entry_id: WebSocketEntryId,
    pub close_gateway_entry_identity: GatewayEntryIdentity,
    pub business_identity: Option<String>,
    pub close_code: Option<u16>,
    pub close_reason: Option<String>,
    pub test_effects_enabled: bool,
}
