//! W-WebSocket lane (Router Rust migration batch 7):
//! `ClientConnectionIndex` + `ClientSocketGeneration` finalizer,
//! `RuntimeGenerationPinLedger`, `WebSocketRequestBroker` and the frozen
//! JSON-RPC 2.0 text profile adapter (authority design §3.2/§3.7/§3.8,
//! §5.4 W-WebSocket, §7 E-ws; C-client-lifecycle / C-ws /
//! C-model-connection contracts).
//!
//! This module owns no ordinary dispatcher pending and never re-implements
//! transport codecs; the frozen `skiff-runtime-transport` classifier and
//! frame codecs are consumed directly.

pub mod broker;
pub mod index;
pub mod lane;
pub mod ledger;
pub mod profile;
pub mod types;

pub use broker::{
    BrokerHealthSnapshot, InboundCompletionOutcome, PeerTextOutcome, RuntimeRequest,
    RuntimeRequestOutcome, RuntimeSendOutcome, WebSocketRequestBroker,
    WebSocketRequestBrokerOptions,
};
pub use index::{
    AdmissionOutcome, AttachMeta, BrokerGenerationPort, ClientConnectionIndex,
    ClientConnectionIndexOptions, IndexHealthSnapshot, LedgerReleasePort, OverflowPolicy,
    WriteBudget,
};
pub use lane::{
    BrokerGenerationAdapter, LedgerReleaseAdapter, WebSocketLane, WebSocketLaneOptions,
};
pub use ledger::{
    AcquireDecision, AllowAnyPendingAdmission, LedgerHealthSnapshot, LedgerOptions,
    PendingAdmissionSender, PendingReleaseHandle, ReleaseOutcome, ReleaseResolution,
    RuntimeGenerationPeer, RuntimeGenerationPinLedger, RuntimeSessionClose,
};
pub use profile::{JsonRpc20TextProfile, PeerResponseTerminal, PlatformErrorKind, ProfileLimits};
pub use types::{
    BrokerConnectionGeneration, BrokerRuntimeResponse, BrokerRuntimeSource, BusinessKey,
    ClientTerminal, Clock, DispatchInbound, InboundDispatchAction, InboundDispatchResult,
    InboundExecutionToken, MethodCatalog, NoopNotificationObserver, NoopRuntimeViolationSink,
    NotificationObserver, OwnerToken, PeerWriter, RuntimeResponder, RuntimeViolationSink,
    WebSocketLifecycleClose, WsHealthSnapshot, CLOSE_HIGH_WATER_CAPACITY, CLOSE_POLICY_REJECTED,
    CLOSE_PROTOCOL_ERROR, CLOSE_RELEASE_TIMEOUT, CLOSE_RUNTIME_DISCONNECTED, CLOSE_SHUTDOWN,
    CLOSE_SLOW_CLIENT, CLOSE_SUPERSEDED, CLOSE_TRANSPORT_ERROR,
};
