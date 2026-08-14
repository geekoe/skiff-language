//! Production bytecode server-stream frames over the Router WebSocket writer.

use std::{sync::Arc, time::Instant};

use skiff_runtime_capability_context::{ExecutionScopeTerminal, RouterWriteFailure};
use skiff_runtime_request::{
    BytecodeServerStreamFrame, BytecodeServerStreamWriteFailure, BytecodeServerStreamWriteFuture,
    BytecodeServerStreamWriterPort, HttpAdapterKind, HttpResponseMetadata, OwnedExecutionControl,
    ResponseStreamEvent, RouterWriterMessage,
};
use skiff_runtime_transport::response_mapper::response_stream_event_into_frame;
use tokio::sync::mpsc;

/// Transport-only writer bound to one exact Router request.
///
/// Sequence, capacity and terminal authority remain in the request scheduler.
/// This adapter only maps an owned, heap-free frame, enqueues its exact bytes,
/// and waits for the Router writer's unique WebSocket flush receipt.
struct ProductionBytecodeServerStreamWriter {
    request_id: String,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
}

impl ProductionBytecodeServerStreamWriter {
    fn new(request_id: String, sender: mpsc::UnboundedSender<RouterWriterMessage>) -> Self {
        Self { request_id, sender }
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

            let event = response_stream_event(frame);
            let bytes = response_stream_event_into_frame(&request_id, event).map_err(|error| {
                BytecodeServerStreamWriteFailure::InvalidProviderContract(error.to_string())
            })?;
            let (message, flushed) = RouterWriterMessage::stream_frame(bytes);
            sender
                .send(message)
                .map_err(|_| BytecodeServerStreamWriteFailure::RouterDisconnected)?;
            flushed.wait().await.map_err(map_router_write_failure)
        })
    }
}

pub(super) fn production_bytecode_server_stream_writer_for_entry(
    request_id: String,
    mode: &str,
    adapter_kind: Option<HttpAdapterKind>,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
) -> Option<Arc<dyn BytecodeServerStreamWriterPort>> {
    (mode == "serverStream" && adapter_kind == Some(HttpAdapterKind::RawHttp)).then(|| {
        Arc::new(ProductionBytecodeServerStreamWriter::new(
            request_id, sender,
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
