//! Skiff Router Rust migration (PR 0b): frozen Router config parser and the
//! final listener skeleton assembled from the C-net mechanism.

pub mod activation;
pub mod artifact;
pub mod bootstrap;
pub mod config;
pub mod dispatch;
pub mod listener;
pub mod routing;
pub mod session;

pub use bootstrap::{
    ActiveRoutingEpochStore, BlockingLoader, BootstrapReadOutcome, BootstrapRunner,
    BootstrapStrictLoader, CommittedActivationBootstrapReader, RoutingEpoch,
};
pub use config::{
    load_router_config, redact_router_config, FileBackendConfig, FileBackendLocalConfig,
    FileBackendOssConfig, RouterConfig, RouterConfigError, RouterRewriteRule, ServiceDbConfig,
    TelemetryConfig, ROUTER_CONFIG_REDACTED_VALUE,
};
pub use dispatch::{
    ActorMethodSpawnControl, AdmissionCounters, AdmissionHealth, CancelFrame, CandidateQuery,
    CandidateQueryInput, DerivedSpawnResult, DispatchCapabilities, DispatchMode, DispatchRequest,
    DispatchedFrame, DispatcherHealthSnapshot, FrameOutcome, LeaseRevalidate, PendingHealth,
    PendingTerminal, Permit, PermitLedger, RegisteredSessionLease, RequestAuthority,
    RequestDeadline, RequestDispatcher, RequestOutcome, Reservation, RevalidateOutcome,
    RoutingEpochSource, RuntimeAdmissionPool, RuntimeDispatcherOptions, RuntimePeer,
    RuntimeResponseFrame, ServiceDeploymentQuery, SessionAbortControl, SpawnHealth,
    SpawnRejectReason, SpawnSubmit, SpawnSubmitResult, SpawnTargetKind, SubmitRejectReason,
    SubmitResult, TerminalHealth, TerminalSource, TimeoutCheck, WireTimeoutCheck,
};
pub use listener::{
    run_router, start_listeners, ListenerError, ListenerHandle, ListenerStartOptions,
    RouterListeners,
};
pub use routing::{
    CandidateDirectoryView, CandidateQuery, CandidateQueryError, CandidateSession,
    DispatchCapabilities, DispatchMode, RegisteredSessionLease, RoutingQueryCounters,
    RuntimeCandidateQuery, SessionCancellation,
};
pub use session::{
    ConsumerKind, ConsumerManifest, RuntimeRegistrationDirectory, RuntimeSessionEpoch,
    SessionLayer, SessionLayerError, SessionLayerOptions, TerminalKind,
};
