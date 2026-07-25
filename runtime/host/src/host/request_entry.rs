use skiff_artifact_model::IngressSelector;
use skiff_runtime_request::{
    BoundaryResponse, RequestCancel, RequestEnvelope, RequestError, ResponseEvent,
    RouterWriterMessage,
};
use skiff_runtime_transport::{response_mapper, TransportError};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    loader::assembly_admission::ActiveAssemblyRoute,
};

use super::RuntimeHost;

mod assembly;
mod assembly_wire;
mod websocket_generation;

impl RuntimeHost {
    pub(super) fn send_request_error_response(
        &self,
        request: &RequestEnvelope,
        error: &RuntimeError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        match response_event_into_transport_message(
            request.request_id.clone(),
            ResponseEvent::Error(response_error_from_runtime_error(error)),
        ) {
            Ok(message) => {
                let _ = sender.send(message);
            }
            Err(error) => {
                error!(event = "runtime.response_encode_error", error = %error);
            }
        }
    }

    /// Resolves a canonical ingress only from one immutable active assembly generation.
    ///
    /// The canonical wire bridge supplies the selector. This entry performs no artifact access or
    /// candidate mutation and returns the activation template plus descriptor pinned by
    /// `ActiveAssemblyRoute`.
    pub(crate) fn lookup_active_assembly_request_route(
        &self,
        selector: &IngressSelector,
    ) -> Result<ActiveAssemblyRoute> {
        let route = self
            .active_runtime_assembly_route(selector)
            .map_err(|error| RuntimeError::Decode(error.to_string()))?
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "no active assembly ingress matches {selector:?}"
                ))
            })?;
        Ok(route)
    }

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
    RuntimeError::Opaque(Box::new(error))
}

pub(crate) fn transport_error_into_runtime_error(error: TransportError) -> RuntimeError {
    RuntimeError::Decode(error.to_string())
}

fn response_into_transport_message(
    request_id: String,
    response: BoundaryResponse,
) -> Result<Option<RouterWriterMessage>> {
    match response {
        BoundaryResponse::Event(event) => {
            response_event_into_transport_message(request_id, event).map(Some)
        }
        BoundaryResponse::StreamSent => Ok(None),
    }
}

fn response_event_into_transport_message(
    request_id: String,
    event: ResponseEvent,
) -> Result<RouterWriterMessage> {
    response_mapper::response_event_into_frame(request_id, event)
        .map(RouterWriterMessage::Binary)
        .map_err(transport_error_into_runtime_error)
}

#[cfg(test)]
mod tests {
    use skiff_runtime_capability_context::ExecutionBudgetReason;
    use skiff_runtime_model::service_error::PlatformBuiltinErrorIdentity;

    use crate::error::{RuntimeError, WirePayload};

    use super::*;

    #[test]
    fn request_error_bridge_boxes_and_delegates_payload_and_catch_projection() {
        let request_error = RequestError::protocol("svc.account", "bad frame");
        let expected_payload = request_error.payload();
        let expected_catch_projection = request_error.catch_projection();

        let error = request_error_into_runtime_error(request_error);

        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert_eq!(error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_catch_projection
        );
        assert_eq!(
            WirePayload::catch_projection(&error),
            Some((
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                serde_json::json!({
                    "target": "svc.account",
                    "message": "bad frame",
                })
            ))
        );
    }

    #[test]
    fn request_error_bridge_preserves_carried_cancellation_detection() {
        let error = request_error_into_runtime_error(RequestError::Cancelled);
        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert!(error.is_request_cancelled());

        let error = request_error_into_runtime_error(RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        });
        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert!(error.is_request_cancelled());
    }
}
