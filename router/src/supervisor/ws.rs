//! E-ws production assembly: WebSocket gateway surface, connect admission,
//! inbound JSON-RPC dispatch and the ledger admission sender
//! (authority design §7 E-ws; C-ws / C-client-lifecycle / C-model-connection).
//!
//! The composition owns only correlation/pending state that has no lane
//! owner: the connect-admission correlation, the pinned-connection table and
//! the inbound JSON-RPC dispatch correlation. The WS lane owners
//! (`ClientConnectionIndex`, `RuntimeGenerationPinLedger`,
//! `WebSocketRequestBroker`) stay untouched and are consumed through their
//! public APIs; ordinary dispatch pending stays in `RequestDispatcher` and is
//! never duplicated here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use skiff_artifact_identity::websocket_entry_id;
use skiff_artifact_model::{
    GatewayEntryIdentity, GatewayProtocolSurface, IngressProtocol, ServiceDeploymentRef,
    WebSocketEntryId,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_transport::protocol::{encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestDeadlineFrameHeader,
    RuntimeAssemblyRequestNameValueFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
    RuntimeAssemblyWebSocketConnectIngressFrameHeader,
    RuntimeAssemblyWebSocketConnectIngressProtocol,
    RuntimeAssemblyWebSocketConnectRequestFrameHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader, RuntimeAssemblyWebSocketJsonRpcProfile,
    RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
    RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader,
};
use skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleTuple;

use crate::dispatch::{Reservation, RuntimeAdmissionPool};
use crate::routing::{CandidateQuery, DispatchMode, RuntimeCandidateQuery};
use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;
use crate::ws::{
    DispatchInbound, InboundDispatchAction, InboundDispatchResult, InboundExecutionToken,
    MethodCatalog, PendingAdmissionSender, WebSocketLane,
};

use super::session_ports::SessionHandle;

/// One JSON-RPC method binding inside a WebSocket connect binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsMethodBinding {
    pub method: String,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

/// One exact WebSocket connect binding (deployment-record projection).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsBinding {
    pub service_id: String,
    pub deployment: ServiceDeploymentRef,
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub websocket_entry_id: String,
    pub path: String,
    /// Whether the connect entry declares a handler (TS
    /// `requiresRuntimePin`); handler-less + method-less bindings fail closed
    /// at admission because the WS lane attach requires an exact runtime.
    pub connect_handler: bool,
    pub methods: BTreeMap<String, WsMethodBinding>,
}

/// Immutable WS gateway surface loaded from committed deployment records.
#[derive(Debug, Clone, Default)]
pub struct WsGatewaySurfaceView {
    by_service_path: BTreeMap<(String, String), WsBinding>,
}

impl WsGatewaySurfaceView {
    /// Composition/test constructor from explicit bindings (the production
    /// loader builds this from deployment records).
    pub fn from_bindings(by_service_path: BTreeMap<(String, String), WsBinding>) -> Self {
        Self { by_service_path }
    }

    pub fn resolve(&self, service_id: &str, path: &str) -> Option<&WsBinding> {
        self.by_service_path
            .get(&(service_id.to_string(), path.to_string()))
    }

    /// Union of every declared JSON-RPC method (the W-WebSocket
    /// `MethodCatalog` seam is global; per-connection exactness is enforced
    /// again at inbound dispatch time).
    pub fn method_union(&self) -> BTreeSet<String> {
        self.by_service_path
            .values()
            .flat_map(|binding| binding.methods.keys().cloned())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.by_service_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_service_path.is_empty()
    }
}

/// Loads the WS surface from the deployment records referenced by the
/// release pointer table (M4: pointer-table scan).
pub fn load_ws_surface_view(
    artifact_root: &Path,
    profile: &str,
) -> Result<Arc<WsGatewaySurfaceView>, String> {
    let store = CanonicalArtifactStore::open(artifact_root)
        .map_err(|error| format!("open artifact store for WS surface: {error}"))?;
    ws_surface_view_from_store(&store, profile)
}

