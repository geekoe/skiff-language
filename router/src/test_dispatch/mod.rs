//! Test-dispatch control endpoint (plan §7 E-http test-dispatch isolation;
//! TS `AssemblyControlPlane.handleTestDispatch` parity).
//!
//! The runtime/control listener routes `POST /__skiff/test-dispatch` to
//! [`http::TestDispatchHttpHandler`]. The handler strictly decodes the
//! runtimeAssembly test-dispatch request, validates it against the exact
//! active assembly epoch and gateway binding, builds the canonical
//! `request.start` frame with test effects enabled, and dispatches it
//! through the production [`crate::http::dispatch::HttpDispatchPort`] seam.

pub mod http;

pub use http::{
    TestDispatchHttpHandler, TestDispatchHttpHandlerOptions, TestDispatchHttpResponse,
    TEST_DISPATCH_CONTROL_PATH, TEST_DISPATCH_REQUEST_BODY_CAP,
};
