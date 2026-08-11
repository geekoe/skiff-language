use skiff_runtime_request::{BoundaryResponse, RequestCancel, RequestError, RouterWriterMessage};
use skiff_runtime_transport::response_mapper::OrdinaryResponseEvent;
use skiff_runtime_transport::{response_mapper, TransportError};
use tracing::info;

use crate::error::{Result, RuntimeError};

use super::RuntimeHost;

mod assembly;
mod assembly_wire;
#[cfg(test)]
mod bytecode_http_tests;
mod resumable;
mod websocket_jsonrpc;

impl RuntimeHost {
    pub(crate) async fn cancel_request(&self, cancel: RequestCancel) {
        if self.request_supervisor.cancel(&cancel).await {
            info!(
                event = "runtime.request_cancelled",
                request_id = %cancel.request_id,
                reason = cancel.reason.as_deref().unwrap_or("unknown")
            );
        }
    }
}

fn request_error_into_runtime_error(error: RequestError) -> RuntimeError {
    if error.is_cancellation_terminal() {
        RuntimeError::Cancelled
    } else {
        RuntimeError::Opaque(Box::new(
            skiff_runtime_request::OrdinaryRequestError::try_new(error)
                .expect("request cancellation was split before Host trait erasure"),
        ))
    }
}

pub(crate) fn transport_error_into_runtime_error(error: TransportError) -> RuntimeError {
    RuntimeError::Decode(error.to_string())
}

fn response_into_transport_message(
    request_id: String,
    response: BoundaryResponse,
) -> Result<Option<RouterWriterMessage>> {
    match response {
        BoundaryResponse::Event(event) => response_event_into_transport_message(
            request_id,
            OrdinaryResponseEvent::try_from_non_error(event)
                .map_err(transport_error_into_runtime_error)?,
        )
        .map(Some),
        BoundaryResponse::StreamSent => Ok(None),
    }
}

fn response_event_into_transport_message(
    request_id: String,
    event: OrdinaryResponseEvent,
) -> Result<RouterWriterMessage> {
    response_mapper::response_event_into_frame(request_id, event)
        .map(RouterWriterMessage::Binary)
        .map_err(transport_error_into_runtime_error)
}

#[cfg(test)]
mod tests;
