use tokio::sync::mpsc;

use crate::error::{Result, RuntimeError};

use skiff_runtime_capability_context::{
    ConnectionSendControl, OutboundControlMessage, RouterWriterMessage,
};

#[derive(Clone, Copy)]
pub struct WebsocketCapabilityContext<'a> {
    service_id: &'a str,
    websocket_entry_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
}

impl<'a> WebsocketCapabilityContext<'a> {
    pub fn with_entry_id(
        service_id: &'a str,
        websocket_entry_id: Option<&'a str>,
        router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
    ) -> Self {
        Self {
            service_id,
            websocket_entry_id,
            router_sender,
        }
    }

    pub fn service_id(&self) -> &'a str {
        self.service_id
    }

    pub fn websocket_entry_id(&self) -> Option<&'a str> {
        self.websocket_entry_id
    }

    /// Clones the router-writer sender (cheap; `mpsc` senders are `Clone`) so an
    /// owned execution context can keep emitting connection frames after the
    /// borrow scope ends.
    pub fn router_sender_handle(&self) -> Option<mpsc::UnboundedSender<RouterWriterMessage>> {
        self.router_sender.cloned()
    }

    pub fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> Result<()> {
        let business_identity = self.validate_websocket_target(
            business_identity,
            "std.websocket.sendTextToBusinessIdentity",
        )?;
        self.send_connection_frame(
            ConnectionSendTarget::BusinessIdentity(business_identity),
            text.into_bytes(),
            "text",
            "std.websocket.sendTextToBusinessIdentity",
        )
    }

    pub fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> Result<()> {
        let business_identity = self.validate_websocket_target(
            business_identity,
            "std.websocket.sendBinaryToBusinessIdentity",
        )?;
        self.send_connection_frame(
            ConnectionSendTarget::BusinessIdentity(business_identity),
            payload,
            "binary",
            "std.websocket.sendBinaryToBusinessIdentity",
        )
    }

    pub fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> Result<()> {
        let connection_id =
            self.validate_websocket_target(connection_id, "std.websocket.sendTextToConnection")?;
        self.send_connection_frame(
            ConnectionSendTarget::Connection(connection_id),
            text.into_bytes(),
            "text",
            "std.websocket.sendTextToConnection",
        )
    }

    pub fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> Result<()> {
        let connection_id =
            self.validate_websocket_target(connection_id, "std.websocket.sendBinaryToConnection")?;
        self.send_connection_frame(
            ConnectionSendTarget::Connection(connection_id),
            payload,
            "binary",
            "std.websocket.sendBinaryToConnection",
        )
    }

    fn send_connection_frame(
        &self,
        connection_target: ConnectionSendTarget,
        payload: Vec<u8>,
        payload_kind: &str,
        target: &str,
    ) -> Result<()> {
        let sender = self
            .router_sender
            .ok_or_else(|| RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: "router writer is not available".to_string(),
            })?;
        let request = ConnectionSendControl {
            service_id: self.service_id.to_string(),
            websocket_entry_id: connection_target
                .websocket_entry_id(self.websocket_entry_id, target)?,
            business_identity: connection_target.business_identity(),
            connection_id: connection_target.connection_id(),
            payload_kind: Some(payload_kind.to_string()),
        };
        sender
            .send(RouterWriterMessage::Control(
                OutboundControlMessage::ConnectionSend { request, payload },
            ))
            .map_err(|_| RuntimeError::ProviderUnavailable {
                target: target.to_string(),
                reason: "router writer channel closed".to_string(),
            })
    }

    fn validate_websocket_target(&self, value: String, target: &str) -> Result<String> {
        if value.trim().is_empty() {
            return Err(RuntimeError::Decode(format!(
                "{target} target must be a non-empty string"
            )));
        }
        Ok(value)
    }
}

enum ConnectionSendTarget {
    BusinessIdentity(String),
    Connection(String),
}

impl ConnectionSendTarget {
    fn websocket_entry_id(&self, entry_id: Option<&str>, target: &str) -> Result<Option<String>> {
        match self {
            ConnectionSendTarget::BusinessIdentity(_) => {
                let entry_id = entry_id.ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "websocket entry id is not available".to_string(),
                })?;
                Ok(Some(entry_id.to_string()))
            }
            ConnectionSendTarget::Connection(_) => Ok(None),
        }
    }

    fn business_identity(&self) -> Option<String> {
        match self {
            ConnectionSendTarget::BusinessIdentity(value) => Some(value.clone()),
            ConnectionSendTarget::Connection(_) => None,
        }
    }

    fn connection_id(&self) -> Option<String> {
        match self {
            ConnectionSendTarget::BusinessIdentity(_) => None,
            ConnectionSendTarget::Connection(value) => Some(value.clone()),
        }
    }
}
