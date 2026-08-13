//! Stateless exact candidate projection (integration-contract-v2 §1).
//!
//! Invariant: the candidate set is determined only by the directory view's
//! registration facts (registered build ids / lazy-load capability /
//! artifact root), registration revision / cancellation / dispatch
//! capabilities and the queried `{ mode, build_id }`. A cancelled session is
//! never a candidate; one query reads one complete revision per session;
//! heartbeat/health never affects eligibility; the query has no side effects.

use std::collections::{HashMap, HashSet};

use skiff_runtime_transport::protocol::RuntimeDispatchModeCapability;

use crate::session::directory::RuntimeRegistrationDirectory;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::layer::SessionRegistrationFacts;

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
/// (integration-contract-v2 §1). `registered` is the routable marker: pending
/// sessions (not yet ACKed) are never candidates. `registered_build_ids` /
/// `lazy_load` / `artifact_root` are the capabilities-refresh registration
/// facts used by the candidate rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSession {
    pub session_epoch: RuntimeSessionEpoch,
    pub registered: bool,
    pub registration_revision: u64,
    pub cancelled: bool,
    pub capabilities: DispatchCapabilities,
    pub registered_build_ids: Vec<String>,
    pub lazy_load: bool,
    pub artifact_root: Option<String>,
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
///
/// `router_artifact_root` is the router's own artifact store root: a session
/// qualifies via the lazy-load rule only when its advertised `artifact_root`
/// equals it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDirectoryView {
    pub revision: Option<u64>,
    pub router_artifact_root: Option<String>,
    pub sessions: Vec<CandidateSession>,
}

/// Query input (integration-contract-v2 §1): one dispatch mode plus the exact
/// deployment build id resolved from the request routing header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateQuery {
    pub mode: DispatchMode,
    pub build_id: String,
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
/// fail-closed signal for the admission layer. The registered tuple is
/// retired (M4): the lease carries the build-id registration facts only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSessionLease {
    pub session_epoch: RuntimeSessionEpoch,
    pub registration_revision: u64,
    pub cancellation: SessionCancellation,
    pub capabilities: DispatchCapabilities,
    pub registered_build_ids: Vec<String>,
    pub lazy_load: bool,
    pub artifact_root: Option<String>,
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
    pub excluded_build_id: u64,
}

/// Stateless exact candidate projection port (C-routing-query §5.1).
///
/// No mailbox, no capacity permit, no eligibility cache and no refresh: one
/// query is a pure function of the typed view and the `{ mode, build_id }`
/// query. A build id without any eligible session is an empty result (the
/// fail-closed `no eligible runtime` signal); there is no dedicated error.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeCandidateQuery;

impl RuntimeCandidateQuery {
    /// Frozen projection over a typed directory view.
    pub fn query(
        &self,
        view: &CandidateDirectoryView,
        query: &CandidateQuery,
    ) -> Vec<RegisteredSessionLease> {
        self.query_with_counters(view, query).0
    }

    /// Same projection, returning per-call counters for `routingQuery.*`
    /// health aggregation.
    pub fn query_with_counters(
        &self,
        view: &CandidateDirectoryView,
        query: &CandidateQuery,
    ) -> (Vec<RegisteredSessionLease>, RoutingQueryCounters) {
        let mut counters = RoutingQueryCounters {
            queries: 1,
            ..RoutingQueryCounters::default()
        };
        let leases = Self::project_sessions(view, query, &mut counters, true);
        (leases, counters)
    }

    /// Project one dispatch mode across a coherent view.
    ///
    /// One current session per replica (directory invariant); defensively
    /// de-duplicate the view so a session never projects twice.
    fn project_sessions(
        view: &CandidateDirectoryView,
        query: &CandidateQuery,
        counters: &mut RoutingQueryCounters,
        require_capability: bool,
    ) -> Vec<RegisteredSessionLease> {
        let mut leases = Vec::new();
        let mut seen = HashSet::new();
        for session in &view.sessions {
            if !seen.insert(session.session_epoch.clone()) {
                continue;
            }
            if let Some(lease) = project_session_with_capability(
                view.revision,
                &view.router_artifact_root,
                query,
                session,
                counters,
                require_capability,
            ) {
                counters.candidates_returned += 1;
                leases.push(lease);
            }
        }
        leases
    }