/// Builds the WS surface view from the deployment records referenced by the
/// release pointer table (M4: every connect admission resolves against the
/// current pointer table so a stale deployment revision never survives a
/// pointer update).
pub fn ws_surface_view_from_store(
    store: &CanonicalArtifactStore,
    profile: &str,
) -> Result<Arc<WsGatewaySurfaceView>, String> {
    let release: Arc<dyn crate::release::ReleaseResolver> =
        Arc::new(crate::release::StoreReleaseResolver::new(store.clone()));
    let mut by_service_path = BTreeMap::new();
    for deployment in release
        .all_deployments(profile)
        .map_err(|error| format!("scan release pointers for WS surface: {error}"))?
    {
        let record = store.read_service_deployment(&deployment).map_err(|error| {
            format!(
                "read deployment record {} for WS surface: {error}",
                deployment.service_id
            )
        })?;
        for ingress in &record.ingress {
            if ingress.selector.protocol != IngressProtocol::WebSocket {
                continue;
            }
            let entry = record
                .gateway_entries
                .get(&ingress.gateway_entry_key)
                .ok_or_else(|| {
                    format!(
                        "deployment {} websocket ingress {} references missing gateway entry",
                        deployment.service_id,
                        ingress.gateway_entry_key.as_str()
                    )
                })?;
            let key = (deployment.service_id.clone(), ingress.selector.path.clone());
            match (&entry.protocol_surface.protocol, &ingress.selector.method) {
                (GatewayProtocolSurface::WebSocketConnect(_), None) => {
                    if by_service_path.contains_key(&key) {
                        return Err(format!(
                            "duplicate websocket connect binding for {} {}",
                            key.0, key.1
                        ));
                    }
                    let websocket_entry_id =
                        websocket_entry_id(&deployment.service_id, &ingress.gateway_entry_key)
                            .map_err(|error| {
                                format!(
                                    "derive websocket entry id for {}: {error}",
                                    deployment.service_id
                                )
                            })?;
                    by_service_path.insert(
                        key,
                        WsBinding {
                            service_id: deployment.service_id.clone(),
                            deployment: deployment.clone(),
                            gateway_entry_identity: entry.gateway_entry_identity.clone(),
                            websocket_entry_id: websocket_entry_id.to_string(),
                            path: ingress.selector.path.clone(),
                            connect_handler: entry.handler.is_some(),
                            methods: BTreeMap::new(),
                        },
                    );
                }
                (GatewayProtocolSurface::WebSocketJsonRpc(_), Some(method)) => {
                    let binding = by_service_path.get_mut(&key).ok_or_else(|| {
                        format!(
                            "websocket JSON-RPC method {} references missing connect binding {} {}",
                            method, key.0, key.1
                        )
                    })?;
                    if binding.methods.contains_key(method) {
                        return Err(format!(
                            "duplicate websocket JSON-RPC method {} for {} {}",
                            method, key.0, key.1
                        ));
                    }
                    binding.methods.insert(
                        method.clone(),
                        WsMethodBinding {
                            method: method.clone(),
                            gateway_entry_identity: entry.gateway_entry_identity.clone(),
                        },
                    );
                }
                _ => {
                    return Err(format!(
                        "deployment {} websocket ingress {} must be websocketConnect (method null) or websocketJsonRpc (method present)",
                        deployment.service_id, ingress.gateway_entry_key.as_str()
                    ));
                }
            }
        }
    }
    Ok(Arc::new(WsGatewaySurfaceView { by_service_path }))
}

/// Exact-session frame writer port (production = session layer outbound
/// registry; tests inject fakes).
pub trait WsSessionWriter: Send + Sync + fmt::Debug {
    fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct LayerWsSessionWriter {
    session: SessionHandle,
}

impl LayerWsSessionWriter {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl WsSessionWriter for LayerWsSessionWriter {
    fn write(&self, runtime: &RuntimeSessionEpoch, bytes: Vec<u8>) -> Result<(), String> {
        let layer = self
            .session
            .layer()
            .ok_or_else(|| "session layer is not wired yet".to_string())?;
        layer.write_session_frame(runtime, bytes)
    }
}

/// WS connect runtime selection port. The production implementation uses the
/// canonical candidate projection plus a composition-owned admission pool
/// (WS dispatch capacity is separate from ordinary dispatch capacity;
/// owner-split composition decision).
pub trait WsConnectSelector: Send + Sync + fmt::Debug {
    fn select(
        &self,
        connection_id: &str,
        binding: &WsBinding,
    ) -> Result<RuntimeSessionEpoch, String>;
    fn release(&self, connection_id: &str);
}

#[derive(Debug)]
pub struct ProductionWsConnectSelector {
    view: Arc<dyn crate::dispatch::CandidateViewSource>,
    pool: RuntimeAdmissionPool,
    reservations: Mutex<HashMap<String, Reservation>>,
}

impl ProductionWsConnectSelector {
    pub fn new(
        view: Arc<dyn crate::dispatch::CandidateViewSource>,
        max_concurrency: usize,
    ) -> Self {
        Self {
            view,
            pool: RuntimeAdmissionPool::new(max_concurrency),
            reservations: Mutex::new(HashMap::new()),
        }
    }

