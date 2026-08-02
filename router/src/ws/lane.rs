//! `WebSocketLane`: WS-chain composition owner. It wires the three reducers
//! (`ClientConnectionIndex`, `RuntimeGenerationPinLedger`,
//! `WebSocketRequestBroker`) through typed ports and routes peer close /
//! protocol-close / inbound-terminal outcomes into the connection finalizer.

use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::session::identity::RuntimeSessionEpoch;

use super::broker::{
    BrokerHealthSnapshot, InboundCompletionOutcome, PeerTextOutcome, RuntimeRequest,
    RuntimeRequestOutcome, RuntimeSendOutcome, WebSocketRequestBroker,
    WebSocketRequestBrokerOptions,
};
use super::index::{
    AdmissionOutcome, AttachMeta, BrokerGenerationPort, ClientConnectionIndex,
    ClientConnectionIndexOptions, IndexHealthSnapshot, LedgerReleasePort, OverflowPolicy,
};
use super::ledger::{
    LedgerHealthSnapshot, LedgerOptions, PendingAdmissionSender, ReleaseOutcome,
    RuntimeGenerationPeer, RuntimeGenerationPinLedger, RuntimeSessionClose,
};
use super::types::{
    BrokerConnectionGeneration, BrokerRuntimeSource, BusinessKey, ClientTerminal, Clock,
    DispatchInbound, InboundDispatchResult, InboundExecutionToken, MethodCatalog,
    NotificationObserver, OwnerToken, PeerWriter, RuntimeViolationSink, SystemClock,
    WebSocketLifecycleClose, WsHealthSnapshot,
};

#[derive(Debug, Clone, Default)]
pub struct WebSocketLaneOptions {
    pub index: ClientConnectionIndexOptions,
    pub ledger: LedgerOptions,
    pub broker: WebSocketRequestBrokerOptions,
}

#[derive(Debug)]
pub struct BrokerGenerationAdapter {
    broker: Arc<WebSocketRequestBroker>,
}

impl BrokerGenerationPort for BrokerGenerationAdapter {
    fn attach_generation(
        &self,
        handle: BrokerConnectionGeneration,
        writer: Arc<dyn PeerWriter>,
        generation: u64,
    ) -> Result<OwnerToken, String> {
        self.broker.attach_generation(handle, writer, generation)
    }

    fn close_generation(
        &self,
        connection_id: &str,
        generation: &skiff_runtime_transport::connection_protocol::ClientSocketGeneration,
        protocol_outcome: bool,
    ) -> Result<(), String> {
        self.broker
            .close_generation(connection_id, generation, protocol_outcome)
    }
}

#[derive(Debug)]
pub struct LedgerReleaseAdapter {
    ledger: Arc<RuntimeGenerationPinLedger>,
}

impl LedgerReleasePort for LedgerReleaseAdapter {
    fn release_connection(
        &self,
        connection_id: &str,
        socket_open: bool,
    ) -> Result<ReleaseOutcome, String> {
        self.ledger.release_connection(connection_id, socket_open)
    }
}

/// WS-chain composition (fake-seam consumer for corpus/probe; production
/// wiring lands the same ports).
#[derive(Debug)]
pub struct WebSocketLane {
    pub index: Arc<ClientConnectionIndex>,
    pub ledger: Arc<RuntimeGenerationPinLedger>,
    pub broker: Arc<WebSocketRequestBroker>,
}

