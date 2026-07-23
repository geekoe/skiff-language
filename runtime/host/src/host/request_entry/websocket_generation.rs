use serde_json::Value;
use skiff_artifact_model::IngressSelector;
use skiff_runtime_request::{RequestEnvelope, WebSocketAdapterKind};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyWebSocketAdapterKindFrameHeader,
};

use crate::{
    error::{Result, RuntimeError},
    host::RuntimeHost,
    loader::assembly_admission::ActiveAssemblyRoute,
};

#[derive(Debug)]
pub(super) struct WebSocketConnectGenerationPin {
    pub(super) router_session_id: String,
    pub(super) websocket_entry_id: String,
    pub(super) connection_id: String,
}

impl RuntimeHost {
    pub(super) fn runtime_assembly_route_for_wire(
        &self,
        router_session_id: &str,
        header: &RuntimeAssemblyRequestStartFrameHeader,
        selector: &IngressSelector,
    ) -> Result<ActiveAssemblyRoute> {
        let Some(adapter) = &header.websocket_adapter else {
            return self.lookup_active_assembly_request_route(selector);
        };
        if adapter.kind != RuntimeAssemblyWebSocketAdapterKindFrameHeader::Receive {
            return self.lookup_active_assembly_request_route(selector);
        }
        let receive = adapter.receive_event.as_ref().ok_or_else(|| {
            RuntimeError::Decode(
                "canonical WebSocket receive requires receiveEvent metadata".to_string(),
            )
        })?;
        let websocket_entry_id = header.websocket_entry_id.as_deref().ok_or_else(|| {
            RuntimeError::Decode(
                "canonical WebSocket receive requires websocketEntryId".to_string(),
            )
        })?;
        self.websocket_generations.pinned_route(
            router_session_id,
            &receive.connection_id,
            &header.routing.assembly_identity,
            header.routing.assembly_generation,
            websocket_entry_id,
        )
    }
}

pub(super) fn websocket_connect_generation_pin(
    router_session_id: &str,
    request: &RequestEnvelope,
) -> Result<Option<WebSocketConnectGenerationPin>> {
    let Some(adapter) = &request.websocket_adapter else {
        return Ok(None);
    };
    if adapter.kind != WebSocketAdapterKind::Connect {
        return Ok(None);
    }
    let connect = adapter.connect_request.as_ref().ok_or_else(|| {
        RuntimeError::Decode(
            "canonical WebSocket connect requires connectRequest metadata".to_string(),
        )
    })?;
    let websocket_entry_id = request
        .extra
        .get("websocketEntryId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Decode(
                "canonical WebSocket connect requires websocketEntryId".to_string(),
            )
        })?;
    Ok(Some(WebSocketConnectGenerationPin {
        router_session_id: router_session_id.to_string(),
        websocket_entry_id: websocket_entry_id.to_string(),
        connection_id: connect.connection_id.clone(),
    }))
}