    pub fn pool(&self) -> RuntimeAdmissionPool {
        self.pool.clone()
    }
}

impl WsConnectSelector for ProductionWsConnectSelector {
    fn select(
        &self,
        connection_id: &str,
        binding: &WsBinding,
    ) -> Result<RuntimeSessionEpoch, String> {
        // M4: the gate is the candidate projection over the binding's build
        // id; there is no routing epoch.
        let view = self.view.view();
        let query = CandidateQuery {
            mode: DispatchMode::Unary,
            build_id: binding.deployment.deployment_artifact_identity.as_str().to_string(),
        };
        let leases = RuntimeCandidateQuery.query(&view, &query);
        let selected = self
            .pool
            .select(&leases, None)
            .ok_or_else(|| "no eligible runtime for websocket connect".to_string())?;
        let session = selected.lease.session_epoch.clone();
        self.reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection_id.to_string(), selected.reservation);
        Ok(session)
    }

    fn release(&self, connection_id: &str) {
        if let Some(reservation) = self
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(connection_id)
        {
            reservation.release();
        }
    }
}

/// Metadata extracted from the upgrade HTTP request for the
/// `websocketConnect` frame (C-model-connection connect request shape).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WsConnectMetadata {
    pub url: String,
    pub query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
}

/// One settled connect outcome (accept/reject/unavailable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectOutcome {
    Accepted {
        business_identity: Option<String>,
        admission_rank: Option<u64>,
        max_connections: u32,
        overflow: crate::ws::OverflowPolicy,
        close_code: Option<u16>,
        close_reason: Option<String>,
    },
    Rejected {
        code: u16,
        reason: String,
    },
    Unavailable {
        reason: String,
    },
}

/// Pinned WS connection record (composition-owned; the lane index remains
/// the authoritative connection lifecycle).
#[derive(Debug, Clone)]
pub struct WsConnectionRecord {
    pub connection_id: String,
    pub runtime: RuntimeSessionEpoch,
    pub binding: WsBinding,
    pub business_identity: Option<String>,
    /// Deployment build id the connection is pinned to (M4).
    pub build_id: String,
}

#[derive(Debug, Clone)]
struct ConnectPending {
    runtime: RuntimeSessionEpoch,
    connection_id: String,
    response: tokio::sync::watch::Sender<Option<ConnectOutcome>>,
}

#[derive(Debug)]
struct InboundPending {
    token: InboundExecutionToken,
    runtime: RuntimeSessionEpoch,
    reservation: Reservation,
}

/// In-flight connect admission (TS parity: the Runtime sends the generation
/// `Acquire` while the websocketConnect dispatch is still pending, before
/// `response.end` settles and the pinned connection table is populated).
#[derive(Debug, Clone)]
struct PendingAdmission {
    runtime: RuntimeSessionEpoch,
    service_id: String,
    build_id: String,
    websocket_entry_id: String,
    connection_id: String,
}

impl PendingAdmission {
    /// TS `isPendingWebSocketAcquireSender` parity: the sender owns the
    /// pending admission when the build-id/entry/connection tuple matches
    /// the in-flight websocketConnect request (M4: buildId keying).
    fn matches(
        &self,
        runtime: &RuntimeSessionEpoch,
        tuple: &WebSocketGenerationLifecycleTuple,
    ) -> bool {
        self.runtime == *runtime
            && self.service_id == tuple.service_id
            && self.build_id == tuple.build_id
            && self.websocket_entry_id == tuple.websocket_entry_id
            && self.connection_id == tuple.connection_id
    }
}

