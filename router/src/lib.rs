//! Skiff Router Rust migration (PR 0b): frozen Router config parser and the
//! final listener skeleton assembled from the C-net mechanism.

pub mod activation;
pub mod actor;
pub mod artifact;
pub mod bootstrap;
pub mod config;
pub mod dispatch;
pub mod http;
pub mod listener;
pub mod routing;
pub mod session;
pub mod ws;

pub use actor::{
    ActorActivationRequestBroker, ActorHealthSnapshot, ActorInvocationRelay, ActorLaneSpawnControl,
    ActorLeaseExpiryScheduler, ActorMethodCatalogView, ActorMethodSpawnExecutionSink,
    ActorOwnerControlBroker, ActorOwnershipRegistry, ActorSpawnParentResolver,
    FunctionSpawnParentResolver, RelaySpawnParentLookup, SpawnSubmitRouter,
};
pub use bootstrap::{
    ActiveRoutingEpochStore, BlockingLoader, BootstrapReadOutcome, BootstrapRunner,
    BootstrapStrictLoader, CommittedActivationBootstrapReader, RouterBootstrapAssembly,
    RoutingEpoch, ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};
pub use config::{
    load_router_config, redact_router_config, FileBackendConfig, FileBackendLocalConfig,
    FileBackendOssConfig, RouterConfig, RouterConfigError, RouterRewriteRule, ServiceDbConfig,
    TelemetryConfig, ROUTER_CONFIG_REDACTED_VALUE,
};
pub use dispatch::{
    candidate_query_from_request, capabilities_from_wire_names, dispatch_mode_as_str,
    dispatch_mode_from_wire, ActorMethodSpawnControl, AdmissionCounters, AdmissionHealth,
    CancelFrame, CandidateViewSource, DerivedSpawnResult, DispatchedFrame,
    DispatcherHealthSnapshot, FrameOutcome, LeaseRevalidate, PendingHealth, PendingTerminal,
    Permit, PermitLedger, RequestAuthority, RequestDeadline, RequestDispatcher, RequestOutcome,
    Reservation, RevalidateOutcome, RoutingEpochSource, RuntimeAdmissionPool,
    RuntimeDispatcherOptions, RuntimePeer, RuntimeResponseFrame, SessionAbortControl, SpawnHealth,
    SpawnRejectReason, SpawnSubmit, SpawnSubmitResult, SpawnTargetKind, SubmitRejectReason,
    SubmitResult, TerminalHealth, TerminalSource, TimeoutCheck, WireTimeoutCheck,
};
pub use http::{
    CancelSignal, CancelWatch, DispatchRequest, EpochHttpIngressResolver, HttpAdapterKind,
    HttpDispatchError, HttpDispatchMode, HttpDispatchPort, HttpError, HttpGatewayHealth,
    HttpGatewayServer, HttpGatewayServerOptions, HttpGatewaySurface, HttpGatewaySurfaceView,
    HttpIngressBinding, HttpIngressResolver, HttpStreamError, HttpStreamErrorSource,
    HttpStreamSink, UnaryHttpResponse,
};
pub use listener::{
    run_router, start_listeners, ListenerError, ListenerHandle, ListenerStartOptions,
    RouterListeners,
};
pub use activation::{
    ActivationCoordinator, ActivationCoordinatorHandle, ActivationCoordinatorHealth,
    ActivationCoordinatorOptions, ActivationCoordinatorPorts, ActivationParticipantBinding,
    ActivationPhase, ActivationRevalidateOutcome, BlockingLoaderPort, CandidateEpochRefs,
    CoordinatorError, EnqueueResult, PublishCommittedEpochPort, RecoveryTransaction,
    RuntimeCandidateQueryPort, SessionEnqueuePort,
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
pub use ws::{
    AllowAnyPendingAdmission, BrokerConnectionGeneration, BrokerGenerationAdapter,
    BrokerGenerationPort, BrokerHealthSnapshot, BrokerRuntimeResponse, BrokerRuntimeSource,
    BusinessKey, ClientConnectionIndex, ClientConnectionIndexOptions, ClientTerminal, Clock,
    DispatchInbound, InboundDispatchAction, InboundDispatchResult, InboundExecutionToken,
    JsonRpc20TextProfile, LedgerReleaseAdapter, LedgerReleasePort, MethodCatalog,
    NoopNotificationObserver, NoopRuntimeViolationSink, NotificationObserver, OverflowPolicy,
    PeerResponseTerminal, PeerTextOutcome, PeerWriter, PendingAdmissionSender,
    PendingReleaseHandle, PlatformErrorKind, ProfileLimits, ReleaseOutcome, ReleaseResolution,
    RuntimeGenerationPeer, RuntimeGenerationPinLedger, RuntimeRequest, RuntimeRequestOutcome,
    RuntimeResponder, RuntimeSessionClose, RuntimeViolationSink, WebSocketLane,
    WebSocketLaneOptions, WebSocketLifecycleClose, WebSocketRequestBroker,
    WebSocketRequestBrokerOptions, WriteBudget, WsHealthSnapshot,
};
