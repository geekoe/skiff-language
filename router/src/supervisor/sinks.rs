//! Production lane sinks installed by the composition into the session
//! inbound sink bundle (plan §5.5). Each sink owns its family codec; the
//! demux only performs framing/direction/payload-presence checks before
//! handing the raw frame over.

use std::sync::Arc;

use skiff_runtime_transport::connection_protocol::{
    decode_connection_request_cancel_frame, decode_connection_request_frame,
    CONNECTION_REQUEST_MAX_PAYLOAD_BYTES,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, ConnectionSendFrameHeader, RuntimeFrameFamily,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
};

use crate::session::demux::InboundFrameSink;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::TerminalKind;
use crate::ws::{BrokerRuntimeSource, RuntimeRequest, RuntimeSendOutcome, WebSocketLane};

use super::session_ports::{SessionHandle, WsRuntimeResponder};

/// Connection-family sink: Runtime generation lifecycle controls
/// (Acquire/Ack/Reject) go to the `RuntimeGenerationPinLedger` and Runtime
/// outbound RPCs (`connection.request` / `connection.request.cancel`) go to
/// the `WebSocketRequestBroker`.
#[derive(Debug, Clone)]
pub struct ConnectionFrameSink {
    lane: Arc<WebSocketLane>,
    session: SessionHandle,
}

impl ConnectionFrameSink {
    pub fn new(lane: Arc<WebSocketLane>, session: SessionHandle) -> Self {
        Self { lane, session }
    }

    fn write_lifecycle_control(
        &self,
        runtime: &RuntimeSessionEpoch,
        control: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), TerminalKind> {
        let bytes = encode_websocket_generation_lifecycle_frame(
            WebSocketGenerationLifecycleDirection::RouterToRuntime,
            control,
        )
        .map_err(|_| TerminalKind::MalformedFrame)?;
        let layer = self
            .session
            .layer()
            .ok_or(TerminalKind::UnimplementedFamily)?;
        layer
            .write_session_frame(runtime, bytes)
            .map_err(|_| TerminalKind::MalformedFrame)
    }

    fn handle_lifecycle(
        &self,
        runtime: &RuntimeSessionEpoch,
        control: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), TerminalKind> {
        match control {
            WebSocketGenerationLifecycleControl::Acquire { .. } => {
                let decision = self.lane.ledger.handle_acquire(runtime, control);
                match decision {
                    crate::ws::AcquireDecision::Ack(ack) => {
                        self.write_lifecycle_control(runtime, &ack)
                    }
                    crate::ws::AcquireDecision::Reject(reject) => {
                        self.write_lifecycle_control(runtime, &reject)
                    }
                }
            }
            WebSocketGenerationLifecycleControl::Ack { .. }
            | WebSocketGenerationLifecycleControl::Reject { .. } => self
                .lane
                .ledger
                .handle_release_response(runtime, control)
                .map_err(|_| TerminalKind::MalformedFrame),
            WebSocketGenerationLifecycleControl::Release { .. } => {
                Err(TerminalKind::MalformedFrame)
            }
        }
    }

    fn handle_request(
        &self,
        runtime: &RuntimeSessionEpoch,
        header: &skiff_runtime_transport::connection_protocol::ConnectionRequestFrameHeader,
        payload: Vec<u8>,
    ) -> Result<(), TerminalKind> {
        let source = BrokerRuntimeSource {
            sender: runtime.clone(),
            session_token: format!("{}#{}", runtime.replica_id, runtime.connection_generation),
            respond: Arc::new(WsRuntimeResponder::new(
                self.session.clone(),
                runtime.clone(),
            )),
        };
        let request = RuntimeRequest {
            request_id: header.request_id.clone(),
            service_id: header.service_id.clone(),
            websocket_entry_id: header.websocket_entry_id.as_str().to_string(),
            owner_token: self
                .lane
                .broker
                .owner_token(&header.connection_id)
                .map(|token| token.0)
                .unwrap_or(0),
            profile: header.profile,
            method: header.method.clone(),
            payload,
            deadline: header.deadline.clone(),
        };
        match self
            .lane
            .handle_runtime_request(&header.connection_id, &source, &request)
        {
            crate::ws::RuntimeRequestOutcome::Success
            | crate::ws::RuntimeRequestOutcome::ConnectionUnavailable
            | crate::ws::RuntimeRequestOutcome::ResourceLimit => Ok(()),
            crate::ws::RuntimeRequestOutcome::ProtocolError
            | crate::ws::RuntimeRequestOutcome::TransportUnavailable => {
                Err(TerminalKind::MalformedFrame)
            }
        }
    }