impl WebSocketLane {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        options: WebSocketLaneOptions,
        runtime_peer: Arc<dyn RuntimeGenerationPeer>,
        runtime_close: Arc<dyn RuntimeSessionClose>,
        admission: Arc<dyn PendingAdmissionSender>,
        methods: Arc<dyn MethodCatalog>,
        notifications: Arc<dyn NotificationObserver>,
        violations: Arc<dyn RuntimeViolationSink>,
        dispatch: Arc<dyn DispatchInbound>,
    ) -> Arc<Self> {
        Self::with_clock(
            options,
            runtime_peer,
            runtime_close,
            admission,
            methods,
            notifications,
            violations,
            dispatch,
            Arc::new(SystemClock),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_clock(
        options: WebSocketLaneOptions,
        runtime_peer: Arc<dyn RuntimeGenerationPeer>,
        runtime_close: Arc<dyn RuntimeSessionClose>,
        admission: Arc<dyn PendingAdmissionSender>,
        methods: Arc<dyn MethodCatalog>,
        notifications: Arc<dyn NotificationObserver>,
        violations: Arc<dyn RuntimeViolationSink>,
        dispatch: Arc<dyn DispatchInbound>,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        let ledger = Arc::new(RuntimeGenerationPinLedger::with_clock(
            runtime_peer,
            runtime_close,
            admission,
            options.ledger,
            clock.clone(),
        ));
        let broker = Arc::new(WebSocketRequestBroker::with_clock(
            methods,
            notifications,
            violations,
            dispatch,
            options.broker,
            clock.clone(),
        ));
        let index = ClientConnectionIndex::with_clock(
            Arc::new(BrokerGenerationAdapter {
                broker: broker.clone(),
            }),
            Arc::new(LedgerReleaseAdapter {
                ledger: ledger.clone(),
            }),
            options.index,
            clock,
        );
        Arc::new(Self {
            index,
            ledger,
            broker,
        })
    }

    pub fn reserve(&self, id: &str) -> Result<(), String> {
        self.index.reserve(id)
    }

    pub fn admit(
        self: &Arc<Self>,
        id: &str,
        business_key: Option<BusinessKey>,
        rank: Option<u64>,
        max_connections: usize,
        overflow: OverflowPolicy,
    ) -> AdmissionOutcome {
        self.index
            .admit(id, business_key, rank, max_connections, overflow)
    }

    pub fn attach(
        self: &Arc<Self>,
        id: &str,
        generation: u64,
        display: String,
        runtime: RuntimeSessionEpoch,
        writer: Arc<dyn PeerWriter>,
        meta: AttachMeta,
    ) -> Result<Arc<dyn PeerWriter>, String> {
        self.index
            .attach(id, generation, display, runtime, writer, meta)
    }

    pub fn finish(
        self: &Arc<Self>,
        id: &str,
        terminal: ClientTerminal,
        close: Option<WebSocketLifecycleClose>,
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        self.index.finish(id, terminal, close)
    }

    /// Runtime disconnect drives all three owners in finalizer order
    /// (C-ws §3.3, C-client-lifecycle §6.5).
    pub fn runtime_disconnected(
        self: &Arc<Self>,
        runtime: &RuntimeSessionEpoch,
    ) -> Vec<JoinHandle<Result<(), Vec<String>>>> {
        self.ledger.runtime_disconnected(runtime);
        self.broker.runtime_disconnected_sender(runtime);
        self.index.runtime_disconnected(runtime)
    }

    pub fn shutdown(self: &Arc<Self>) -> Vec<JoinHandle<Result<(), Vec<String>>>> {
        self.index.shutdown()
    }

    /// One peer text frame; protocol/budget closes enter the connection
    /// finalizer. Returns the finalizer handle when the frame closed the
    /// generation.
    pub fn handle_peer_text(
        self: &Arc<Self>,
        connection_id: &str,
        frame: &[u8],
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        match self.broker.handle_peer_text(connection_id, frame) {
            PeerTextOutcome::Ok => None,
            PeerTextOutcome::Close(close) => {
                self.index
                    .finish(connection_id, ClientTerminal::ProtocolClose, Some(close))
            }
        }
    }

    pub fn handle_peer_binary(
        self: &Arc<Self>,
        connection_id: &str,
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        match self.broker.handle_peer_binary(connection_id) {
            PeerTextOutcome::Ok => None,
            PeerTextOutcome::Close(close) => {
                self.index
                    .finish(connection_id, ClientTerminal::ProtocolClose, Some(close))
            }
        }
    }

    pub fn handle_peer_disconnect(
        self: &Arc<Self>,
        connection_id: &str,
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        self.index
            .finish(connection_id, ClientTerminal::PeerClose, None)
    }

    pub fn handle_runtime_request(
        &self,
        connection_id: &str,
        source: &BrokerRuntimeSource,
        request: &RuntimeRequest,
    ) -> RuntimeRequestOutcome {
        self.broker
            .handle_runtime_request(connection_id, source, request)
    }

    pub fn handle_runtime_cancel(&self, source: &BrokerRuntimeSource, request_id: &str) -> bool {
        self.broker.handle_runtime_cancel(source, request_id)
    }

    /// Runtime `connection.send` (server->client business message, TS
    /// parity). `connectionId` targets one exact generation; `businessIdentity`
    /// targets every admitted connection for that business key. At least one
    /// of the two targets must be present (never both).
    pub fn handle_runtime_send(
        &self,
        connection_id: Option<&str>,
        business_identity: Option<&str>,
        service_id: &str,
        websocket_entry_id: &str,
        payload_kind: &str,
        payload: &[u8],
    ) -> RuntimeSendOutcome {
        match (connection_id, business_identity) {
            (Some(connection_id), None) => self.broker.handle_runtime_send(
                connection_id,
                service_id,
                websocket_entry_id,
                payload_kind,
                payload,
            ),
            (None, Some(business_identity)) => {
                let connection_ids = self.index.connections_for_business_identity(
                    service_id,
                    websocket_entry_id,
                    business_identity,
                );
                if connection_ids.is_empty() {
                    return RuntimeSendOutcome::DeliveryMiss {
                        reason: format!(
                            "no admitted connection for business identity {business_identity}"
                        ),
                    };
                }
                let mut outcome = RuntimeSendOutcome::DeliveryMiss {
                    reason: "no delivery attempted".to_string(),
                };
                for connection_id in connection_ids {
                    outcome = self.broker.handle_runtime_send(
                        &connection_id,
                        service_id,
                        websocket_entry_id,
                        payload_kind,
                        payload,
                    );
                    if matches!(outcome, RuntimeSendOutcome::ProtocolViolation { .. }) {
                        return outcome;
                    }
                }
                outcome
            }
            _ => RuntimeSendOutcome::ProtocolViolation {
                reason:
                    "connection.send must target exactly one of connectionId or businessIdentity"
                        .to_string(),
            },
        }
    }

    pub fn complete_inbound(
        self: &Arc<Self>,
        token: &InboundExecutionToken,
        result: InboundDispatchResult,
    ) -> Option<JoinHandle<Result<(), Vec<String>>>> {
        match self.broker.complete_inbound(token, result) {
            InboundCompletionOutcome::Completed | InboundCompletionOutcome::IgnoredLate => None,
            InboundCompletionOutcome::Close(close) => self.index.finish(
                &token.connection_id,
                ClientTerminal::ProtocolClose,
                Some(close),
            ),
        }
    }

    pub fn fire_deadline(&self, connection_id: &str, request_id: &str) -> bool {
        self.broker.fire_deadline(connection_id, request_id)
    }

    pub fn fire_inbound_deadline(&self, token: &InboundExecutionToken) -> bool {
        self.broker.fire_inbound_deadline(token)
    }

    pub fn snapshot(&self) -> WsHealthSnapshot {
        let index: IndexHealthSnapshot = self.index.snapshot();
        let ledger: LedgerHealthSnapshot = self.ledger.snapshot();
        let broker: BrokerHealthSnapshot = self.broker.snapshot();
        WsHealthSnapshot {
            connection_count: index.connection_count,
            open_connections: index.open_connections,
            finalizer_pending: index.finalizer_pending,
            finalizer_count: index.finalizer_count,
            finalizer_failures: index.finalizer_failures,
            slow_client_count: index.slow_client_count,
            pins_acquired: ledger.pins_acquired,
            pins_pending_release: ledger.pins_pending_release,
            release_acks: ledger.release_acks,
            release_failures: ledger.release_failures,
            runtime_closed: ledger.runtime_closed,
            generation_count: broker.generation_count,
            outbound_pending: broker.outbound_pending,
            inbound_pending: broker.inbound_pending,
            tombstones: broker.outbound_tombstones + broker.inbound_tombstones,
            timer_count: broker.timer_count,
            fail_stop_reason: ledger.fail_stop_reason,
        }
    }
}
