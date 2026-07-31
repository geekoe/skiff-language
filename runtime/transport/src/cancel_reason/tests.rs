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
