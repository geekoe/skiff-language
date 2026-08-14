//! Production bytecode server-stream frames over the Router WebSocket writer.

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use skiff_runtime_capability_context::{
    ExecutionScopeTerminal, RouterStreamFrameFlush, RouterWriteFailure,
};
use skiff_runtime_request::{
    BytecodeServerStreamFrame, BytecodeServerStreamWriteFailure, BytecodeServerStreamWriteFuture,
    BytecodeServerStreamWriterPort, HttpAdapterKind, HttpResponseMetadata, OwnedExecutionControl,
    ResponseStreamEvent, RouterWriterMessage,
};
use skiff_runtime_transport::response_mapper::response_stream_event_into_frame;
use tokio::sync::mpsc;

/// Transport-only writer bound to one exact Router request.
///
/// Sequence, capacity and stream-resource termination remain in the request
/// scheduler. This adapter maps an owned, heap-free frame, shares only the
/// request's wire-terminal arbiter with the ordinary Host response sink,
/// enqueues exact bytes and waits for the unique WebSocket flush receipt.
struct ProductionBytecodeServerStreamWriter {
    request_id: String,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    terminal: Arc<HttpGatewayTerminalArbiter>,
}

impl ProductionBytecodeServerStreamWriter {
    fn new(
        request_id: String,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        terminal: Arc<HttpGatewayTerminalArbiter>,
    ) -> Self {
        Self {
            request_id,
            sender,
            terminal,
        }
    }
}

/// Per-request wire-terminal state shared by the stream writer and the
/// ordinary HTTP response sink.
///
/// The scheduler still owns stream sequence, capacity and resource
/// termination. This arbiter owns only the external fact that the Host may
/// enqueue at most one terminal Router response for the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HttpGatewayTerminalState {
    Available,
    EndAcceptedOrInFlight,
    EndCommitted,
    OrdinaryTerminal,
}

pub(super) struct HttpGatewayTerminalArbiter {
    state: Mutex<HttpGatewayTerminalState>,
}

impl HttpGatewayTerminalArbiter {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(HttpGatewayTerminalState::Available),
        }
    }

    /// Atomically enqueue `response.end` and transfer the wire-terminal claim.
    /// A closed queue leaves the claim available because no write became
    /// possible. Once enqueue succeeds, ACK failure cannot reopen the claim:
    /// the peer may already have observed the frame.
    fn enqueue_end(
        &self,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        bytes: Vec<u8>,
    ) -> Result<RouterStreamFrameFlush, BytecodeServerStreamWriteFailure> {
        let mut state = self.lock_state();
        if *state != HttpGatewayTerminalState::Available {
            return Err(BytecodeServerStreamWriteFailure::InvalidProviderContract(
                "HTTP gateway response terminal is already settled".to_string(),
            ));
        }
        let (message, flushed) = RouterWriterMessage::stream_frame(bytes);
        sender
            .send(message)
            .map_err(|_| BytecodeServerStreamWriteFailure::RouterDisconnected)?;
        *state = HttpGatewayTerminalState::EndAcceptedOrInFlight;
        Ok(flushed)
    }

    fn commit_end(&self) {
        let mut state = self.lock_state();
        if *state == HttpGatewayTerminalState::EndAcceptedOrInFlight {
            *state = HttpGatewayTerminalState::EndCommitted;
        }
    }

    /// Enqueues an ordinary `response.end`/`response.error` only while no
    /// stream End owns the request terminal. The state changes only after the
    /// queue accepts the exact message.
    pub(super) fn enqueue_ordinary_terminal(
        &self,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        message: RouterWriterMessage,
    ) -> bool {
        let mut state = self.lock_state();
        if *state != HttpGatewayTerminalState::Available || sender.send(message).is_err() {
            return false;
        }
        *state = HttpGatewayTerminalState::OrdinaryTerminal;
        true
    }

    /// Settles a request that deliberately has no wire response. An already
    /// accepted End remains the terminal owner.
    pub(super) fn settle_without_response(&self) {
        let mut state = self.lock_state();
        if *state == HttpGatewayTerminalState::Available {
            *state = HttpGatewayTerminalState::OrdinaryTerminal;
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, HttpGatewayTerminalState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn state(&self) -> HttpGatewayTerminalState {
        *self.lock_state()
    }
}

impl BytecodeServerStreamWriterPort for ProductionBytecodeServerStreamWriter {
    fn flush(
        &self,
        frame: BytecodeServerStreamFrame,
        execution: OwnedExecutionControl,
    ) -> BytecodeServerStreamWriteFuture {
        let request_id = self.request_id.clone();
        let sender = self.sender.clone();
        let terminal = Arc::clone(&self.terminal);
        Box::pin(async move {
            // The async body begins only on the future's first poll. Once the
            // message is enqueued, the sole completion authority is the real
            // Router flush receipt; do not race it against a second cancel or
            // deadline observer.
            if let Some(terminal) = execution.scope_terminal_at(Instant::now()) {
                return Err(match terminal {
                    ExecutionScopeTerminal::AncestorCancelled => {
                        BytecodeServerStreamWriteFailure::Cancelled
                    }
                    ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                    | ExecutionScopeTerminal::InheritedDeadlineExceeded(_) => {
                        BytecodeServerStreamWriteFailure::DeadlineExceeded
                    }
                });
            }

            let is_end = matches!(&frame, BytecodeServerStreamFrame::End);
            let event = response_stream_event(frame);
            let bytes = response_stream_event_into_frame(&request_id, event).map_err(|error| {
                BytecodeServerStreamWriteFailure::InvalidProviderContract(error.to_string())
            })?;
            let flushed = if is_end {
                terminal.enqueue_end(&sender, bytes)?
            } else {
                let (message, flushed) = RouterWriterMessage::stream_frame(bytes);
                sender
                    .send(message)
                    .map_err(|_| BytecodeServerStreamWriteFailure::RouterDisconnected)?;
                flushed
            };
            let result = flushed.wait().await.map_err(map_router_write_failure);
            if is_end && result.is_ok() {
                terminal.commit_end();
            }
            result
        })
    }
}

pub(super) fn production_bytecode_server_stream_writer_for_entry(
    request_id: String,
    mode: &str,
    adapter_kind: Option<HttpAdapterKind>,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    terminal: Arc<HttpGatewayTerminalArbiter>,
) -> Option<Arc<dyn BytecodeServerStreamWriterPort>> {
    (mode == "serverStream" && adapter_kind == Some(HttpAdapterKind::RawHttp)).then(|| {
        Arc::new(ProductionBytecodeServerStreamWriter::new(
            request_id, sender, terminal,
        )) as Arc<dyn BytecodeServerStreamWriterPort>
    })
}

fn response_stream_event(frame: BytecodeServerStreamFrame) -> ResponseStreamEvent {
    match frame {
        BytecodeServerStreamFrame::Start { status, headers } => ResponseStreamEvent::Start {
            http_response: HttpResponseMetadata::new(status, headers),
        },
        BytecodeServerStreamFrame::Chunk { sequence, payload } => ResponseStreamEvent::Chunk {
            seq: sequence,
            payload,
        },
        BytecodeServerStreamFrame::End => ResponseStreamEvent::End,
    }
}

fn map_router_write_failure(error: RouterWriteFailure) -> BytecodeServerStreamWriteFailure {
    match error {
        RouterWriteFailure::SessionClosed => BytecodeServerStreamWriteFailure::RouterDisconnected,
        RouterWriteFailure::WebSocketWrite { message } => {
            BytecodeServerStreamWriteFailure::WriterFailed(message)
        }
    }
}

#[cfg(test)]
mod tests;