    fn handle_cancel(
        &self,
        runtime: &RuntimeSessionEpoch,
        header: &skiff_runtime_transport::connection_protocol::ConnectionRequestCancelFrameHeader,
    ) -> Result<(), TerminalKind> {
        let source = BrokerRuntimeSource {
            sender: runtime.clone(),
            session_token: format!("{}#{}", runtime.replica_id, runtime.connection_generation),
            respond: Arc::new(WsRuntimeResponder::new(
                self.session.clone(),
                runtime.clone(),
            )),
        };
        self.lane.handle_runtime_cancel(&source, &header.request_id);
        Ok(())
    }

    /// Runtime `connection.send` (server->client business message, TS
    /// parity). Delivery misses are non-fatal (the connection may have
    /// closed concurrently); protocol violations terminate the exact Runtime
    /// session, matching the TS 1008 close.
    fn handle_send(
        &self,
        _runtime: &RuntimeSessionEpoch,
        header: &ConnectionSendFrameHeader,
        payload: &[u8],
    ) -> Result<(), TerminalKind> {
        if header.envelope_type != "connection.send"
            || header.service_id.trim().is_empty()
            || payload.is_empty()
            || payload.len() > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES
        {
            return Err(TerminalKind::MalformedFrame);
        }
        let connection_id = header.connection_id.as_deref();
        let business_identity = header.business_identity.as_deref();
        // TS parity: exactly one of connectionId / businessIdentity.
        if connection_id.is_some() == business_identity.is_some() {
            return Err(TerminalKind::MalformedFrame);
        }
        let websocket_entry_id = header.websocket_entry_id.as_deref().unwrap_or_default();
        if websocket_entry_id.trim().is_empty() {
            return Err(TerminalKind::MalformedFrame);
        }
        let payload_kind = header.payload_kind.as_deref().unwrap_or_default();
        if payload_kind != "text" && payload_kind != "binary" {
            return Err(TerminalKind::MalformedFrame);
        }
        match self.lane.handle_runtime_send(
            connection_id,
            business_identity,
            &header.service_id,
            websocket_entry_id,
            payload_kind,
            payload,
        ) {
            RuntimeSendOutcome::Delivered => Ok(()),
            RuntimeSendOutcome::DeliveryMiss { reason } => {
                eprintln!("[connection.send] delivery miss: {reason}");
                Ok(())
            }
            RuntimeSendOutcome::ProtocolViolation { reason } => {
                eprintln!("[connection.send] protocol violation: {reason}");
                Err(TerminalKind::MalformedFrame)
            }
        }
    }
}

impl InboundFrameSink for ConnectionFrameSink {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Connection
    }

    fn accepts_frame_type(&self, frame_type: &str) -> bool {
        matches!(
            frame_type,
            "websocket.generation.lifecycle"
                | "connection.request"
                | "connection.request.cancel"
                | "connection.send"
        )
    }

    fn handle(&self, runtime: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        if let Ok(control) = decode_websocket_generation_lifecycle_frame(
            WebSocketGenerationLifecycleDirection::RuntimeToRouter,
            raw,
        ) {
            return self.handle_lifecycle(runtime, &control);
        }
        if let Ok((header, payload)) = decode_connection_request_frame(raw) {
            return self.handle_request(runtime, &header, payload);
        }
        if let Ok(header) = decode_connection_request_cancel_frame(raw) {
            return self.handle_cancel(runtime, &header);
        }
        if let Ok((header, payload)) = decode_typed_binary_frame::<ConnectionSendFrameHeader>(raw) {
            return self.handle_send(runtime, &header, &payload);
        }
        Err(TerminalKind::MalformedFrame)
    }
}
