use std::sync::Arc;

use tokio::sync::mpsc;

use crate::error::{Result, RuntimeError};

use skiff_runtime_capability_context::{
    ConnectionRequestCancelControl, ConnectionRequestControl, ConnectionRequestRegistry,
    ConnectionRequestSession, ConnectionRequestTerminal, ConnectionSendControl, ExecutionScope,
    OutboundControlMessage, RouterWriterMessage, RuntimeDeadlineControl,
};

#[derive(Clone, Copy)]
pub struct WebsocketCapabilityContext<'a> {
    service_id: &'a str,
    websocket_entry_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<RouterWriterMessage>>,
    request_transport: Option<ConnectionRequestTransport<'a>>,
}

#[derive(Clone, Copy)]
struct ConnectionRequestTransport<'a> {
    registry: &'a ConnectionRequestRegistry,
    session: &'a ConnectionRequestSession,
    scope: &'a ExecutionScope,
    deadline_control: Option<&'a RuntimeDeadlineControl>,
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
            request_transport: None,
        }
    }

    pub fn with_request_transport(
        mut self,
        registry: &'a ConnectionRequestRegistry,
        session: &'a ConnectionRequestSession,
        scope: &'a ExecutionScope,
        deadline_control: Option<&'a RuntimeDeadlineControl>,
    ) -> Self {
        self.request_transport = Some(ConnectionRequestTransport {
            registry,
            session,
            scope,
            deadline_control,
        });
        self
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

    pub async fn request_json_to_connection(
        &self,
        connection_id: String,
        method: String,
        payload: Vec<u8>,
    ) -> ConnectionRequestTerminal {
        let Some(transport) = self.request_transport else {
            return ConnectionRequestTerminal::TransportUnavailable;
        };
        let Some(websocket_entry_id) = self.websocket_entry_id else {
            return ConnectionRequestTerminal::ConnectionUnavailable;
        };
        let Some(router_sender) = self.router_sender else {
            return ConnectionRequestTerminal::TransportUnavailable;
        };
        if skiff_artifact_model::WebSocketEntryId::parse(websocket_entry_id).is_err()
            || transport.scope.effective_deadline().is_some()
                != transport.deadline_control.is_some()
        {
            return ConnectionRequestTerminal::ProtocolError;
        }
        if connection_id.is_empty()
            || connection_id.trim() != connection_id
            || method.is_empty()
            || method.trim() != method
        {
            return ConnectionRequestTerminal::ProtocolError;
        }
        if method.len()
            > skiff_runtime_transport::connection_protocol::CONNECTION_REQUEST_MAX_METHOD_BYTES
            || payload.len()
                > skiff_runtime_transport::connection_protocol::CONNECTION_REQUEST_MAX_PAYLOAD_BYTES
        {
            return ConnectionRequestTerminal::ResourceLimit;
        }
        if payload.is_empty()
            || !serde_json::from_slice::<serde_json::Value>(&payload)
                .is_ok_and(|value| value.is_object() || value.is_array())
        {
            return ConnectionRequestTerminal::ProtocolError;
        }

        let cancel_sender = router_sender.clone();
        let mut pending = match transport.registry.install(
            transport.session.clone(),
            transport.scope.clone(),
            Arc::new(move |request_id, reason| {
                cancel_sender
                    .send(RouterWriterMessage::Control(
                        OutboundControlMessage::ConnectionRequestCancel {
                            request: ConnectionRequestCancelControl {
                                request_id: request_id.to_string(),
                                reason: reason.as_str().to_string(),
                            },
                        },
                    ))
                    .map_err(|_| ())
            }),
        ) {
            Ok(pending) => pending,
            Err(_) => return ConnectionRequestTerminal::ResourceLimit,
        };
        let request = ConnectionRequestControl {
            request_id: pending.request_id().to_string(),
            service_id: self.service_id.to_string(),
            websocket_entry_id: websocket_entry_id.to_string(),
            connection_id,
            method,
            deadline: transport.deadline_control.cloned(),
        };
        if router_sender
            .send(RouterWriterMessage::Control(
                OutboundControlMessage::ConnectionRequest { request, payload },
            ))
            .is_err()
        {
            transport.registry.complete(
                transport.session,
                pending.request_id(),
                ConnectionRequestTerminal::TransportUnavailable,
            );
        }
        pending.wait().await
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
        let entry_id = entry_id.ok_or_else(|| RuntimeError::ProviderUnavailable {
            target: target.to_string(),
            reason: "websocket entry id is not available".to_string(),
        })?;
        Ok(Some(entry_id.to_string()))
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

#[cfg(test)]
mod connection_request_tests {
    use super::*;
    use skiff_runtime_capability_context::{
        CancellationSource, ConnectionRequestRegistry, ConnectionRequestSession,
        ConnectionRequestTerminal, ExecutionScope,
    };

    #[tokio::test]
    async fn connection_request_installs_before_queue_and_returns_opaque_terminal() {
        let registry = ConnectionRequestRegistry::new(4);
        let session = ConnectionRequestSession::new("router-session-1").expect("canonical session");
        let cancellation = CancellationSource::new();
        let scope = ExecutionScope::request(cancellation.token(), None);
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "a".repeat(64));
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let context = WebsocketCapabilityContext::with_entry_id(
            "example.com/chat",
            Some(&websocket_entry_id),
            Some(&sender),
        )
        .with_request_transport(&registry, &session, &scope, None);

        let request = context.request_json_to_connection(
            "connection-1".to_string(),
            "chat.send".to_string(),
            br#"{"message":"hi"}"#.to_vec(),
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("request control"),
            terminal = &mut request => panic!("request settled before queue: {terminal:?}"),
        };
        let request_id = match queued {
            RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequest {
                request,
                payload,
            }) => {
                assert_eq!(request.service_id, "example.com/chat");
                assert_eq!(request.connection_id, "connection-1");
                assert_eq!(request.method, "chat.send");
                assert_eq!(payload, br#"{"message":"hi"}"#);
                request.request_id
            }
            other => panic!("unexpected writer message: {other:?}"),
        };
        assert_eq!(registry.pending_count(), 1);
        assert!(registry.complete(
            &session,
            &request_id,
            ConnectionRequestTerminal::Success(b"null".to_vec())
        ));
        assert_eq!(
            request.await,
            ConnectionRequestTerminal::Success(b"null".to_vec())
        );
        assert_eq!(registry.pending_count(), 0);
        assert_eq!(registry.active_lease_count(), 0);
        assert_eq!(registry.active_timer_count(), 0);
    }
}
