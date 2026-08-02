//! W-WebSocket shared identities, options and typed seams
//! (C-model-connection §2, C-client-lifecycle §2/§6.2, C-ws §2/§5.2).
//!
//! `ClientSocketGeneration` and `OpaquePeerId` are consumed from the frozen
//! `skiff-runtime-transport` codec; this module never re-implements them.

use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use skiff_runtime_transport::connection_protocol::{
    ClientSocketGeneration, ConnectionRemoteErrorFrameHeader, ConnectionResponseOutcome,
    OpaquePeerId, WebSocketRpcProfile,
};

use crate::session::identity::RuntimeSessionEpoch;

pub const CONNECTION_LIMIT_DEFAULT: usize = 5000;
pub const SLOW_CLIENT_BUDGET_BYTES_DEFAULT: u64 = 16 * 1024 * 1024;
pub const RELEASE_TIMEOUT_MS_DEFAULT: u64 = 5000;
pub const OUTBOUND_GLOBAL_CAPACITY_DEFAULT: usize = 4096;
pub const OUTBOUND_PER_GENERATION_CAPACITY_DEFAULT: usize = 128;
pub const INBOUND_GLOBAL_CAPACITY_DEFAULT: usize = 4096;
pub const INBOUND_PER_GENERATION_CAPACITY_DEFAULT: usize = 128;
pub const TOMBSTONE_CAPACITY_DEFAULT: usize = 4096;
pub const TOMBSTONE_TTL_MS_DEFAULT: u64 = 60_000;
pub const INBOUND_TIMEOUT_MS_DEFAULT: u64 = 120_000;
pub const SHUTDOWN_FINALIZER_TIMEOUT_MS_DEFAULT: u64 = 1000;

/// Injected monotonic clock (milliseconds). The broker and ledger only read
/// `now_ms`; timers are fired through explicit methods so every owner stays a
/// pure reducer (authority design §3.8).
pub trait Clock: Send + Sync + fmt::Debug {
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// A single explicit WebSocket close (code + reason <= 123 UTF-8 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSocketLifecycleClose {
    pub code: u16,
    pub reason: String,
}

impl WebSocketLifecycleClose {
    pub fn new(code: u16, reason: impl Into<String>) -> Result<Self, String> {
        let reason = reason.into();
        if reason.len() > 123 {
            return Err("websocket close reason exceeds 123 bytes".to_string());
        }
        Ok(Self { code, reason })
    }
}

/// One external terminal classification for a client socket generation
/// (C-client-lifecycle §4/§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientTerminal {
    /// Peer initiated close; no close frame is written.
    PeerClose,
    /// Business replacement closed the old generation.
    Replacement,
    /// Admission policy rejected the connection before attach.
    PolicyRejected,
    /// The exact Runtime session disconnected.
    RuntimeDisconnect,
    /// Process shutdown.
    Shutdown,
    /// Slow-client byte budget overflow.
    SlowClient,
    /// Peer protocol violation (1002/1003/1009).
    ProtocolClose,
    /// Runtime generation release timed out / rejected / send failure.
    ReleaseTimeout,
    /// Transport-level error.
    TransportError,
}

impl ClientTerminal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PeerClose => "PeerClose",
            Self::Replacement => "Replacement",
            Self::PolicyRejected => "PolicyRejected",
            Self::RuntimeDisconnect => "RuntimeDisconnect",
            Self::Shutdown => "Shutdown",
            Self::SlowClient => "SlowClient",
            Self::ProtocolClose => "ProtocolClose",
            Self::ReleaseTimeout => "ReleaseTimeout",
            Self::TransportError => "TransportError",
        }
    }
}

pub const CLOSE_POLICY_REJECTED: (u16, &str) = (1008, "connection rejected by admission policy");
pub const CLOSE_SUPERSEDED: (u16, &str) = (4009, "connection superseded");
pub const CLOSE_HIGH_WATER_CAPACITY: (u16, &str) = (1013, "admission high-water capacity");
pub const CLOSE_RUNTIME_DISCONNECTED: (u16, &str) = (1011, "websocket runtime disconnected");
pub const CLOSE_SHUTDOWN: (u16, &str) = (1001, "websocket gateway shutting down");
pub const CLOSE_SLOW_CLIENT: (u16, &str) = (1011, "websocket client is too slow");
pub const CLOSE_TRANSPORT_ERROR: (u16, &str) = (1011, "websocket transport error");
pub const CLOSE_RELEASE_TIMEOUT: (u16, &str) = (1008, "websocket generation release timed out");
pub const CLOSE_PROTOCOL_ERROR: (u16, &str) = (1002, "websocket protocol error");
pub const CLOSE_BINARY_FRAME: (u16, &str) = (1003, "binary frame not supported");

/// Business identity key: `service_id \0 websocket_entry_id \0
/// business_identity`; absent business identity means no key and no
/// replacement participation (C-client-lifecycle §2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BusinessKey(String);

impl BusinessKey {
    pub fn from_parts(service_id: &str, websocket_entry_id: &str, business_identity: &str) -> Self {
        Self(format!(
            "{service_id}\0{websocket_entry_id}\0{business_identity}"
        ))
    }

    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BusinessKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Broker-side generation handle (C-ws §2). `socket_generation` is the
/// wire-visible token used for peer ids (`<socketGeneration>:<seq>`); the
/// typed `ClientSocketGeneration` remains the lifecycle fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerConnectionGeneration {
    pub connection_id: String,
    pub socket_generation: String,
    pub service_id: String,
    pub websocket_entry_id: String,
    pub profile: WebSocketRpcProfile,
}

/// Opaque per-generation owner fence checked against every Runtime
/// `connection.request` (C-ws §4.2(2)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OwnerToken(pub u64);

