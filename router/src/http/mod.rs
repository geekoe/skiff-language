//! W-http: HTTP socket layer over the C-net frozen listener mechanism.
//!
//! This module owns the public HTTP socket boundary of the Rust Router:
//! trusted service/version selectors, service-scoped ingress resolution
//! against a captured `RoutingEpoch`, typed `request.start` construction and
//! raw opaque body mapping, unary/server-stream response mapping, stream
//! ordering and cumulative response ceiling, backpressure, disconnect/cancel/
//! deadline terminals, CORS preflight / service-managed CORS, platform error
//! projection and test-dispatch correlation isolation.
//!
//! W-http does not wire into `run_router` (E-bootstrap gate owns the
//! production assembly). The real boundary delivered here is real HTTP over a
//! real socket into the `HttpDispatchPort` seam; tests use the contract's
//! fake dispatcher (`fake::FakeHttpDispatcher`) and E-http later connects the
//! real Runtime dispatch chain.

pub mod cors;
pub mod dispatch;
pub mod error;
pub mod fake;
pub mod frame;
pub mod ingress;
pub mod selector;
pub mod server;
pub mod stream;

pub use dispatch::{
    CancelSignal, CancelWatch, DispatchRequest, HttpDispatchError, HttpDispatchPort,
    TestDispatchOutcome, UnaryHttpResponse,
};
pub use error::HttpError;
pub use ingress::{
    StoreHttpIngressResolver, HttpAdapterKind, HttpDispatchMode, HttpGatewaySurface,
    HttpGatewaySurfaceView, HttpIngressBinding, HttpIngressResolver,
};
pub use server::{
    start_http_gateway, GatewayUpgradeHandler, GatewayUpgradeOptions, HttpGatewayServer,
    HttpGatewayServerOptions,
};
pub use stream::{HttpStreamError, HttpStreamErrorSource, HttpStreamSink};

/// HTTP-layer health counters (plan §10; no payload/requestId exposure).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpGatewayHealth {
    pub requests: u64,
    pub unary_dispatches: u64,
    pub stream_dispatches: u64,
    pub cors_preflights: u64,
    pub service_managed_cors: u64,
    pub selector_rejects: u64,
    pub ingress_misses: u64,
    pub request_too_large: u64,
    pub response_too_large: u64,
    pub backpressure_cancels: u64,
    pub client_disconnect_cancels: u64,
    pub timeouts: u64,
    pub platform_errors: u64,
}
