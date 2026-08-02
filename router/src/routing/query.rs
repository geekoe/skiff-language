//! Stateless exact candidate projection (C-routing-query §3/§5).
//!
//! Invariant: the candidate set is determined only by the captured epoch and
//! the directory view's exact registered tuple / registration revision /
//! cancellation / capabilities. A cancelled session is never a candidate; one
//! query reads one complete revision per session; heartbeat/health never
//! affects eligibility; the query has no side effects.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_transport::protocol::RuntimeDispatchModeCapability;

use crate::bootstrap::RoutingEpoch;
use crate::session::directory::RuntimeRegistrationDirectory;
use crate::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};

/// Dispatch mode of an ordinary request (C-routing-query §2.1/§3 rule 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DispatchMode {
    Unary,
    ServerStream,
}

/// Per-session dispatch capabilities projected from the runtime capabilities
/// binding (`runtime.capabilities.capabilities.dispatchModes`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchCapabilities {
    pub unary: bool,
    pub server_stream: bool,
}

impl DispatchCapabilities {
    /// Builds the query-side projection from the wire dispatch-mode list.
    pub fn from_dispatch_modes(
        modes: impl IntoIterator<Item = RuntimeDispatchModeCapability>,
    ) -> Self {
        let mut capabilities = Self::default();
        for mode in modes {
            match mode {
                RuntimeDispatchModeCapability::Unary => capabilities.unary = true,
                RuntimeDispatchModeCapability::ServerStream => {
                    capabilities.server_stream = true;
                }
            }
        }
        capabilities
    }

    /// Frozen capability rule (C-routing-query §3 rule 5): unary requires
    /// `unary`, serverStream requires `server_stream`.
    pub fn supports(self, mode: DispatchMode) -> bool {
        match mode {
            DispatchMode::Unary => self.unary,
            DispatchMode::ServerStream => self.server_stream,
        }
    }
}

/// One session's exact facts in the typed directory view
/// (C-routing-query §2.1). `registered` is the routable marker: pending
/// sessions (not yet ACKed) are never candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSession {
    pub session_epoch: RuntimeSessionEpoch,
    pub registered: bool,
    pub registered_tuple: Option<RegisteredAssemblyTuple>,
    pub registration_revision: u64,
    pub cancelled: bool,
    pub capabilities: DispatchCapabilities,
}

/// One coherent directory view (C-routing-query §2.1/§4).
///
/// `revision` is the frozen view-level revision marker (`directoryRevision`):
/// when `Some(n)`, a session whose `registration_revision != n` is a torn /
/// stale revision and is never a candidate. `None` marks a lock-held
/// production snapshot (`RuntimeCandidateQuery::snapshot_directory_view`):
/// the W-session directory does not expose a global revision counter, so a
/// snapshot taken under the directory lock is coherent by construction and
/// each session's own revision is current (one complete tuple+revision per
/// record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDirectoryView {
    pub revision: Option<u64>,
    pub sessions: Vec<CandidateSession>,
}

/// Query input: one dispatch mode plus the exact deployment resolved from the
/// captured epoch's deployment projection (C-routing-query §2.1/§3 rule 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateQuery {
    pub mode: DispatchMode,
    pub deployment: ServiceDeploymentRef,
}

/// Typed cancellation projection carried by a lease.
///
/// The stateless candidate query never holds (or creates) the real per-session
/// cancellation token (C-routing-query §2.2: "candidate 不持有真 token");
/// W-dispatch wires the actual token/barrier cancellation from the session
/// layer. `cancelled` is always false for a projected lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCancellation {
    pub cancelled: bool,
}

/// Exact candidate lease (C-routing-query §2.2). Empty results are the
/// fail-closed signal for the admission layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSessionLease {
    pub session_epoch: RuntimeSessionEpoch,
    pub registration_revision: u64,
    pub exact_registered_tuple: RegisteredAssemblyTuple,
    pub cancellation: SessionCancellation,
    pub capabilities: DispatchCapabilities,
}

/// Per-query projection counters (C-routing-query §5.6 `routingQuery.*`).
/// Callers aggregate them into health; no payload/tuple content is exposed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoutingQueryCounters {
    pub queries: u64,
    pub candidates_returned: u64,
    pub excluded_cancelled: u64,
    pub excluded_stale_revision: u64,
    pub excluded_capability: u64,
    pub excluded_tuple_mismatch: u64,
}

/// Fail-closed query errors (C-routing-query §5.4). No partial projection is
/// ever returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CandidateQueryError {
    #[error("query deployment is not an exact deployment of the captured routing epoch")]
    DeploymentNotInEpoch,
}

/// Stateless exact candidate projection port (C-routing-query §5.1).
///
/// No mailbox, no capacity permit, no eligibility cache and no refresh: one
/// query is a pure function of the captured epoch and the typed view.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeCandidateQuery;

