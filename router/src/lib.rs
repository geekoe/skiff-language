//! Skiff Router Rust migration (PR 0b): frozen Router config parser and the
//! final listener skeleton assembled from the C-net mechanism.

pub mod activation;
pub mod actor;
pub mod artifact;
pub mod bootstrap;
pub mod config;
pub mod dispatch;
pub mod health;
pub mod http;
pub mod listener;
pub mod routing;
pub mod session;
pub mod supervisor;
pub mod task;
pub mod telemetry;
pub mod test_dispatch;
pub mod ws;

pub use activation::{
    ActivationCoordinator, ActivationCoordinatorHandle, ActivationCoordinatorHealth,
    ActivationCoordinatorOptions, ActivationCoordinatorPorts, ActivationHttpHandler,
    ActivationParticipantBinding, ActivationPhase, ActivationRevalidateOutcome, BlockingLoaderPort,
    CandidateEpochRefs, CoordinatorError, EnqueueResult, PublishCommittedEpochPort,
    RecoveryTransaction, RuntimeCandidateQueryPort, SessionEnqueuePort,
    ACTIVATION_REQUEST_BODY_CAP, ASSEMBLY_ACTIVATION_CONTROL_PATH,
};
pub use actor::{
    ActorActivationRequestBroker, ActorHealthSnapshot, ActorInvocationRelay,
    ActorLeaseExpiryScheduler, ActorMethodCatalogView, ActorOwnerControlBroker,
    ActorOwnershipRegistry,
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
    dispatch_mode_from_wire, AdmissionCounters, AdmissionHealth, CancelFrame, CandidateViewSource,
    DispatchedFrame, DispatcherHealthSnapshot, FrameOutcome, LeaseRevalidate,
    NoopTaskAttemptTerminalSink, PendingHealth, PendingTerminal, Permit, PermitLedger,
    RequestAuthority, RequestDeadline, RequestDispatcher, RequestOutcome, Reservation,
    RevalidateOutcome, RoutingEpochSource, RuntimeAdmissionPool, RuntimeDispatcherOptions,
    RuntimePeer, RuntimeResponseFrame, SessionAbortControl, SubmitRejectReason, SubmitResult,
    TaskAttemptSubmit, TaskAttemptSubmitResult, TaskAttemptTerminalOutcome,
    TaskAttemptTerminalSink, TaskHealth, TerminalHealth, TerminalSource, TimeoutCheck,
    WireTimeoutCheck,
};
pub use health::{
    project_capability_connections, project_loop_risk_runtimes, project_replicas, render_base,
    session_facts, ActiveAssemblyProjection, CapabilitiesProjection,
    CapabilityConnectionProjection, HealthAggregator, HealthCounters, LoopRiskDispatcherProjection,
    LoopRiskHttpStreamProjection, LoopRiskProjection, LoopRiskRouterProjection,
    LoopRiskRuntimeProjection, ReplicaProjection, SessionFacts,
};
pub use http::{
    CancelSignal, CancelWatch, DispatchRequest, EpochHttpIngressResolver, HttpAdapterKind,
    HttpDispatchError, HttpDispatchMode, HttpDispatchPort, HttpError, HttpGatewayHealth,
    HttpGatewayServer, HttpGatewayServerOptions, HttpGatewaySurface, HttpGatewaySurfaceView,
    HttpIngressBinding, HttpIngressResolver, HttpStreamError, HttpStreamErrorSource,
    HttpStreamSink, TestDispatchOutcome, UnaryHttpResponse,
};
pub use listener::{
    run_router, start_listeners, ClientWsContext, ListenerError, ListenerHandle,
    ListenerStartOptions, RouterListeners, WsTaskRegistry,
};
pub use routing::{
    CandidateDirectoryView, CandidateQuery, CandidateSession, DispatchCapabilities, DispatchMode,
    RegisteredSessionLease, RoutingQueryCounters, RuntimeCandidateQuery, SessionCancellation,
};
pub use session::{
    ConsumerKind, ConsumerManifest, RegistrationObserver, RuntimeRegistrationDirectory,
    RuntimeSessionEpoch, SessionLayer, SessionLayerError, SessionLayerOptions, TerminalKind,
};
pub use supervisor::{RouterComponents, RouterSupervisor, SupervisorError, SupervisorListeners};
pub use telemetry::{
    router_telemetry_event, task_event, telemetry_timestamp_now, NoopTaskTelemetrySink,
    RouterTelemetryExporter, RouterTelemetryExporterHandle, RouterTelemetryProducer,
    TaskTelemetrySink, EXPORTER_SHUTDOWN_FLUSH_TIMEOUT,
};
pub use test_dispatch::{
    TestDispatchHttpHandler, TestDispatchHttpHandlerOptions, TestDispatchHttpResponse,
    TEST_DISPATCH_CONTROL_PATH, TEST_DISPATCH_REQUEST_BODY_CAP,
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