#[derive(Debug, Default)]
struct WsDispatchInner {
    connections: HashMap<String, WsConnectionRecord>,
    connects: HashMap<String, ConnectPending>,
    pending_admissions: HashMap<String, PendingAdmission>,
    inbound: HashMap<String, InboundPending>,
}

/// Deferred reference to the process `WebSocketLane` (composition seam).
///
/// The WS dispatch store is constructed before the lane (the lane consumes
/// store-backed ports), so inbound terminals resolve the lane through this
/// handle. The supervisor sets it immediately after lane construction;
/// `complete_inbound` fails closed if it is ever called before then.
#[derive(Debug, Clone, Default)]
pub struct WsLaneHandle {
    lane: Arc<Mutex<Option<Arc<WebSocketLane>>>>,
}

impl WsLaneHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, lane: Arc<WebSocketLane>) {
        *self
            .lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(lane);
    }

    pub fn lane(&self) -> Option<Arc<WebSocketLane>> {
        self.lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Composition-owned WS dispatch correlation store (no lane internals are
/// touched; ordinary dispatcher pending is never duplicated here).
#[derive(Debug)]
pub struct WsDispatchStore {
    lane: WsLaneHandle,
    writer: Arc<dyn WsSessionWriter>,
    pool: RuntimeAdmissionPool,
    inner: Mutex<WsDispatchInner>,
    next_request_seq: AtomicU64,
    inbound_timeout_ms: u64,
}

impl WsDispatchStore {
    pub fn new(
        lane: WsLaneHandle,
        writer: Arc<dyn WsSessionWriter>,
        pool: RuntimeAdmissionPool,
        inbound_timeout_ms: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            lane,
            writer,
            pool,
            inner: Mutex::new(WsDispatchInner::default()),
            next_request_seq: AtomicU64::new(0),
            inbound_timeout_ms,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WsDispatchInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn new_request_id(&self) -> String {
        format!(
            "ws-{}-{}-{}",
            now_nanos(),
            self.next_request_seq.fetch_add(1, Ordering::Relaxed),
            std::process::id()
        )
    }

    /// Registers the pinned connection after connect Accept (before attach).
    pub fn register_connection(&self, record: WsConnectionRecord) {
        self.lock()
            .connections
            .insert(record.connection_id.clone(), record);
    }

    /// Removes the pinned connection on finalizer/cleanup.
    pub fn unregister_connection(&self, connection_id: &str) {
        let mut inner = self.lock();
        inner.connections.remove(connection_id);
        let affected = inner
            .inbound
            .iter()
            .filter(|(_, pending)| pending.token.connection_id == connection_id)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in affected {
            if let Some(pending) = inner.inbound.remove(&request_id) {
                pending.reservation.release();
            }
        }
    }

    pub fn connection_runtime(&self, connection_id: &str) -> Option<RuntimeSessionEpoch> {
        self.lock()
            .connections
            .get(connection_id)
            .map(|record| record.runtime.clone())
    }

    /// Read-only pinned connection record (used by the client WS peer task
    /// to attach with the captured binding).
    pub fn connection_record(&self, connection_id: &str) -> Option<WsConnectionRecord> {
        self.lock().connections.get(connection_id).cloned()
    }

    /// Starts one connect admission: builds the canonical
    /// `websocketConnect` request.start, writes it to the exact runtime and
    /// registers the correlation. Returns the request id and a wait handle.
    pub fn connect_begin(
        &self,
        connection_id: &str,
        binding: &WsBinding,
        runtime: &RuntimeSessionEpoch,
        build_id: &str,
        metadata: &WsConnectMetadata,
        timeout_ms: u64,
    ) -> Result<(String, tokio::sync::watch::Receiver<Option<ConnectOutcome>>), String> {
        let request_id = self.new_request_id();
        let (tx, rx) = tokio::sync::watch::channel(None);
        let websocket_entry_id = WebSocketEntryId::parse(&binding.websocket_entry_id)
            .map_err(|error| format!("websocket entry id parse failed: {error}"))?;
        let header = RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: request_id.clone(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: None,
                assembly_generation: None,
                deployment: binding.deployment.clone(),
                build_id: Some(binding.deployment.deployment_artifact_identity.to_string()),
                gateway_entry_identity: binding.gateway_entry_identity.clone(),
                ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                    method: (),
                    path: binding.path.clone(),
                },
            },
            client_session: None,
            deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
                timeout_ms,
                expires_at: format_iso8601_now_plus(timeout_ms),
            }),
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: format!("ws-trace-{}", now_nanos()),
                span_id: format!("ws-span-{}", now_nanos()),
                parent_span_id: None,
                sampled: None,
            },
            websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
                connection_id: connection_id.to_string(),
                url: metadata.url.clone(),
                query: metadata.query.clone(),
                headers: metadata.headers.clone(),
                cookies: metadata.cookies.clone(),
                version: None,
                websocket_entry_id,
                gateway_entry_identity: binding.gateway_entry_identity.clone(),
            },
            test_effects_enabled: false,
        };
        let bytes = encode_binary_frame(&header, &[])
            .map_err(|error| format!("websocketConnect encode failed: {error}"))?;
        self.writer.write(runtime, bytes)?;
        {
            let mut inner = self.lock();
            inner.pending_admissions.insert(
                connection_id.to_string(),
                PendingAdmission {
                    runtime: runtime.clone(),
                    service_id: binding.service_id.clone(),
                    build_id: build_id.to_string(),
                    websocket_entry_id: binding.websocket_entry_id.clone(),
                    connection_id: connection_id.to_string(),
                },
            );
            inner.connects.insert(
                request_id.clone(),
                ConnectPending {
                    runtime: runtime.clone(),
                    connection_id: connection_id.to_string(),
                    response: tx,
                },
            );
        }
        Ok((request_id, rx))
    }

    /// Settles one connect correlation (response.end decode).
    pub fn connect_response(&self, request_id: &str, outcome: ConnectOutcome) {
        let mut inner = self.lock();
        if let Some(pending) = inner.connects.remove(request_id) {
            inner.pending_admissions.remove(&pending.connection_id);
            let _ = pending.response.send(Some(outcome));
        }
    }

    /// Fails one connect correlation (timeout / session close / write error).
    pub fn connect_unavailable(&self, request_id: &str, reason: String) {
        self.connect_response(request_id, ConnectOutcome::Unavailable { reason });
    }

    /// Inbound JSON-RPC dispatch (C-ws §4.4 production seam).
    pub fn dispatch_inbound(&self, action: &InboundDispatchAction) -> Result<(), String> {
        let record = self
            .lock()
            .connections
            .get(&action.connection_id)
            .cloned()
            .ok_or_else(|| "websocket connection is not pinned".to_string())?;
        let method_binding =
            record.binding.methods.get(&action.method).ok_or_else(|| {
                "websocket method is not declared for this connection".to_string()
            })?;
        let reservation = self
            .pool
            .reserve_exact(&record.runtime)
            .ok_or_else(|| "websocket runtime dispatch capacity reached".to_string())?;
        let request_id = self.new_request_id();
        let websocket_entry_id = WebSocketEntryId::parse(&record.binding.websocket_entry_id)
            .map_err(|error| format!("websocket entry id parse failed: {error}"))?;
        let header = RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: request_id.clone(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: None,
                assembly_generation: None,
                deployment: record.binding.deployment.clone(),
                build_id: Some(
                    record.binding.deployment.deployment_artifact_identity.to_string(),
                ),
                gateway_entry_identity: method_binding.gateway_entry_identity.clone(),
                ingress: RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
                    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                    method: action.method.clone(),
                    path: record.binding.path.clone(),
                },
            },
            client_session: None,
            deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
                timeout_ms: self.inbound_timeout_ms,
                expires_at: format_iso8601_now_plus(self.inbound_timeout_ms),
            }),
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: format!("ws-trace-{}", now_nanos()),
                span_id: format!("ws-span-{}", now_nanos()),
                parent_span_id: None,
                sampled: None,
            },
            websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader {
                profile: RuntimeAssemblyWebSocketJsonRpcProfile::JsonRpc2_0Text,
                connection_id: action.connection_id.clone(),
                websocket_entry_id,
                gateway_entry_identity: method_binding.gateway_entry_identity.clone(),
                business_identity: record.business_identity.clone(),
            },
            test_effects_enabled: false,
        };
        let bytes = encode_binary_frame(&header, &action.params)
            .map_err(|error| format!("websocketJsonRpc encode failed: {error}"))?;
        self.writer.write(&record.runtime, bytes)?;
        self.lock().inbound.insert(
            request_id,
            InboundPending {
                token: action.execution_token.clone(),
                runtime: record.runtime.clone(),
                reservation,
            },
        );
        let token = action.execution_token.clone();
        let lane = self.lane.clone();
        let timeout = Duration::from_millis(self.inbound_timeout_ms);
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if let Some(lane) = lane.lane() {
                let _ = lane.fire_inbound_deadline(&token);
            }
        });
        Ok(())
    }

    /// Settles one inbound JSON-RPC dispatch from a `response.end` outcome.
    pub fn on_inbound_response(
        &self,
        request_id: &str,
        outcome: RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
        payload: Vec<u8>,
    ) {
        let pending = self.lock().inbound.remove(request_id);
        if let Some(pending) = pending {
            pending.reservation.release();
            let result = match outcome {
                RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success => {
                    InboundDispatchResult::Success { result: payload }
                }
                RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InvalidParams => {
                    InboundDispatchResult::InvalidParams
                }
                RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InternalError => {
                    InboundDispatchResult::InternalError
                }
                RuntimeAssemblyWebSocketJsonRpcResponseOutcome::DeadlineExceeded => {
                    InboundDispatchResult::DeadlineExceeded
                }
            };
            if let Some(lane) = self.lane.lane() {
                let _ = lane.complete_inbound(&pending.token, result);
            }
        }
    }

    /// Settles one inbound JSON-RPC dispatch from a non-end terminal
    /// (`response.error` / `request.cancel`): fail closed to the broker.
    pub fn on_inbound_terminal(&self, request_id: &str, result: InboundDispatchResult) {
        let pending = self.lock().inbound.remove(request_id);
        if let Some(pending) = pending {
            pending.reservation.release();
            if let Some(lane) = self.lane.lane() {
                let _ = lane.complete_inbound(&pending.token, result);
            }
        }
    }

    /// Fails every pending connect/inbound of the exact runtime (called from
    /// the composition's session-consumer wrapper on runtime disconnect).
    pub fn on_session_closed(&self, runtime: &RuntimeSessionEpoch) {
        let mut inner = self.lock();
        inner
            .connections
            .retain(|_, record| &record.runtime != runtime);
        inner
            .pending_admissions
            .retain(|_, pending| &pending.runtime != runtime);
        let connects = inner
            .connects
            .iter()
            .filter(|(_, pending)| &pending.runtime == runtime)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in connects {
            if let Some(pending) = inner.connects.remove(&request_id) {
                inner.pending_admissions.remove(&pending.connection_id);
                let _ = pending.response.send(Some(ConnectOutcome::Unavailable {
                    reason: "runtime disconnected".to_string(),
                }));
            }
        }
        let inbound = inner
            .inbound
            .iter()
            .filter(|(_, pending)| &pending.runtime == runtime)
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in inbound {
            if let Some(pending) = inner.inbound.remove(&request_id) {
                pending.reservation.release();
                if let Some(lane) = self.lane.lane() {
                    let _ = lane.complete_inbound(
                        &pending.token,
                        InboundDispatchResult::RuntimeUnavailable,
                    );
                }
            }
        }
    }

    pub fn pending_connect_count(&self) -> usize {
        self.lock().connects.len()
    }

    pub fn pending_admission_count(&self) -> usize {
        self.lock().pending_admissions.len()
    }

    /// Answers whether the runtime owns an in-flight websocketConnect
    /// admission matching the acquire tuple (TS parity; called by the
    /// production `PendingAdmissionSender`).
    pub fn is_pending_admission_sender(
        &self,
        runtime: &RuntimeSessionEpoch,
        tuple: &WebSocketGenerationLifecycleTuple,
    ) -> bool {
        self.lock()
            .pending_admissions
            .get(&tuple.connection_id)
            .is_some_and(|pending| pending.matches(runtime, tuple))
    }

    pub fn pending_inbound_count(&self) -> usize {
        self.lock().inbound.len()
    }

    pub fn has_inbound(&self, request_id: &str) -> bool {
        self.lock().inbound.contains_key(request_id)
    }

    pub fn pinned_connection_count(&self) -> usize {
        self.lock().connections.len()
    }
}

