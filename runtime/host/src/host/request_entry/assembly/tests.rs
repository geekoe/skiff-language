use super::*;

#[test]
fn websocket_connect_accept_preserves_admission_rank_through_host_wire_encoding() {
    let message = websocket_connect_result_into_message(
        "ranked-websocket-connect".to_string(),
        RuntimeWebSocketConnectResult::Accept {
            business_identity: Some("ranked-business".to_string()),
            connection_policy: None,
            admission_rank: Some(42),
        },
    )
    .expect("ranked websocket connect result should encode");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("websocket connect result must use the binary transport")
    };
    let decoded = skiff_runtime_transport::runtime_assembly_request::decode_runtime_assembly_websocket_connect_response_end_frame(
        &frame,
    )
    .expect("Host websocket connect response should decode from the canonical wire");

    assert_eq!(decoded.request_id, "ranked-websocket-connect");
    assert_eq!(
        decoded.websocket_connect,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
            business_identity: Some("ranked-business".to_string()),
            connection_policy: None,
            admission_rank: Some(42),
        }
    );
}

#[test]
fn stream_terminal_waits_for_root_owner_and_cancel_discards_it() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let sink = HostHttpGatewayResponseSink::new(sender, 1024);

    sink.send_stream_event("request-stream-race", ResponseStreamEvent::End)
        .expect("stream producer may select its end");
    assert!(
        receiver.try_recv().is_err(),
        "stream end must remain private until root grants success ownership"
    );

    sink.cancel_without_response();
    sink.send_pending_stream_terminal();
    assert!(
        receiver.try_recv().is_err(),
        "root cancellation must discard the losing stream terminal"
    );
}

#[test]
fn stream_terminal_flushes_after_root_success_owner() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let sink = HostHttpGatewayResponseSink::new(sender, 1024);

    sink.send_stream_event("request-stream-success", ResponseStreamEvent::End)
        .expect("stream producer may select its end");
    sink.send_pending_stream_terminal();

    let message = receiver
        .try_recv()
        .expect("root success owner must flush the pending stream terminal");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("stream terminal must use the binary transport")
    };
    let (header, payload): (
        skiff_runtime_transport::protocol::ResponseEndFrameHeader,
        Vec<u8>,
    ) = skiff_runtime_transport::protocol::decode_typed_binary_frame(&frame)
        .expect("pending stream terminal must remain canonical");
    assert_eq!(header.request_id, "request-stream-success");
    assert!(payload.is_empty());
    assert!(receiver.try_recv().is_err());
}
