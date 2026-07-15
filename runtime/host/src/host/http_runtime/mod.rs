#![allow(dead_code)]

mod call_context;
mod cancel;
mod egress;
mod input;
mod request;
mod response;
mod response_parts;
mod sse;
mod stream;
mod transport;

pub const HTTP_REQUEST_TIMEOUT_REASON: &str = "request timeout";

#[allow(unused_imports)]
pub use request::request;
pub(crate) use request::request_with_cancellation_and_options;
pub(crate) use sse::open_sse_with_cancellation_and_options;
#[allow(unused_imports)]
pub use sse::{open_sse_with_cancel_flags, sse};
pub(crate) use stream::open_body_stream_with_cancellation_and_options;
#[allow(unused_imports)]
pub use stream::{open_stream_with_cancel_flags, stream, HttpBodyStream, HttpEventStream};

#[cfg(test)]
mod tests;
