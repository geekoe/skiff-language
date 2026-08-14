use std::{
    sync::Arc,
    task::Poll,
    time::{Duration, Instant},
};

use futures_util::future::poll_fn;
use skiff_runtime_capability_context::{CancellationToken, RouterWriteFailure};
use skiff_runtime_request::{
    execution_budget::AdmittedRequestDeadline, BytecodeServerStreamFrame,
    BytecodeServerStreamWriteFailure, BytecodeServerStreamWriterPort, ExecutionBudget,
    ExecutionControl, HttpNameValue, RouterWriterMessage,
};
use skiff_runtime_transport::protocol::{
    decode_response_chunk_frame, decode_response_end_frame, decode_response_start_frame,
    ResponseEndFrameMetadata,
};
use tokio::sync::{mpsc, oneshot};

use super::*;

fn execution_control() -> (CancellationToken, OwnedExecutionControl) {
    let cancellation = CancellationToken::new();
    let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
    let control = ExecutionControl::new(cancellation.clone(), &budget).owned();
    (cancellation, control)
}

fn expired_execution_control() -> OwnedExecutionControl {
    let budget = Arc::new(ExecutionBudget::for_runtime_request(Some(
        AdmittedRequestDeadline::new(Instant::now() - Duration::from_secs(1)),
    )));
    ExecutionControl::new(CancellationToken::new(), &budget).owned()
}

fn writer() -> (
    ProductionBytecodeServerStreamWriter,
    mpsc::UnboundedReceiver<RouterWriterMessage>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (
        ProductionBytecodeServerStreamWriter::new("request-stream-42".to_string(), sender),
        receiver,
    )
}

