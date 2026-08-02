//! Skiff Router Rust migration (PR 0b): frozen Router config parser and the
//! final listener skeleton assembled from the C-net mechanism.

pub mod activation;
pub mod artifact;
pub mod bootstrap;
pub mod config;
pub mod http;
pub mod listener;
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
pub use session::{
    ConsumerKind, ConsumerManifest, RuntimeRegistrationDirectory, RuntimeSessionEpoch,
    SessionLayer, SessionLayerError, SessionLayerOptions, TerminalKind,
};
