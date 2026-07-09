#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
        RequestCancelReason::from_wire(internal_reason).unwrap_or_else(|| match internal_reason {
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
mod tests {
    use super::*;

    #[test]
    fn contract_h_situations_map_to_stable_wire_reasons() {
        let mappings = [
            (RequestCancelSituation::CallerAbort, "caller_cancel"),
            (
                RequestCancelSituation::ClientDisconnect,
                "client_disconnect",
            ),
            (RequestCancelSituation::Timeout, "timeout"),
            (
                RequestCancelSituation::DeadlineExceeded,
                "deadline_exceeded",
            ),
            (RequestCancelSituation::Backpressure, "backpressure"),
            (RequestCancelSituation::ProtocolError, "protocol_error"),
            (RequestCancelSituation::StreamDropped, "stream_dropped"),
            (
                RequestCancelSituation::RuntimeDisconnect,
                "runtime_disconnect",
            ),
            (RequestCancelSituation::RouterShutdown, "router_shutdown"),
        ];

        assert_eq!(RequestCancelSituation::CONTRACT_H.len(), mappings.len());
        assert_eq!(RequestCancelReason::CONTRACT_H.len(), mappings.len());

        for (situation, expected_wire_reason) in mappings {
            let wire_reason = request_cancel_reason_for_situation(situation);
            assert_eq!(wire_reason.as_str(), expected_wire_reason);
            assert_eq!(
                RequestCancelReason::from_wire(expected_wire_reason),
                Some(wire_reason)
            );
        }
    }

    #[test]
    fn internal_reason_mapping_exposes_original_and_wire_reason() {
        let mapping = map_internal_request_cancel_reason("chunk_seq_mismatch");
        assert_eq!(mapping.internal_reason, "chunk_seq_mismatch");
        assert_eq!(mapping.wire_reason, RequestCancelReason::ProtocolError);

        let mapping = map_internal_request_cancel_reason("stream_cancelled");
        assert_eq!(mapping.internal_reason, "stream_cancelled");
        assert_eq!(mapping.wire_reason, RequestCancelReason::StreamDropped);

        let mapping = map_internal_request_cancel_reason("unknown_internal_reason");
        assert_eq!(mapping.internal_reason, "unknown_internal_reason");
        assert_eq!(mapping.wire_reason, RequestCancelReason::CallerCancel);
    }
}