impl RuntimeCandidateQuery {
    /// Frozen projection over a typed directory view.
    pub fn query(
        &self,
        epoch: &Arc<RoutingEpoch>,
        view: &CandidateDirectoryView,
        query: &CandidateQuery,
    ) -> Result<Vec<RegisteredSessionLease>, CandidateQueryError> {
        Ok(self.query_with_counters(epoch, view, query)?.0)
    }

    /// Same projection, returning per-call counters for `routingQuery.*`
    /// health aggregation.
    pub fn query_with_counters(
        &self,
        epoch: &Arc<RoutingEpoch>,
        view: &CandidateDirectoryView,
        query: &CandidateQuery,
    ) -> Result<(Vec<RegisteredSessionLease>, RoutingQueryCounters), CandidateQueryError> {
        if !epoch.deployment_projection().contains(&query.deployment) {
            return Err(CandidateQueryError::DeploymentNotInEpoch);
        }

        let mut counters = RoutingQueryCounters {
            queries: 1,
            ..RoutingQueryCounters::default()
        };
        let expected_tuple = epoch.registered_tuple();
        let mut leases = Vec::new();
        // One current session per replica (directory invariant); defensively
        // de-duplicate the view so a session never projects twice.
        let mut seen = HashSet::new();
        for session in &view.sessions {
            if !seen.insert(session.session_epoch.clone()) {
                continue;
            }
            if let Some(lease) = project_session(
                &expected_tuple,
                view.revision,
                query.mode,
                session,
                &mut counters,
            ) {
                counters.candidates_returned += 1;
                leases.push(lease);
            }
        }
        Ok((leases, counters))
    }

    /// Directory seam (W-routing-query delivery obligation 2): snapshots the
    /// real `RuntimeRegistrationDirectory` into a coherent typed view.
    ///
    /// The caller must hold the directory lock
    /// (`SessionLayer::directory_lock()`) for the whole snapshot so every
    /// record read is one critical section: replacement/transition races
    /// cannot mix an old tuple with a new revision (each `SessionRecord` is
    /// cloned atomically). `capabilities` are supplied by the caller from the
    /// capabilities binding (`dispatchModes`); the W-session directory does
    /// not retain them. A session missing from the map gets empty
    /// capabilities and is capability-excluded (fail closed).
    ///
    /// The returned view has `revision: None` (per-session revision is
    /// current by construction; the directory exposes no global revision
    /// counter). Order is deterministic by
    /// `(replica_id, connection_generation)`; candidate ordering is not a
    /// frozen property (selection belongs to the admission pool).
    pub fn snapshot_directory_view(
        directory: &RuntimeRegistrationDirectory,
        capabilities: &HashMap<RuntimeSessionEpoch, DispatchCapabilities>,
    ) -> CandidateDirectoryView {
        let mut sessions = directory
            .current_by_replica()
            .values()
            .filter_map(|session| {
                let record = directory.record(session)?;
                Some(CandidateSession {
                    session_epoch: session.clone(),
                    registered: record.routable,
                    registered_tuple: record.registered_tuple.clone(),
                    registration_revision: record.registration_revision,
                    cancelled: record.cancelled,
                    capabilities: capabilities.get(session).copied().unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            left.session_epoch
                .replica_id
                .cmp(&right.session_epoch.replica_id)
                .then(
                    left.session_epoch
                        .connection_generation
                        .cmp(&right.session_epoch.connection_generation),
                )
        });
        CandidateDirectoryView {
            revision: None,
            sessions,
        }
    }
}

/// Frozen per-session projection (C-routing-query §3). Checks run in the
/// order: registered → exact tuple → one complete revision → cancelled →
/// capability; an excluded session increments exactly one counter (the first
/// failing rule; `registered == false` has no dedicated counter in §5.6).
fn project_session(
    expected_tuple: &RegisteredAssemblyTuple,
    view_revision: Option<u64>,
    mode: DispatchMode,
    session: &CandidateSession,
    counters: &mut RoutingQueryCounters,
) -> Option<RegisteredSessionLease> {
    if !session.registered {
        return None;
    }
    if session.registered_tuple.as_ref() != Some(expected_tuple) {
        counters.excluded_tuple_mismatch += 1;
        return None;
    }
    if view_revision.is_some_and(|revision| session.registration_revision != revision) {
        counters.excluded_stale_revision += 1;
        return None;
    }
    if session.cancelled {
        counters.excluded_cancelled += 1;
        return None;
    }
    if !session.capabilities.supports(mode) {
        counters.excluded_capability += 1;
        return None;
    }
    Some(RegisteredSessionLease {
        session_epoch: session.session_epoch.clone(),
        registration_revision: session.registration_revision,
        exact_registered_tuple: session
            .registered_tuple
            .clone()
            .expect("exact tuple checked above"),
        cancellation: SessionCancellation { cancelled: false },
        capabilities: session.capabilities,
    })
}