/// Production `MethodCatalog`: union of all WS JSON-RPC methods in the
/// committed epoch surface (W-WebSocket global-catalog seam).
#[derive(Debug)]
pub struct WsMethodCatalog {
    surface: Arc<WsGatewaySurfaceView>,
}

impl WsMethodCatalog {
    pub fn new(surface: Arc<WsGatewaySurfaceView>) -> Self {
        Self { surface }
    }
}

impl MethodCatalog for WsMethodCatalog {
    fn accepts(&self, method: &str) -> bool {
        self.surface.method_union().contains(method)
    }
}

/// Production `PendingAdmissionSender`: the pinned connection table answers
/// whether the Runtime sender owns the pending WebSocket connect admission.
#[derive(Debug, Clone)]
pub struct WsPendingAdmissionSender {
    store: Arc<WsDispatchStore>,
}

impl WsPendingAdmissionSender {
    pub fn new(store: Arc<WsDispatchStore>) -> Self {
        Self { store }
    }
}

impl PendingAdmissionSender for WsPendingAdmissionSender {
    fn is_pending_acquire_sender(
        &self,
        runtime: &RuntimeSessionEpoch,
        tuple: &WebSocketGenerationLifecycleTuple,
    ) -> bool {
        // The Runtime acquire arrives while the websocketConnect dispatch is
        // still pending (before `response.end` populates the pinned
        // connection table); the in-flight admission answers it. The pinned
        // table remains the fallback for post-settle lookups.
        self.store.is_pending_admission_sender(runtime, tuple)
            || self.store.connection_runtime(&tuple.connection_id).as_ref() == Some(runtime)
    }
}

