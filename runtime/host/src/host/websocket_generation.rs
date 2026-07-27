use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use skiff_artifact_model::{
    AssemblyIdentity, GatewayEntryIdentity, GatewayWebSocketRpcProfile, WebSocketEntryId,
};
use skiff_runtime_request::{RouterWriterMessage, RuntimeAssemblyWebSocketJsonRpcTarget};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    assert_websocket_generation_lifecycle_response_matches,
    decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
    WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleRejectionCode,
    WebSocketGenerationLifecycleSender, WebSocketGenerationLifecycleTuple,
    WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::{
    error::{Result, RuntimeError},
    loader::assembly_admission::ActiveAssemblyRoute,
};

#[derive(Debug)]
struct WebSocketGenerationPin {
    tuple: WebSocketGenerationLifecycleTuple,
    route: ActiveAssemblyRoute,
    acquired: bool,
}

#[derive(Debug)]
struct AcquireRecord {
    request: WebSocketGenerationLifecycleControl,
    connection_key: ConnectionKey,
    inserted_pin: bool,
    receipt: Option<oneshot::Sender<std::result::Result<(), String>>>,
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
/// Each pin retains the immutable physical `ActiveAssemblyRoute` selected for connect, so a later
/// JSON-RPC method lookup never consults the current assembly pointer or performs artifact I/O.
#[derive(Debug, Default)]
pub(super) struct WebSocketGenerationRegistry {
    state: Mutex<WebSocketGenerationState>,
}

pub(super) struct WebSocketGenerationAcquireReceipt {
    receiver: oneshot::Receiver<std::result::Result<(), String>>,
}

#[derive(Debug)]
#[allow(dead_code)] // Consumed by the downstream Host dispatch leaf after this pin-owner checkpoint.
pub(super) struct ResolvedWebSocketJsonRpcExecution {
    pub(super) target: RuntimeAssemblyWebSocketJsonRpcTarget,
    pub(super) method_route: ActiveAssemblyRoute,
}

impl WebSocketGenerationAcquireReceipt {
    pub(super) async fn wait(self) -> Result<()> {
        match self.receiver.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(message)) => Err(RuntimeError::Decode(message)),
            Err(_) => Err(RuntimeError::Decode(
                "WebSocket generation acquire receipt owner was dropped".to_string(),
            )),
        }
    }
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
        self.begin_acquire_inner(
            router_session_id,
            route,
            websocket_entry_id,
            connection_id,
            None,
        )
    }

    pub(super) fn begin_acquire_with_receipt(
        &self,
        router_session_id: &str,
        route: ActiveAssemblyRoute,
        websocket_entry_id: String,
        connection_id: String,
    ) -> Result<(
        WebSocketGenerationLifecycleControl,
        WebSocketGenerationAcquireReceipt,
    )> {
        let (sender, receiver) = oneshot::channel();
        let request = self.begin_acquire_inner(
            router_session_id,
            route,
            websocket_entry_id,
            connection_id,
            Some(sender),
        )?;
        Ok((request, WebSocketGenerationAcquireReceipt { receiver }))
    }

    fn begin_acquire_inner(
        &self,
        router_session_id: &str,
        route: ActiveAssemblyRoute,
        websocket_entry_id: String,
        connection_id: String,
        receipt: Option<oneshot::Sender<std::result::Result<(), String>>>,
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
                WebSocketGenerationPin {
                    tuple,
                    route,
                    acquired: false,
                },
            );
            inserted_pin = true;
        }
        state.acquires.insert(
            request_id,
            AcquireRecord {
                request: request.clone(),
                connection_key,
                inserted_pin,
                receipt,
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
            settle_receipt(
                record.receipt,
                Err("WebSocket generation acquire was rolled back".to_string()),
            );
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
        if let Err(error) =
            assert_websocket_generation_lifecycle_response_matches(&record.request, response)
        {
            let message = format!("WebSocket generation acquire receipt mismatch: {error}");
            let record = state
                .acquires
                .remove(request_id)
                .expect("correlated acquire record remains present");
            if record.inserted_pin {
                state.pins.remove(&record.connection_key);
            }
            settle_receipt(record.receipt, Err(message.clone()));
            return Err(RuntimeError::Decode(message));
        }
        let record = state
            .acquires
            .remove(request_id)
            .expect("validated acquire record remains present");
        match response {
            WebSocketGenerationLifecycleControl::Ack {
                operation: WebSocketGenerationLifecycleOperation::Acquire,
                ..
            } => {
                let tuple = request_tuple(&record.request);
                let pin_matches = state
                    .pins
                    .get(&record.connection_key)
                    .is_some_and(|pin| pin.tuple == *tuple);
                if !pin_matches {
                    if record.inserted_pin {
                        state.pins.remove(&record.connection_key);
                    }
                    settle_receipt(
                        record.receipt,
                        Err(
                            "WebSocket generation acquire receipt changed its tentative pin"
                                .to_string(),
                        ),
                    );
                    return Err(RuntimeError::Decode(
                        "WebSocket generation acquire receipt changed its tentative pin"
                            .to_string(),
                    ));
                }
                state
                    .pins
                    .get_mut(&record.connection_key)
                    .expect("matching tentative pin remains present")
                    .acquired = true;
                settle_receipt(record.receipt, Ok(()));
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
                if inserted_pin {
                    state.pins.remove(&connection_key);
                }
                settle_receipt(
                    record.receipt,
                    Err(format!(
                        "WebSocket generation acquire was rejected ({code:?}): {reason}"
                    )),
                );
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

    fn acquired_physical_route(
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
                "WebSocket JSON-RPC request has no acquired generation pin".to_string(),
            )
        })?;
        if !pin.acquired {
            return Err(RuntimeError::Unsupported(
                "WebSocket JSON-RPC generation pin has no exact acquire receipt".to_string(),
            ));
        }
        if pin.tuple.assembly_identity != *assembly_identity
            || pin.tuple.assembly_generation != assembly_generation
            || pin.tuple.websocket_entry_id != websocket_entry_id
            || pin.tuple.service_id != pin.route.entry().owner().service_id
        {
            return Err(RuntimeError::Protocol {
                target: connection_id.to_string(),
                message: "WebSocket JSON-RPC tuple does not match its acquired generation pin"
                    .to_string(),
            });
        }
        Ok(pin.route.clone())
    }

    #[allow(dead_code, clippy::too_many_arguments)] // Downstream Host dispatch consumes this seam.
    pub(super) fn websocket_jsonrpc_execution_route(
        &self,
        router_session_id: &str,
        connection_id: &str,
        assembly_identity: &AssemblyIdentity,
        assembly_generation: u64,
        websocket_entry_id: &WebSocketEntryId,
        host: &str,
        path: &str,
        method: &str,
        gateway_entry_identity: &GatewayEntryIdentity,
        profile: GatewayWebSocketRpcProfile,
    ) -> Result<ResolvedWebSocketJsonRpcExecution> {
        let physical_route = self.acquired_physical_route(
            router_session_id,
            connection_id,
            assembly_identity,
            assembly_generation,
            websocket_entry_id.as_str(),
        )?;
        let method_route = physical_route
            .websocket_jsonrpc_method_route(
                host,
                path,
                method,
                gateway_entry_identity,
                profile,
                websocket_entry_id,
            )
            .map_err(|error| RuntimeError::Protocol {
                target: connection_id.to_string(),
                message: error.to_string(),
            })?;
        let target = method_route
            .websocket_jsonrpc_target(&physical_route)
            .map_err(|error| RuntimeError::Protocol {
                target: connection_id.to_string(),
                message: error.to_string(),
            })?;
        Ok(ResolvedWebSocketJsonRpcExecution {
            target,
            method_route,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn websocket_jsonrpc_target(
        &self,
        router_session_id: &str,
        connection_id: &str,
        assembly_identity: &AssemblyIdentity,
        assembly_generation: u64,
        websocket_entry_id: &WebSocketEntryId,
        host: &str,
        path: &str,
        method: &str,
        gateway_entry_identity: &GatewayEntryIdentity,
        profile: GatewayWebSocketRpcProfile,
    ) -> Result<RuntimeAssemblyWebSocketJsonRpcTarget> {
        Ok(self
            .websocket_jsonrpc_execution_route(
                router_session_id,
                connection_id,
                assembly_identity,
                assembly_generation,
                websocket_entry_id,
                host,
                path,
                method,
                gateway_entry_identity,
                profile,
            )?
            .target)
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
                    let pending = take_connection_acquires(&mut state.acquires, &key);
                    for record in pending {
                        settle_receipt(
                            record.receipt,
                            Err("WebSocket generation was released before acquire completed"
                                .to_string()),
                        );
                    }
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
        let disconnected_keys = state
            .acquires
            .iter()
            .filter(|(_, record)| record.connection_key.router_session_id == router_session_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let disconnected_acquires = disconnected_keys
            .into_iter()
            .filter_map(|request_id| state.acquires.remove(&request_id))
            .collect::<Vec<_>>();
        for record in disconnected_acquires {
            settle_receipt(
                record.receipt,
                Err("WebSocket generation Router session disconnected".to_string()),
            );
        }
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

fn settle_receipt(
    receipt: Option<oneshot::Sender<std::result::Result<(), String>>>,
    result: std::result::Result<(), String>,
) {
    if let Some(receipt) = receipt {
        let _ = receipt.send(result);
    }
}

fn take_connection_acquires(
    acquires: &mut HashMap<String, AcquireRecord>,
    connection_key: &ConnectionKey,
) -> Vec<AcquireRecord> {
    let request_ids = acquires
        .iter()
        .filter(|(_, record)| &record.connection_key == connection_key)
        .map(|(request_id, _)| request_id.clone())
        .collect::<Vec<_>>();
    request_ids
        .into_iter()
        .filter_map(|request_id| acquires.remove(&request_id))
        .collect()
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
