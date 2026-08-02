//! Typed candidate/lease ports consumed by W-dispatch
//! (C-routing-query §2/§5, plan §3.3 steps 1-5).
//!
//! W-routing-query (same wave, `router/src/routing/`) owns the stateless
//! exact candidate projection. This module freezes the dispatch-side typed
//! port exactly as the contracts-request chain defines it: a whole captured
//! [`crate::bootstrap::RoutingEpoch`] plus a directory view project to
//! [`RegisteredSessionLease`] candidates. The corpus harness drives a fake
//! implementation; integration aligns the real W-routing-query port to this
//! seam without changing dispatch semantics.

use std::fmt;
use std::sync::Arc;

use skiff_artifact_model::ServiceDeploymentRef;

use crate::bootstrap::RoutingEpoch;
use crate::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};

/// Ordinary dispatch mode (C-routing-query §2.1, C-model-request §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DispatchMode {
    Unary,
    ServerStream,
}

impl DispatchMode {
    /// Wire `request.start` mode. The codec already rejects other values;
    /// this stays a fallible mapping so the dispatcher can fail closed.
    pub fn from_wire(mode: &str) -> Option<Self> {
        match mode {
            "unary" => Some(Self::Unary),
            "serverStream" => Some(Self::ServerStream),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unary => "unary",
            Self::ServerStream => "serverStream",
        }
    }
}

/// Dispatch capability bitmap (C-routing-query §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DispatchCapabilities {
    pub unary: bool,
    pub server_stream: bool,
}

impl DispatchCapabilities {
    /// Builds from a wire capability name list (`"unary"` / `"serverStream"`).
    pub fn from_wire(names: &[String]) -> Self {
        Self {
            unary: names.iter().any(|name| name == "unary"),
            server_stream: names.iter().any(|name| name == "serverStream"),
        }
    }

    pub fn supports(&self, mode: DispatchMode) -> bool {
        match mode {
            DispatchMode::Unary => self.unary,
            DispatchMode::ServerStream => self.server_stream,
        }
    }
}

/// Exact deployment coordinates of one query (C-routing-query §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceDeploymentQuery {
    pub service_id: String,
    pub contract_version: String,
    pub deployment_revision: String,
    pub deployment_artifact_identity: String,
}

impl ServiceDeploymentQuery {
    pub fn from_deployment_ref(deployment: &ServiceDeploymentRef) -> Self {
        Self {
            service_id: deployment.service_id.clone(),
            contract_version: deployment.contract_version.clone(),
            deployment_revision: deployment.deployment_revision.as_str().to_string(),
            deployment_artifact_identity: deployment
                .deployment_artifact_identity
                .as_str()
                .to_string(),
        }
    }
}

/// Inputs of one candidate query (C-routing-query §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateQueryInput {
    pub mode: DispatchMode,
    pub deployment: ServiceDeploymentQuery,
}

/// Typed candidate lease returned by the projection (C-routing-query §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSessionLease {
    pub session_epoch: RuntimeSessionEpoch,
    pub registration_revision: u64,
    pub exact_registered_tuple: RegisteredAssemblyTuple,
    /// Snapshot at query time; the enqueue path revalidates this atomically
    /// through [`LeaseRevalidate`].
    pub cancelled: bool,
    pub capabilities: DispatchCapabilities,
}

/// Whole-epoch capture + exact candidate projection port
/// (plan §3.3 steps 1-3, C-routing-query §5.1).
///
/// W-routing-query seam: the real implementation reads one complete revision
/// of the `RuntimeRegistrationDirectory` and never mixes epochs. Queries are
/// synchronous and side-effect free.
pub trait CandidateQuery: Send + Sync + fmt::Debug {
    fn query(
        &self,
        epoch: &RoutingEpoch,
        query: &CandidateQueryInput,
    ) -> Vec<RegisteredSessionLease>;
}

/// Atomic revalidation immediately before enqueue (plan §3.3 step 5).
///
/// Re-checks session epoch, registration revision, exact registered tuple and
/// cancellation against the directory. `request_id` is passed for fake
/// injection; real implementations ignore it.
pub trait LeaseRevalidate: Send + Sync + fmt::Debug {
    fn revalidate(&self, request_id: &str, lease: &RegisteredSessionLease) -> RevalidateOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevalidateOutcome {
    Ok,
    Cancelled,
    StaleRevision,
    TupleMismatch,
}

/// Captured routing epoch source (plan §3.3 step 1).
///
/// W-bootstrap seam: the production implementation wraps
/// `Arc<ActiveRoutingEpochStore>` and captures the current whole epoch.
/// `None` means no epoch published yet: admission fails closed.
pub trait RoutingEpochSource: Send + Sync + fmt::Debug {
    fn capture(&self) -> Option<Arc<RoutingEpoch>>;
}