/// Production `DispatchInbound`: pinned-connection JSON-RPC dispatch.
#[derive(Debug, Clone)]
pub struct WsInboundDispatch {
    store: Arc<WsDispatchStore>,
}

impl WsInboundDispatch {
    pub fn new(store: Arc<WsDispatchStore>) -> Self {
        Self { store }
    }
}

impl DispatchInbound for WsInboundDispatch {
    fn dispatch(&self, action: InboundDispatchAction) -> Result<(), String> {
        self.store.dispatch_inbound(&action)
    }
}

/// Session-consumer wrapper: on runtime disconnect the composition drives the
/// WS lane finalizer (`WebSocketLane::runtime_disconnected`) and fails its
/// own dispatch correlations, then delegates to the wrapped lane consumer
/// (idempotent double cleanup by design; the ledger/broker removals are
/// exact-fence and repeat-safe).
#[derive(Debug, Clone)]
pub struct WsLaneSessionConsumer {
    lane: Arc<WebSocketLane>,
    store: Arc<WsDispatchStore>,
    inner: Arc<dyn SessionConsumer>,
}

impl WsLaneSessionConsumer {
    pub fn new(
        lane: Arc<WebSocketLane>,
        store: Arc<WsDispatchStore>,
        inner: Arc<dyn SessionConsumer>,
    ) -> Self {
        Self { lane, store, inner }
    }
}

impl SessionConsumer for WsLaneSessionConsumer {
    fn kind(&self) -> ConsumerKind {
        self.inner.kind()
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        self.store.on_session_closed(session);
        let _ = self.lane.runtime_disconnected(session);
        self.inner.on_session_closed(session)
    }
}

/// Helpers for request-id/deadline generation (composition-local utilities;
/// no HTTP lane module is modified).
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub(crate) fn format_iso8601_now_plus(timeout_ms: u64) -> String {
    let instant = SystemTime::now() + Duration::from_millis(timeout_ms);
    let duration = instant.duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = duration.as_millis() as u64;
    let days = millis / 86_400_000;
    let millis_of_day = millis % 86_400_000;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = millis_of_day / 3_600_000;
    let minute = (millis_of_day % 3_600_000) / 60_000;
    let second = (millis_of_day % 60_000) / 1000;
    let millisecond = millis_of_day % 1000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millisecond:03}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month as i64, day as i64)
}