/// Inbound execution token handed to the dispatcher (C-ws §2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InboundExecutionToken {
    pub connection_id: String,
    pub socket_generation: u64,
    pub sequence: u64,
}

/// Captured Runtime source for one outbound correlation (C-ws §2).
#[derive(Clone)]
pub struct BrokerRuntimeSource {
    pub sender: RuntimeSessionEpoch,
    pub session_token: String,
    pub respond: Arc<dyn RuntimeResponder>,
}

impl fmt::Debug for BrokerRuntimeSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrokerRuntimeSource")
            .field("sender", &self.sender)
            .field("session_token", &self.session_token)
            .finish_non_exhaustive()
    }
}

/// Port that delivers one `connection.response` to the exact Runtime
/// (C-model-connection §3.3).
pub trait RuntimeResponder: Send + Sync + fmt::Debug {
    fn respond(&self, response: &BrokerRuntimeResponse) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRuntimeResponse {
    pub request_id: String,
    pub outcome: ConnectionResponseOutcome,
    pub remote: Option<ConnectionRemoteErrorFrameHeader>,
    pub payload: Vec<u8>,
}

/// Inbound dispatcher terminal (C-ws §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundDispatchResult {
    Success { result: Vec<u8> },
    InvalidParams,
    InternalError,
    RuntimeUnavailable,
    DeadlineExceeded,
}

/// Inbound peer request handed to the dispatcher (C-ws §4.4).
#[derive(Debug, Clone)]
pub struct InboundDispatchAction {
    pub profile: WebSocketRpcProfile,
    pub connection_id: String,
    pub socket_generation: u64,
    pub peer_id: OpaquePeerId,
    pub method: String,
    /// Opaque lexical params slice; never decoded by the Router.
    pub params: Vec<u8>,
    pub execution_token: InboundExecutionToken,
    /// Cancellation signal; the dispatcher observes it (C-client-lifecycle
    /// §4 step 2: client cancellation is observable even under mailbox
    /// saturation).
    pub cancel: tokio::sync::watch::Receiver<bool>,
}

/// Dispatcher seam: the broker never owns the dispatcher's pending; the
/// dispatcher completes an inbound entry with
/// [`WebSocketRequestBroker::complete_inbound`] carrying the exact
/// `InboundExecutionToken`.
pub trait DispatchInbound: Send + Sync + fmt::Debug {
    fn dispatch(&self, action: InboundDispatchAction) -> Result<(), String>;
}

/// Method catalog for inbound peer requests (routing projection consumer
/// seam; defaults reject unknown methods).
pub trait MethodCatalog: Send + Sync + fmt::Debug {
    fn accepts(&self, method: &str) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct EmptyMethodCatalog;

impl MethodCatalog for EmptyMethodCatalog {
    fn accepts(&self, _method: &str) -> bool {
        false
    }
}

/// Notification observation is diagnostic and never produces an RPC terminal.
pub trait NotificationObserver: Send + Sync + fmt::Debug {
    fn observe(&self, connection_id: &str, method: &str, params: Option<&[u8]>);
}

#[derive(Debug, Clone, Default)]
pub struct NoopNotificationObserver;

impl NotificationObserver for NoopNotificationObserver {
    fn observe(&self, _connection_id: &str, _method: &str, _params: Option<&[u8]>) {}
}

/// Runtime protocol violation sink (C-ws §4.2(4)); the session owner decides
/// to terminate the exact Runtime session.
pub trait RuntimeViolationSink: Send + Sync + fmt::Debug {
    fn on_violation(&self, source: &BrokerRuntimeSource, reason: &str);
}

#[derive(Debug, Clone, Default)]
pub struct NoopRuntimeViolationSink;

impl RuntimeViolationSink for NoopRuntimeViolationSink {
    fn on_violation(&self, _source: &BrokerRuntimeSource, _reason: &str) {}
}

/// Captured per-generation writer (C-client-lifecycle §3.3). Implementations
/// must be single-writer per generation and report queued bytes through
/// [`PeerWriter::buffered_bytes`].
pub trait PeerWriter: Send + Sync + fmt::Debug {
    /// Enqueue one text frame. `Err` means queue full or transport closed;
    /// the exact correlation settles `transportUnavailable` and the owner
    /// decides whether to finish the connection.
    fn write_text(&self, frame: String) -> Result<(), String>;

    /// Queued + in-flight bytes (budget input).
    fn buffered_bytes(&self) -> u64;

    /// Graceful close (best effort; no wait for the queue to accept a close
    /// frame on overflow).
    fn close(&self, code: u16, reason: &str) -> Result<(), String>;

    /// Immediate terminate (drop the socket).
    fn terminate(&self);
}

/// Health snapshots (C-ws §5.6 / C-client-lifecycle §6.6). No business
/// payload, query, Authorization or cookie fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WsHealthSnapshot {
    pub connection_count: usize,
    pub open_connections: Vec<String>,
    pub finalizer_pending: usize,
    pub finalizer_count: u64,
    pub finalizer_failures: Vec<String>,
    pub slow_client_count: u64,
    pub pins_acquired: usize,
    pub pins_pending_release: usize,
    pub release_acks: u64,
    pub release_failures: Vec<String>,
    pub runtime_closed: Vec<RuntimeSessionEpoch>,
    pub generation_count: usize,
    pub outbound_pending: usize,
    pub inbound_pending: usize,
    pub tombstones: usize,
    pub timer_count: usize,
    pub fail_stop_reason: Option<String>,
}

/// Construct the typed generation identity; validates the connection token
/// through the frozen transport constructor.
pub fn client_generation(
    connection_id: &str,
    generation: u64,
) -> Result<ClientSocketGeneration, String> {
    ClientSocketGeneration::new(connection_id.to_string(), generation)
}
