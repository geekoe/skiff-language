use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use futures_util::{future::poll_fn, Sink};
use skiff_runtime_capability_context::RouterWriteFailure;
use skiff_runtime_request::RouterWriterMessage;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};

use super::super::*;

#[derive(Default)]
struct WriterState {
    accepted: Vec<Message>,
    flush_open: bool,
    flush_waker: Option<Waker>,
    fail_flush: bool,
}

#[derive(Clone, Default)]
struct WriterControl(Arc<Mutex<WriterState>>);

impl WriterControl {
    fn open_flush(&self) {
        let waker = {
            let mut state = self.0.lock().expect("writer state lock");
            state.flush_open = true;
            state.flush_waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn accepted(&self) -> Vec<Message> {
        self.0.lock().expect("writer state lock").accepted.clone()
    }
}

struct ControlledWriter {
    control: WriterControl,
}

impl ControlledWriter {
    fn gated() -> (Self, WriterControl) {
        let control = WriterControl::default();
        (
            Self {
                control: control.clone(),
            },
            control,
        )
    }

    fn failing() -> (Self, WriterControl) {
        let control = WriterControl::default();
        {
            let mut state = control.0.lock().expect("writer state lock");
            state.flush_open = true;
            state.fail_flush = true;
        }
        (
            Self {
                control: control.clone(),
            },
            control,
        )
    }
}

impl Sink<Message> for ControlledWriter {
    type Error = WebSocketError;

    fn poll_ready(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, message: Message) -> std::result::Result<(), Self::Error> {
        let mut state = self.control.0.lock().expect("writer state lock");
        state.accepted.push(message);
        Ok(())
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        let mut state = self.control.0.lock().expect("writer state lock");
        if !state.flush_open {
            state.flush_waker = Some(context.waker().clone());
            return Poll::Pending;
        }
        if state.fail_flush {
            Poll::Ready(Err(WebSocketError::ConnectionClosed))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        self.poll_flush(context)
    }
}

#[tokio::test]
async fn stream_frame_ack_waits_for_websocket_flush_and_preserves_exact_bytes() {
    let expected = vec![0, 1, 2, 3, 255];
    let (message, flushed) = RouterWriterMessage::stream_frame(expected.clone());
    let (mut writer, control) = ControlledWriter::gated();
    let send = send_writer_message(&mut writer, message);
    tokio::pin!(send);
    let flush = flushed.wait();
    tokio::pin!(flush);

    poll_fn(|context| {
        assert!(send.as_mut().poll(context).is_pending());
        assert!(flush.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(
        control.accepted(),
        vec![Message::Binary(expected.into())],
        "the writer must receive the exact pre-encoded stream frame"
    );

    control.open_flush();
    send.await.expect("websocket flush succeeds");
    flush.await.expect("flush acknowledgement is successful");
}

#[tokio::test]
async fn stream_frame_cannot_bypass_ack_through_generic_encoder() {
    let (message, flushed) = RouterWriterMessage::stream_frame(vec![4, 5, 6]);

    let error = encode_writer_message(message)
        .expect_err("stream frames require the flush-aware writer path");
    assert!(error.to_string().contains("flush-aware"));
    assert_eq!(
        flushed
            .wait()
            .await
            .expect_err("rejected generic encoding drops the unique message"),
        RouterWriteFailure::SessionClosed
    );
}

#[tokio::test]
async fn stream_frame_websocket_failure_is_acknowledged_as_error() {
    let (message, flushed) = RouterWriterMessage::stream_frame(vec![7, 8, 9]);
    let (mut writer, _control) = ControlledWriter::failing();

    let send_error = send_writer_message(&mut writer, message)
        .await
        .expect_err("failed websocket flush must fail the writer");
    assert!(send_error.to_string().contains("router write failed"));
    let ack_error = flushed
        .wait()
        .await
        .expect_err("failed websocket flush must not acknowledge success");
    assert!(matches!(
        ack_error,
        RouterWriteFailure::WebSocketWrite { ref message }
            if message.contains("router write failed")
    ));
}

#[tokio::test]
async fn dropping_queued_stream_frame_closes_ack_as_error() {
    let (message, flushed) = RouterWriterMessage::stream_frame(vec![1]);
    drop(message);

    assert_eq!(
        flushed
            .wait()
            .await
            .expect_err("dropping the unique message must close its acknowledgement"),
        RouterWriteFailure::SessionClosed
    );
}
