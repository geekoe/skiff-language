//! Typed dispatch DTOs (C-dispatch §7.2, C-model-request §3).

use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_transport::protocol::{
    BytecodeRequestDeadlineFrameHeader, BytecodeRequestStartFrameHeader,
    BytecodeTaskRequestStartFrameHeader,
};

use crate::routing::DispatchMode;
use crate::session::identity::RuntimeSessionEpoch;

use super::candidate::dispatch_mode_from_wire;

/// Internal ordinary unary/stream admission envelope.
///
/// `header` must already be codec-validated (C-model-request §3.2); the
/// dispatcher never re-parses wire bytes. This is the dispatcher-internal
/// submit shape, not a public contract: the crate-root `DispatchRequest`
/// (C-dispatch §7.2 `{ header, payload_bytes, timeout, cancel_signal }`) is
/// owned by W-http, and E-http converts it into this envelope (including the
/// `prefer_session` selection hint, which ordinary HTTP admission leaves
/// `None`).
#[derive(Debug, Clone)]
pub struct DispatchSubmit {
    pub header: BytecodeRequestStartFrameHeader,
    pub payload_bytes: Vec<u8>,
    pub prefer_session: Option<RuntimeSessionEpoch>,
}

impl DispatchSubmit {
    pub fn request_id(&self) -> &str {
        &self.header.request_id
    }

    /// Validated `request.start` mode. Precondition: codec-validated header.
    pub fn mode(&self) -> DispatchMode {
        dispatch_mode_from_wire(&self.header.mode)
            .expect("dispatch request mode must be codec-validated")
    }

    pub fn deadline(&self) -> Option<RequestDeadline> {
        self.header.deadline.as_ref().map(RequestDeadline::from)
    }

    /// Exact parent authority captured at admission (C-dispatch §5.1).
    pub fn authority(&self, session_epoch: &RuntimeSessionEpoch) -> RequestAuthority {
        RequestAuthority::from_header(&self.header, session_epoch)
    }
}

/// Deadline surface kept by a pending (C-model-request §3.1).
///
/// `expires_at` stays the opaque ISO-8601 wire value; full remaining-time
/// arithmetic is caller-owned (W-http supplies the parsed deadline through
/// [`super::frame::TimeoutCheck`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDeadline {
    pub timeout_ms: u64,
    pub expires_at: String,
}

impl From<&BytecodeRequestDeadlineFrameHeader> for RequestDeadline {
    fn from(value: &BytecodeRequestDeadlineFrameHeader) -> Self {
        Self {
            timeout_ms: value.timeout_ms,
            expires_at: value.expires_at.clone(),
        }
    }
}

/// Exact routing authority of a request parent (C-dispatch §5.1): the
/// deployment build id, the deployment coordinates and the exact runtime
/// connection/session that owns the parent pending (M4: no assembly
/// identity/generation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestAuthority {
    pub build_id: String,
    pub deployment: ServiceDeploymentRef,
    pub session_epoch: RuntimeSessionEpoch,
}

impl RequestAuthority {
    pub fn from_header(
        header: &BytecodeRequestStartFrameHeader,
        session_epoch: &RuntimeSessionEpoch,
    ) -> Self {
        Self {
            build_id: header.routing.build_id.clone().unwrap_or_else(|| {
                header
                    .routing
                    .deployment
                    .deployment_artifact_identity
                    .to_string()
            }),
            deployment: header.routing.deployment.clone(),
            session_epoch: session_epoch.clone(),
        }
    }
}

/// One durable task attempt entering the ordinary request dispatcher
/// (authoritative design "Runtime Admission And Settlement"): the attempt is
/// a normal `request.start` frame whose header carries the
/// `taskAttempt` association (taskId/attemptId/leaseId). The dispatcher
/// treats it exactly like an ordinary unary admission (same pool, permit,
/// revalidation and deadline machinery) and returns the terminal to the task
/// control plane through the injected [`super::frame::TaskAttemptTerminalSink`].
#[derive(Debug, Clone)]
pub struct TaskAttemptSubmit {
    /// Fresh per-attempt transport frame (`request.start` with the task
    /// invocation shape and `taskAttempt` populated). `request_id` is not
    /// the task identity.
    pub header: BytecodeTaskRequestStartFrameHeader,
    pub payload: Vec<u8>,
    pub task_id: String,
    pub attempt_id: String,
    pub lease_id: String,
    /// Exact origin session for test-case attempts (F2a). When set the
    /// dispatcher admits only that Runtime connection and fails closed when
    /// it is not a current candidate; production attempts leave it `None`
    /// and keep location-transparent selection.
    pub prefer_session: Option<RuntimeSessionEpoch>,
}

impl TaskAttemptSubmit {
    pub fn request_id(&self) -> &str {
        &self.header.request_id
    }

    pub fn deadline(&self) -> Option<RequestDeadline> {
        self.header.deadline.as_ref().map(RequestDeadline::from)
    }

    pub fn authority(&self, session_epoch: &RuntimeSessionEpoch) -> RequestAuthority {
        RequestAuthority {
            build_id: self.header.routing.build_id.clone().unwrap_or_else(|| {
                self.header
                    .routing
                    .deployment
                    .deployment_artifact_identity
                    .to_string()
            }),
            deployment: self.header.routing.deployment.clone(),
            session_epoch: session_epoch.clone(),
        }
    }
}
