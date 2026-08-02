//! Skiff Router Rust migration (PR 0b): frozen Router config parser and the
//! final listener skeleton assembled from the C-net mechanism.

pub mod config;
pub mod listener;

pub use config::{
    load_router_config, redact_router_config, FileBackendConfig, FileBackendLocalConfig,
    FileBackendOssConfig, RouterConfig, RouterConfigError, RouterRewriteRule, ServiceDbConfig,
    TelemetryConfig, ROUTER_CONFIG_REDACTED_VALUE,
};
pub use listener::{
    run_router, start_listeners, ListenerError, ListenerHandle, ListenerStartOptions,
    RouterListeners,
};
