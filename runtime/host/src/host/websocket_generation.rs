use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::websocket_generation_lifecycle::{
    assert_websocket_generation_lifecycle_response_matches,
    decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
    WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleRejectionCode,
    WebSocketGenerationLifecycleSender, WebSocketGenerationLifecycleTuple,
    WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{
    error::{Result, RuntimeError},
    loader::assembly_admission::ActiveAssemblyRoute,
};

#[derive(Debug)]
struct WebSocketGenerationPin {
    tuple: WebSocketGenerationLifecycleTuple,
    route: ActiveAssemblyRoute,
}

#[derive(Debug)]
struct AcquireRecord {
    request: WebSocketGenerationLifecycleControl,
    connection_key: ConnectionKey,
    inserted_pin: bool,
}

#[derive(Debug)]
struct CachedRelease {
    router_session_id: String,
    request: WebSocketGenerationLifecycleControl,
    response: WebSocketGenerationLifecycleControl,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConnectionKey {
    router_session_id: String,
    connection_id: String,
}

#[derive(Debug, Default)]
struct WebSocketGenerationState {
    live_sessions: HashSet<String>,
    pins: HashMap<ConnectionKey, WebSocketGenerationPin>,
    acquires: HashMap<String, AcquireRecord>,
    releases: HashMap<String, CachedRelease>,
}

/// Runtime owner for WebSocket connection pins that outlive one request.
///
/// Each pin retains the immutable `ActiveAssemblyRoute` selected for connect, so a later receive
/// never consults the current assembly pointer and never performs artifact I/O.
#[derive(Debug, Default)]
pub(super) struct WebSocketGenerationRegistry {
    state: Mutex<WebSocketGenerationState>,
}

impl WebSocketGenerationRegistry {
    pub(super) fn connect(&self, router_session_id: &str) -> Result<()> {
        let mut state = self.lock_state()?;
        if !state.live_sessions.insert(router_session_id.to_string()) {
            return Err(RuntimeError::Decode(
                "WebSocket generation Router session is already connected".to_string(),
            ));
        }
        Ok(())
    }

    pub(super) fn begin_acquire(
        &self,
        router_session_id: &str,
        route: ActiveAssemblyRoute,
        websocket_entry_id: String,
        connection_id: String,
    ) -> Result<WebSocketGenerationLifecycleControl> {
        let tuple = WebSocketGenerationLifecycleTuple {
            router_session_id: router_session_id.to_string(),
            service_id: route.entry().owner().service_id.clone(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            websocket_entry_id,
            connection_id,
        };
        let connection_key = ConnectionKey {
            router_session_id: tuple.router_session_id.clone(),
            connection_id: tuple.connection_id.clone(),
        };
        let request_id = format!(
            "skiff-websocket-lifecycle-request-v1:opaque:{}",
            uuid::Uuid::new_v4()
        );
        let request = WebSocketGenerationLifecycleControl::Acquire {
            schema_version: skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION
                .to_string(),
            frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
            request_id: request_id.clone(),
            sender: WebSocketGenerationLifecycleSender::Runtime,
            tuple: tuple.clone(),
        };
        let mut state = self.lock_state()?;
        if !state.live_sessions.contains(router_session_id) {
            return Err(RuntimeError::Decode(
                "WebSocket generation acquire belongs to a disconnected Router session".to_string(),
            ));
        }
        let mut inserted_pin = false;
        if let Some(existing) = state.pins.get(&connection_key) {
            if existing.tuple != tuple {
                return Err(RuntimeError::Decode(
                    "WebSocket connection already pins a different assembly tuple".to_string(),
                ));
            }
        } else {
            state.pins.insert(
                connection_key.clone(),
                WebSocketGenerationPin { tuple, route },
            );
            inserted_pin = true;
        }
        state.acquires.insert(
            request_id,
            AcquireRecord {
                request: request.clone(),
                connection_key,
                inserted_pin,
            },
        );
        info!(
            event = "runtime.websocket_generation_acquire_queued",
            router_session_id = %request_tuple(&request).router_session_id,
            service_id = %request_tuple(&request).service_id,
            assembly_identity = %request_tuple(&request).assembly_identity,
            assembly_generation = request_tuple(&request).assembly_generation,
            websocket_entry_id = %request_tuple(&request).websocket_entry_id,
            connection_id = %request_tuple(&request).connection_id,
        );
        Ok(request)
    }

    pub(super) fn dispatch_router_control(
        &self,
        router_session_id: &str,
        bytes: &[u8],
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        let control = decode_websocket_generation_lifecycle_frame(
            WebSocketGenerationLifecycleDirection::RouterToRuntime,
            bytes,
        )
        .map_err(super::transport_error_into_runtime_error)?;
        match &control {
            WebSocketGenerationLifecycleControl::Release { .. } => {
                let reply = self.handle_release(router_session_id, control)?;
                let frame = encode_websocket_generation_lifecycle_frame(
                    WebSocketGenerationLifecycleDirection::RuntimeToRouter,
                    &reply,
                )
                .map_err(super::transport_error_into_runtime_error)?;
                sender
                    .send(RouterWriterMessage::Binary(frame))
                    .map_err(|_| {
                        RuntimeError::Decode(
                            "failed to queue WebSocket generation release response".to_string(),
                        )
                    })
            }
            WebSocketGenerationLifecycleControl::Ack {
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                ..
            }
            | WebSocketGenerationLifecycleControl::Reject {
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                ..
            } => self.handle_acquire_response(&control),
            _ => Err(RuntimeError::Decode(
                "Router sent unsupported WebSocket generation lifecycle control".to_string(),
            )),
        }
    }

    pub(super) fn rollback_acquire(
        &self,
        request: &WebSocketGenerationLifecycleControl,
    ) -> Result<()> {
        let Some((request_id, _)) = acquire_parts(request) else {
            return Err(RuntimeError::Decode(
                "WebSocket generation acquire rollback requires an acquire request".to_string(),
            ));
        };
        let mut state = self.lock_state()?;
        if let Some(record) = state.acquires.remove(request_id) {
            if record.inserted_pin {
                state.pins.remove(&record.connection_key);
            }
        }
        Ok(())
    }

    pub(super) fn handle_acquire_response(
        &self,
        response: &WebSocketGenerationLifecycleControl,
    ) -> Result<()> {
        let request_id = response_request_id(response).ok_or_else(|| {
            RuntimeError::Decode(
                "WebSocket generation acquire response must be an ack or rejection".to_string(),
            )
        })?;
        let mut state = self.lock_state()?;
        let record = state.acquires.get(request_id).ok_or_else(|| {
            RuntimeError::Decode(
                "WebSocket generation acquire response has no pending request".to_string(),
            )
        })?;
        assert_websocket_generation_lifecycle_response_matches(&record.request, response)
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        match response {
            WebSocketGenerationLifecycleControl::Ack {
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                ..
            } => {
                let tuple = request_tuple(&record.request);
                info!(
                    event = "runtime.websocket_generation_acquired",
                    router_session_id = %tuple.router_session_id,
                    service_id = %tuple.service_id,
                    assembly_identity = %tuple.assembly_identity,
                    assembly_generation = tuple.assembly_generation,
                    websocket_entry_id = %tuple.websocket_entry_id,
                    connection_id = %tuple.connection_id,
                );
                Ok(())
            }
            WebSocketGenerationLifecycleControl::Reject {
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                code,
                reason,
                ..
            } => {
                let connection_key = record.connection_key.clone();
                let inserted_pin = record.inserted_pin;
                state.acquires.remove(request_id);
                if inserted_pin {
                    state.pins.remove(&connection_key);
                }
                warn!(
                    event = "runtime.websocket_generation_acquire_rejected",
                    router_session_id = %connection_key.router_session_id,
                    connection_id = %connection_key.connection_id,
                    rejection_code = ?code,
                    rejection_reason = reason,
                );
                Ok(())
            }
            _ => Err(RuntimeError::Decode(
                "WebSocket generation response is not for acquire".to_string(),
            )),
        }
    }

    pub(super) fn pinned_route(
        &self,
        router_session_id: &str,
        connection_id: &str,
        assembly_identity: &skiff_artifact_model::AssemblyIdentity,
        assembly_generation: u64,
        websocket_entry_id: &str,
    ) -> Result<ActiveAssemblyRoute> {
        let state = self.lock_state()?;
        let key = ConnectionKey {
            router_session_id: router_session_id.to_string(),
            connection_id: connection_id.to_string(),
        };
        let pin = state.pins.get(&key).ok_or_else(|| {
            RuntimeError::Unsupported(
                "WebSocket receive has no acquired generation pin".to_string(),
            )
        })?;
        if pin.tuple.assembly_identity != *assembly_identity
            || pin.tuple.assembly_generation != assembly_generation
            || pin.tuple.websocket_entry_id != websocket_entry_id
        {
            return Err(RuntimeError::Protocol {
                target: connection_id.to_string(),
                message: "WebSocket receive tuple does not match its acquired generation pin"
                    .to_string(),
            });
        }
        Ok(pin.route.clone())
    }

    pub(super) fn handle_release(
        &self,
        router_session_id: &str,
        request: WebSocketGenerationLifecycleControl,
    ) -> Result<WebSocketGenerationLifecycleControl> {
        let (request_id, tuple) = release_parts(&request).ok_or_else(|| {
            RuntimeError::Decode(
                "Runtime WebSocket generation lifecycle accepts only release requests".to_string(),
            )
        })?;
        let mut state = self.lock_state()?;
        if let Some(cached) = state.releases.get(request_id) {
            if cached.router_session_id != router_session_id {
                return Ok(release_rejection(
                    request_id,
                    tuple,
                    WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
                    "release request id belongs to a different Router session",
                ));
            }
            if cached.request == request {
                return Ok(cached.response.clone());
            }
            return Ok(release_rejection(
                request_id,
                tuple,
                WebSocketGenerationLifecycleRejectionCode::RequestConflict,
                "release request id was reused",
            ));
        }
        let response = if tuple.router_session_id != router_session_id {
            release_rejection(
                request_id,
                tuple,
                WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
                "release router session does not match the connected Router session",
            )
        } else {
            let key = ConnectionKey {
                router_session_id: router_session_id.to_string(),
                connection_id: tuple.connection_id.clone(),
            };
            match state.pins.get(&key) {
                None => release_rejection(
                    request_id,
                    tuple,
                    WebSocketGenerationLifecycleRejectionCode::NotAcquired,
                    "connection has no acquired generation pin",
                ),
                Some(pin) if pin.tuple != *tuple => release_rejection(
                    request_id,
                    tuple,
                    WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
                    "release tuple does not match the acquired generation pin",
                ),
                Some(_) => {
                    state.pins.remove(&key);
                    state
                        .acquires
                        .retain(|_, record| record.connection_key != key);
                    info!(
                        event = "runtime.websocket_generation_released",
                        router_session_id = %tuple.router_session_id,
                        service_id = %tuple.service_id,
                        assembly_identity = %tuple.assembly_identity,
                        assembly_generation = tuple.assembly_generation,
                        websocket_entry_id = %tuple.websocket_entry_id,
                        connection_id = %tuple.connection_id,
                    );
                    release_ack(request_id, tuple)
                }
            }
        };
        state.releases.insert(
            request_id.to_string(),
            CachedRelease {
                router_session_id: router_session_id.to_string(),
                request,
                response: response.clone(),
            },
        );
        Ok(response)
    }

    pub(super) fn disconnect(&self, router_session_id: &str) -> Result<()> {
        let mut state = self.lock_state()?;
        state.live_sessions.remove(router_session_id);
        let released = state
            .pins
            .keys()
            .filter(|key| key.router_session_id == router_session_id)
            .count();
        state
            .pins
            .retain(|key, _| key.router_session_id != router_session_id);
        state
            .acquires
            .retain(|_, record| record.connection_key.router_session_id != router_session_id);
        state
            .releases
            .retain(|_, cached| cached.router_session_id != router_session_id);
        if released > 0 {
            info!(
                event = "runtime.websocket_generation_session_released",
                router_session_id,
                connection_count = released,
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn pin_count(&self) -> Result<usize> {
        Ok(self.lock_state()?.pins.len())
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, WebSocketGenerationState>> {
        self.state.lock().map_err(|_| {
            RuntimeError::Decode("WebSocket generation registry lock is poisoned".to_string())
        })
    }
}

fn request_tuple(
    control: &WebSocketGenerationLifecycleControl,
) -> &WebSocketGenerationLifecycleTuple {
    match control {
        WebSocketGenerationLifecycleControl::Acquire { tuple, .. } => tuple,
        _ => unreachable!("begin_acquire always builds acquire"),
    }
}

fn acquire_parts(
    control: &WebSocketGenerationLifecycleControl,
) -> Option<(&str, &WebSocketGenerationLifecycleTuple)> {
    match control {
        WebSocketGenerationLifecycleControl::Acquire {
            request_id, tuple, ..
        } => Some((request_id, tuple)),
        _ => None,
    }
}

fn release_parts(
    control: &WebSocketGenerationLifecycleControl,
) -> Option<(&str, &WebSocketGenerationLifecycleTuple)> {
    match control {
        WebSocketGenerationLifecycleControl::Release {
            request_id, tuple, ..
        } => Some((request_id, tuple)),
        _ => None,
    }
}

fn response_request_id(control: &WebSocketGenerationLifecycleControl) -> Option<&str> {
    match control {
        WebSocketGenerationLifecycleControl::Ack {
            operation: WebSocketGenerationLifecycleOperation::Acquire,
            request_id,
            ..
        }
        | WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Acquire,
            request_id,
            ..
        } => Some(request_id),
        _ => None,
    }
}

fn release_ack(
    request_id: &str,
    tuple: &WebSocketGenerationLifecycleTuple,
) -> WebSocketGenerationLifecycleControl {
    WebSocketGenerationLifecycleControl::Ack {
        schema_version: skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Release,
        request_id: request_id.to_string(),
        sender: WebSocketGenerationLifecycleSender::Runtime,
        tuple: tuple.clone(),
    }
}

fn release_rejection(
    request_id: &str,
    tuple: &WebSocketGenerationLifecycleTuple,
    code: WebSocketGenerationLifecycleRejectionCode,
    reason: &str,
) -> WebSocketGenerationLifecycleControl {
    WebSocketGenerationLifecycleControl::Reject {
        schema_version: skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Release,
        request_id: request_id.to_string(),
        sender: WebSocketGenerationLifecycleSender::Runtime,
        tuple: tuple.clone(),
        code,
        reason: reason.to_string(),
    }
}