async fn next_stream_frame(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
) -> (Vec<u8>, oneshot::Sender<Result<(), RouterWriteFailure>>) {
    let message = receiver.recv().await.expect("writer enqueues one frame");
    let RouterWriterMessage::StreamFrame { bytes, flush_ack } = message else {
        panic!("server-stream writer used a non-flush-aware Router message");
    };
    (bytes, flush_ack)
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_maps_start_chunk_end_in_order() {
    let (writer, mut receiver) = writer();
    let (_, execution) = execution_control();
    let frames = [
        BytecodeServerStreamFrame::Start {
            status: 207,
            headers: vec![HttpNameValue {
                name: "content-type".to_string(),
                value: "application/octet-stream".to_string(),
            }],
        },
        BytecodeServerStreamFrame::Chunk {
            sequence: 41,
            payload: b"first".to_vec(),
        },
        BytecodeServerStreamFrame::End,
    ];

    for (ordinal, frame) in frames.into_iter().enumerate() {
        let flush = tokio::spawn(writer.flush(frame, execution.clone()));
        let (bytes, flush_ack) = next_stream_frame(&mut receiver).await;
        assert!(
            !flush.is_finished(),
            "frame {ordinal} must remain pending until the real flush receipt"
        );
        match ordinal {
            0 => {
                let header = decode_response_start_frame(&bytes).expect("response.start frame");
                assert_eq!(header.request_id, "request-stream-42");
                assert_eq!(header.http_response.status, 207);
                assert_eq!(header.http_response.headers.len(), 1);
                assert_eq!(header.http_response.headers[0].name, "content-type");
                assert_eq!(
                    header.http_response.headers[0].value,
                    "application/octet-stream"
                );
            }
            1 => {
                let (header, payload) =
                    decode_response_chunk_frame(&bytes).expect("response.chunk frame");
                assert_eq!(header.request_id, "request-stream-42");
                assert_eq!(header.seq, 41, "writer must preserve scheduler sequence");
                assert_eq!(payload, b"first");
            }
            2 => {
                let (header, payload) =
                    decode_response_end_frame(&bytes).expect("response.end frame");
                assert_eq!(header.request_id, "request-stream-42");
                assert!(!header.payload_present);
                assert_eq!(header.metadata, ResponseEndFrameMetadata::None);
                assert!(payload.is_empty());
            }
            _ => unreachable!(),
        }
        flush_ack.send(Ok(())).expect("flush waiter remains live");
        flush
            .await
            .expect("writer task joins")
            .expect("flush succeeds");
    }
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_backpressures_until_flush_ack() {
    let (writer, mut receiver) = writer();
    let (_, execution) = execution_control();
    let mut flush = writer.flush(BytecodeServerStreamFrame::End, execution);

    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    poll_fn(|context| {
        assert!(flush.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    let (_, flush_ack) = next_stream_frame(&mut receiver).await;
    poll_fn(|context| {
        assert!(flush.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;

    flush_ack.send(Ok(())).expect("flush waiter remains live");
    flush.await.expect("acknowledged flush completes");
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_maps_websocket_flush_failure() {
    let (writer, mut receiver) = writer();
    let (_, execution) = execution_control();
    let flush = tokio::spawn(writer.flush(BytecodeServerStreamFrame::End, execution));
    let (_, flush_ack) = next_stream_frame(&mut receiver).await;
    flush_ack
        .send(Err(RouterWriteFailure::WebSocketWrite {
            message: "injected WebSocket failure".to_string(),
        }))
        .expect("flush waiter remains live");

    assert_eq!(
        flush.await.expect("writer task joins"),
        Err(BytecodeServerStreamWriteFailure::WriterFailed(
            "injected WebSocket failure".to_string()
        ))
    );
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_completes_when_session_drops_frame() {
    let (writer, mut receiver) = writer();
    let (_, execution) = execution_control();
    let flush = tokio::spawn(writer.flush(BytecodeServerStreamFrame::End, execution));
    let message = receiver.recv().await.expect("writer enqueues one frame");
    drop(message);

    assert_eq!(
        flush.await.expect("writer task joins"),
        Err(BytecodeServerStreamWriteFailure::RouterDisconnected)
    );
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_completes_when_router_sender_is_closed() {
    let (writer, receiver) = writer();
    drop(receiver);
    let (_, execution) = execution_control();

    assert_eq!(
        writer
            .flush(BytecodeServerStreamFrame::End, execution)
            .await,
        Err(BytecodeServerStreamWriteFailure::RouterDisconnected)
    );
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_awaits_late_ack_after_cancellation() {
    let (writer, mut receiver) = writer();
    let (cancellation, execution) = execution_control();
    let flush = tokio::spawn(writer.flush(BytecodeServerStreamFrame::End, execution));
    let (_, flush_ack) = next_stream_frame(&mut receiver).await;
    cancellation.cancel();
    tokio::task::yield_now().await;
    assert!(
        !flush.is_finished(),
        "an enqueued frame keeps the unique receipt as completion authority"
    );

    flush_ack.send(Ok(())).expect("flush waiter remains live");
    flush
        .await
        .expect("writer task joins")
        .expect("late ack wins");
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_cancel_fails_before_enqueue() {
    let (writer, mut receiver) = writer();
    let (cancellation, execution) = execution_control();
    cancellation.cancel();

    assert_eq!(
        writer
            .flush(BytecodeServerStreamFrame::End, execution)
            .await,
        Err(BytecodeServerStreamWriteFailure::Cancelled)
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn phase_5_bytecode_http_server_stream_deadline_fails_before_enqueue() {
    let (writer, mut receiver) = writer();

    assert_eq!(
        writer
            .flush(BytecodeServerStreamFrame::End, expired_execution_control())
            .await,
        Err(BytecodeServerStreamWriteFailure::DeadlineExceeded)
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn phase_5_bytecode_http_writer_is_injected_only_for_exact_raw_http_server_stream() {
    for (mode, adapter_kind, expected) in [
        ("serverStream", Some(HttpAdapterKind::RawHttp), true),
        ("unary", Some(HttpAdapterKind::RawHttp), false),
        ("serverStream", Some(HttpAdapterKind::TypedJson), false),
        ("serverStream", None, false),
    ] {
        let (sender, _receiver) = mpsc::unbounded_channel();
        let writer = production_bytecode_server_stream_writer_for_entry(
            "request-selection".to_string(),
            mode,
            adapter_kind,
            sender,
        );
        assert_eq!(
            writer.is_some(),
            expected,
            "unexpected writer selection for {mode}/{adapter_kind:?}"
        );
    }
}