    /// Directory seam (W-routing-query delivery obligation 2): snapshots the
    /// real `RuntimeRegistrationDirectory` into a coherent typed view.
    ///
    /// The caller must hold the directory lock
    /// (`SessionLayer::directory_lock()`) for the whole snapshot so every
    /// record read is one critical section: replacement/transition races
    /// cannot mix an old tuple with a new revision (each `SessionRecord` is
    /// cloned atomically). `registration_facts` are supplied by the caller
    /// from the capabilities binding (dispatch modes + build ids + lazy-load
    /// facts); the W-session directory retains the facts through
    /// `record_registration_facts` write-through, and a session missing from
    /// the map gets empty capabilities and is capability-excluded (fail
    /// closed).
    ///
    /// The returned view has `revision: None` (per-session revision is
    /// current by construction; the directory exposes no global revision
    /// counter). Order is deterministic by
    /// `(replica_id, connection_generation)`; candidate ordering is not a
    /// frozen property (selection belongs to the admission pool).
    pub fn snapshot_directory_view(
        directory: &RuntimeRegistrationDirectory,
        registration_facts: &HashMap<RuntimeSessionEpoch, SessionRegistrationFacts>,
        router_artifact_root: Option<String>,
    ) -> CandidateDirectoryView {
        let mut sessions = directory
            .current_by_replica()
            .values()
            .filter_map(|session| {
                let record = directory.record(session)?;
                let facts = registration_facts.get(session).cloned().unwrap_or_default();
                Some(CandidateSession {
                    session_epoch: session.clone(),
                    registered: record.routable,
                    registration_revision: record.registration_revision,
                    cancelled: record.cancelled,
                    capabilities: facts.dispatch,
                    registered_build_ids: facts.registration.registered_build_ids,
                    lazy_load: facts.registration.lazy_load,
                    artifact_root: facts.registration.artifact_root,
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
            router_artifact_root,
            sessions,
        }
    }
}

fn build_lease(session: &CandidateSession) -> RegisteredSessionLease {
    RegisteredSessionLease {
        session_epoch: session.session_epoch.clone(),
        registration_revision: session.registration_revision,
        cancellation: SessionCancellation { cancelled: false },
        capabilities: session.capabilities,
        registered_build_ids: session.registered_build_ids.clone(),
        lazy_load: session.lazy_load,
        artifact_root: session.artifact_root.clone(),
    }
}

/// Frozen per-session projection (integration-contract-v2 §1). Checks run in
/// the order: registered → one complete revision → cancelled → build id
/// (registered in the set OR lazy-loadable from the shared artifact root) →
/// capability (when required); an excluded session increments exactly one
/// counter (the first failing rule; `registered == false` has no dedicated
/// counter in §5.6).
///
/// `require_capability == false` is only used by the empty-epoch activation
/// projection.
fn project_session_with_capability(
    view_revision: Option<u64>,
    router_artifact_root: &Option<String>,
    query: &CandidateQuery,
    session: &CandidateSession,
    counters: &mut RoutingQueryCounters,
    require_capability: bool,
) -> Option<RegisteredSessionLease> {
    if !session.registered {
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
    let build_id_eligible = session
        .registered_build_ids
        .iter()
        .any(|id| id == &query.build_id)
        || (session.lazy_load && session.artifact_root == *router_artifact_root);
    if !build_id_eligible {
        counters.excluded_build_id += 1;
        return None;
    }
    if require_capability && !session.capabilities.supports(query.mode) {
        counters.excluded_capability += 1;
        return None;
    }
    Some(build_lease(session))
}
