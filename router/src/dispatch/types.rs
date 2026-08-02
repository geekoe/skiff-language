//! Typed dispatch DTOs (C-dispatch §7.2, C-model-request §3).

use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
};

use crate::session::identity::RuntimeSessionEpoch;

use super::candidate::{DispatchMode, ServiceDeploymentQuery};

/// Ordinary unary/stream request entering the admission pipeline.
///
/// `header` must already be codec-validated (C-model-request §3.2); the
/// dispatcher never re-parses wire bytes.
#[derive(Debug, Clone)]
pub struct DispatchRequest {
    pub header: RuntimeAssemblyRequestStartFrameHeader,
    pub payload_bytes: Vec<u8>,
    /// Selection hint. Reserved for tests and future ingress pinning;
    /// ordinary HTTP admission leaves it `None`.
    pub prefer_session: Option<RuntimeSessionEpoch>,
}

impl DispatchRequest {
    pub fn request_id(&self) -> &str {
        &self.header.request_id
    }

    /// Validated `request.start` mode. Precondition: codec-validated header.
    pub fn mode(&self) -> DispatchMode {
        DispatchMode::from_wire(&self.header.mode)
            .expect("dispatch request mode must be codec-validated")
    }

    pub fn deadline(&self) -> Option<RequestDeadline> {
        self.header.deadline.as_ref().map(RequestDeadline::from)
    }

    pub fn deployment_query(&self) -> ServiceDeploymentQuery {
        ServiceDeploymentQuery::from_deployment_ref(&self.header.routing.deployment)
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

impl From<&RuntimeAssemblyRequestDeadlineFrameHeader> for RequestDeadline {
    fn from(value: &RuntimeAssemblyRequestDeadlineFrameHeader) -> Self {
        Self {
            timeout_ms: value.timeout_ms,
            expires_at: value.expires_at.clone(),
        }
    }
}

/// Exact routing authority of a request parent (C-dispatch §5.1):
/// assembly identity, assembly generation, deployment coordinates and the
/// exact runtime connection/session that owns the parent pending.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestAuthority {
    pub assembly_identity: String,
    pub assembly_generation: u64,
    pub deployment: ServiceDeploymentQuery,
    pub session_epoch: RuntimeSessionEpoch,
}

impl RequestAuthority {
    pub fn from_header(
        header: &RuntimeAssemblyRequestStartFrameHeader,
        session_epoch: &RuntimeSessionEpoch,
    ) -> Self {
        Self {
            assembly_identity: header.routing.assembly_identity.as_str().to_string(),
            assembly_generation: header.routing.assembly_generation,
            deployment: ServiceDeploymentQuery::from_deployment_ref(&header.routing.deployment),
            session_epoch: session_epoch.clone(),
        }
    }
}

/// Function spawn vs actor-method spawn (C-dispatch §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpawnTargetKind {
    Function,
    ActorMethod,
}

/// `spawn.submit` entering the dispatcher (C-dispatch §5).
///
/// `authority` is captured by the caller from the spawn frame's routing plus
/// the exact current connection; it must equal the parent pending's captured
/// authority (exact parent authority, §5.1).
#[derive(Debug, Clone)]
pub struct SpawnSubmit {
    pub spawn_request_id: String,
    pub caller_request_id: String,
    pub target_kind: SpawnTargetKind,
    pub target: String,
    pub authority: RequestAuthority,
    /// Derived deadline = min(parent remaining, default derived timeout).
    /// Full remaining-time arithmetic is caller-owned; see `derived_deadline`.
    pub deadline: Option<RequestDeadline>,
}

/// Derived function-spawn accepted as a dispatcher-owned pending
/// (C-dispatch §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSpawnResult {
    pub spawn_request_id: String,
    pub parent_request_id: String,
    pub session_epoch: RuntimeSessionEpoch,
}

/// Actor-method spawn forwarded to the actor lane (C-dispatch §5.1/§5.2).
///
/// The dispatcher does not hold actor invocation pending and therefore does
/// not resolve the actor parent session; the actor lane owns that correlation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorMethodSpawnDispatch {
    pub spawn_request_id: String,
    pub caller_request_id: String,
    pub target: String,
}

/// Contract-shaped derived deadline (C-dispatch §5.2): the smaller
/// `timeout_ms` of parent and default, preserving the parent's wire
/// `expires_at`. Callers that already computed remaining time pass the result
/// directly in [`SpawnSubmit::deadline`].
pub fn derived_deadline(
    parent: Option<&RequestDeadline>,
    default: &RequestDeadline,
) -> RequestDeadline {
    match parent {
        Some(parent) => RequestDeadline {
            timeout_ms: parent.timeout_ms.min(default.timeout_ms),
            expires_at: parent.expires_at.clone(),
        },
        None => default.clone(),
    }
}
