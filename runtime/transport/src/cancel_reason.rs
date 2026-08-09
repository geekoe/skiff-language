use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestCancelReason {
    Timeout,
    CallerCancel,
    RuntimeDisconnect,
    GatewayDisconnect,
    Drain,
    Retire,
    ClientDisconnect,
    RouterShutdown,
    Backpressure,
    DeadlineExceeded,
    ProtocolError,
    StreamDropped,
}

impl RequestCancelReason {
    pub const CONTRACT_H: [RequestCancelReason; 9] = [
        RequestCancelReason::CallerCancel,
        RequestCancelReason::ClientDisconnect,
        RequestCancelReason::Timeout,
        RequestCancelReason::DeadlineExceeded,
        RequestCancelReason::Backpressure,
        RequestCancelReason::ProtocolError,
        RequestCancelReason::StreamDropped,
        RequestCancelReason::RuntimeDisconnect,
        RequestCancelReason::RouterShutdown,
    ];

    /// Whether this reason is a frozen `CONTRACT_H` wire value
    /// (C-model-request §4). The codec rejects any other reason.
    pub const fn is_contract_h(self) -> bool {
        matches!(
            self,
            RequestCancelReason::CallerCancel
                | RequestCancelReason::ClientDisconnect
                | RequestCancelReason::Timeout
                | RequestCancelReason::DeadlineExceeded
                | RequestCancelReason::Backpressure
                | RequestCancelReason::ProtocolError
                | RequestCancelReason::StreamDropped
                | RequestCancelReason::RuntimeDisconnect
                | RequestCancelReason::RouterShutdown
        )
    }

    /// Parses a wire reason and rejects values outside the frozen
    /// `CONTRACT_H` vocabulary (legacy internal reasons such as
    /// `gateway_disconnect` / `drain` / `retire` are not legal wire values).
    pub fn from_contract_h_wire(reason: &str) -> Option<Self> {
        let parsed = Self::from_wire(reason)?;
        parsed.is_contract_h().then_some(parsed)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RequestCancelReason::Timeout => "timeout",
            RequestCancelReason::CallerCancel => "caller_cancel",
            RequestCancelReason::RuntimeDisconnect => "runtime_disconnect",
            RequestCancelReason::GatewayDisconnect => "gateway_disconnect",
            RequestCancelReason::Drain => "drain",
            RequestCancelReason::Retire => "retire",
            RequestCancelReason::ClientDisconnect => "client_disconnect",
            RequestCancelReason::RouterShutdown => "router_shutdown",
            RequestCancelReason::Backpressure => "backpressure",
            RequestCancelReason::DeadlineExceeded => "deadline_exceeded",
            RequestCancelReason::ProtocolError => "protocol_error",
            RequestCancelReason::StreamDropped => "stream_dropped",
        }
    }

    pub fn from_wire(reason: &str) -> Option<Self> {
        match reason {
            "timeout" => Some(RequestCancelReason::Timeout),
            "caller_cancel" => Some(RequestCancelReason::CallerCancel),
            "runtime_disconnect" => Some(RequestCancelReason::RuntimeDisconnect),
            "gateway_disconnect" => Some(RequestCancelReason::GatewayDisconnect),
            "drain" => Some(RequestCancelReason::Drain),
            "retire" => Some(RequestCancelReason::Retire),
            "client_disconnect" => Some(RequestCancelReason::ClientDisconnect),
            "router_shutdown" => Some(RequestCancelReason::RouterShutdown),
            "backpressure" => Some(RequestCancelReason::Backpressure),
            "deadline_exceeded" => Some(RequestCancelReason::DeadlineExceeded),
            "protocol_error" => Some(RequestCancelReason::ProtocolError),
            "stream_dropped" => Some(RequestCancelReason::StreamDropped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestCancelSituation {
    CallerAbort,
    ClientDisconnect,
    Timeout,
    DeadlineExceeded,
    Backpressure,
    ProtocolError,
    StreamDropped,
    RuntimeDisconnect,
    RouterShutdown,
}

impl RequestCancelSituation {
    pub const CONTRACT_H: [RequestCancelSituation; 9] = [
        RequestCancelSituation::CallerAbort,
        RequestCancelSituation::ClientDisconnect,
        RequestCancelSituation::Timeout,
        RequestCancelSituation::DeadlineExceeded,
        RequestCancelSituation::Backpressure,
        RequestCancelSituation::ProtocolError,
        RequestCancelSituation::StreamDropped,
        RequestCancelSituation::RuntimeDisconnect,
        RequestCancelSituation::RouterShutdown,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestCancelReasonMapping<'a> {
    pub internal_reason: &'a str,
    pub wire_reason: RequestCancelReason,
}

pub fn request_cancel_reason_for_situation(
    situation: RequestCancelSituation,
) -> RequestCancelReason {
    match situation {
        RequestCancelSituation::CallerAbort => RequestCancelReason::CallerCancel,
        RequestCancelSituation::ClientDisconnect => RequestCancelReason::ClientDisconnect,
        RequestCancelSituation::Timeout => RequestCancelReason::Timeout,
        RequestCancelSituation::DeadlineExceeded => RequestCancelReason::DeadlineExceeded,
        RequestCancelSituation::Backpressure => RequestCancelReason::Backpressure,
        RequestCancelSituation::ProtocolError => RequestCancelReason::ProtocolError,
        RequestCancelSituation::StreamDropped => RequestCancelReason::StreamDropped,
        RequestCancelSituation::RuntimeDisconnect => RequestCancelReason::RuntimeDisconnect,
        RequestCancelSituation::RouterShutdown => RequestCancelReason::RouterShutdown,
    }
}

pub fn map_internal_request_cancel_reason(internal_reason: &str) -> RequestCancelReasonMapping<'_> {
    let wire_reason =
        RequestCancelReason::from_wire(internal_reason).unwrap_or(match internal_reason {
            "caller_abort" => RequestCancelReason::CallerCancel,
            "unexpected_stream_response"
            | "unexpected_control_response"
            | "response_channel_closed"
            | "duplicate_response_start"
            | "chunk_before_start"
            | "chunk_seq_mismatch"
            | "chunk_decode_error"
            | "stream_end_payload" => RequestCancelReason::ProtocolError,
            "stream_cancelled" => RequestCancelReason::StreamDropped,
            _ => RequestCancelReason::CallerCancel,
        });

    RequestCancelReasonMapping {
        internal_reason,
        wire_reason,
    }
}

pub fn request_cancel_wire_reason_for_internal(internal_reason: &str) -> &'static str {
    map_internal_request_cancel_reason(internal_reason)
        .wire_reason
        .as_str()
}

#[cfg(test)]
mod tests;
