//! Spawn lane consumer: `FunctionSpawnParentResolver`,
//! `ActorSpawnParentResolver` and the stateless `SpawnSubmitRouter`
//! (authority design §5.4/§5.5, C-spawn/C-model-spawn contracts).
//!
//! Parent correlation is strictly typed `(callerKind, callerRequestId)`:
//! `request` resolves only through the request namespace and
//! `actorInvocation` only through the actor-invocation namespace. There is
//! no string-prefix fallback and no compatible reader for the legacy shape
//! (missing `callerKind`); the canonical spawn codec rejects that shape and
//! the router counts the fail-closed rejection. The router stores no
//! parent-child mapping: an accepted spawn is handed to its execution owner
//! and outlives parent terminal/replacement.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::protocol::{
    SpawnCallerKind, SpawnSubmitRequestFrameHeaderV2, SpawnTargetKind,
};

use crate::dispatch::{ActorMethodSpawnControl, ActorMethodSpawnDispatch};

use super::health::SpawnHealth;
use super::invocation::ActorInvocationRelay;

/// Closed spawn error vocabulary (C-spawn §3.3; resolver-level
/// `TestCapabilityMismatch` is included so authority drift always fails
/// closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SpawnErrorCode {
    ParentNotFound,
    ParentTerminal,
    ParentReplaced,
    ParentConnectionMismatch,
    CallerKindRejected,
    TargetKindMismatch,
    AuthorityMismatch,
    TestCapabilityMismatch,
    Saturated,
    UnknownTarget,
}

impl SpawnErrorCode {
    pub const ALL: [Self; 10] = [
        Self::ParentNotFound,
        Self::ParentTerminal,
        Self::ParentReplaced,
        Self::ParentConnectionMismatch,
        Self::CallerKindRejected,
        Self::TargetKindMismatch,
        Self::AuthorityMismatch,
        Self::TestCapabilityMismatch,
        Self::Saturated,
        Self::UnknownTarget,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentNotFound => "ParentNotFound",
            Self::ParentTerminal => "ParentTerminal",
            Self::ParentReplaced => "ParentReplaced",
            Self::ParentConnectionMismatch => "ParentConnectionMismatch",
            Self::CallerKindRejected => "CallerKindRejected",
            Self::TargetKindMismatch => "TargetKindMismatch",
            Self::AuthorityMismatch => "AuthorityMismatch",
            Self::TestCapabilityMismatch => "TestCapabilityMismatch",
            Self::Saturated => "Saturated",
            Self::UnknownTarget => "UnknownTarget",
        }
    }
}

/// One spawn rejection with a closed error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSubmitError {
    code: SpawnErrorCode,
}

impl SpawnSubmitError {
    pub fn new(code: SpawnErrorCode) -> Self {
        Self { code }
    }

    pub fn code(&self) -> SpawnErrorCode {
        self.code
    }
}

impl fmt::Display for SpawnSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code.as_str())
    }
}

impl std::error::Error for SpawnSubmitError {}

/// Fenced parent snapshot returned by a parent lookup port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnParentSnapshot {
    pub runtime_id: String,
    pub connection: String,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
    pub active: bool,
    pub replaced: bool,
}

/// Exact parent-namespace lookup port. Implementations never own pending:
/// `RequestDispatcher` / `ActorInvocationRelay` answer from their maps.
pub trait SpawnParentLookup: Send + Sync + fmt::Debug {
    fn find_parent(&self, caller_request_id: &str) -> Option<SpawnParentSnapshot>;
}

/// Typed parent query (C-spawn §3).
#[derive(Debug, Clone)]
pub struct ParentQuery {
    pub caller_request_id: String,
    pub connection: String,
    pub runtime_id: Option<String>,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Resolved parent authority (C-spawn §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnParentAuthority {
    pub runtime_id: String,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Exact parent resolution (C-spawn §3.1/§3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnParentResolution {
    pub caller_kind: SpawnCallerKind,
    pub parent_request_id: String,
    pub authority: SpawnParentAuthority,
    pub origin_runtime_connection: String,
}

/// Captured authority probe for one spawn submit (exact connection +
/// captured routing authority).
#[derive(Debug, Clone)]
pub struct SpawnAuthorityProbe {
    pub connection: String,
    pub runtime_id: Option<String>,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Accepted spawn handed to the execution owner (C-spawn §3.3/§4.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSubmitAcceptance {
    pub spawn_id: String,
    pub request_id: String,
    pub caller_kind: SpawnCallerKind,
    pub target_kind: SpawnTargetKind,
    pub parent_request_id: String,
}

/// `callerKind=request` parent resolver (C-spawn §4.1).
#[derive(Debug)]
pub struct FunctionSpawnParentResolver {
    lookup: Arc<dyn SpawnParentLookup>,
}

impl FunctionSpawnParentResolver {
    pub fn new(lookup: Arc<dyn SpawnParentLookup>) -> Self {
        Self { lookup }
    }
}

/// `callerKind=actorInvocation` parent resolver (C-spawn §4.2).
#[derive(Debug)]
pub struct ActorSpawnParentResolver {
    lookup: Arc<dyn SpawnParentLookup>,
}

impl ActorSpawnParentResolver {
    pub fn new(lookup: Arc<dyn SpawnParentLookup>) -> Self {
        Self { lookup }
    }
}

/// Production `ActorSpawnParentLookup` adapter: answers exact fenced
/// snapshots from the `ActorInvocationRelay` pending map (C-spawn §4.2).
#[derive(Debug)]
pub struct RelaySpawnParentLookup {
    relay: Arc<ActorInvocationRelay>,
}

impl RelaySpawnParentLookup {
    pub fn new(relay: Arc<ActorInvocationRelay>) -> Self {
        Self { relay }
    }
}

impl SpawnParentLookup for RelaySpawnParentLookup {
    fn find_parent(&self, caller_request_id: &str) -> Option<SpawnParentSnapshot> {
        self.relay.parent_snapshot(caller_request_id)
    }
}

fn resolve_parent(
    lookup: &dyn SpawnParentLookup,
    query: &ParentQuery,
    caller_kind: SpawnCallerKind,
) -> Result<SpawnParentResolution, SpawnSubmitError> {
    let Some(parent) = lookup.find_parent(&query.caller_request_id) else {
        return Err(SpawnSubmitError::new(SpawnErrorCode::ParentNotFound));
    };
    if !parent.active {
        return Err(SpawnSubmitError::new(SpawnErrorCode::ParentTerminal));
    }
    if parent.replaced {
        return Err(SpawnSubmitError::new(SpawnErrorCode::ParentReplaced));
    }
    if parent.connection != query.connection {
        return Err(SpawnSubmitError::new(
            SpawnErrorCode::ParentConnectionMismatch,
        ));
    }
    if let Some(runtime_id) = &query.runtime_id {
        if runtime_id != &parent.runtime_id {
            return Err(SpawnSubmitError::new(
                SpawnErrorCode::ParentConnectionMismatch,
            ));
        }
    }
    if parent.test_case_capability.is_some()
        && parent.test_case_capability != query.test_case_capability
    {
        return Err(SpawnSubmitError::new(
            SpawnErrorCode::TestCapabilityMismatch,
        ));
    }
    if parent.assembly_generation != query.assembly_generation {
        return Err(SpawnSubmitError::new(SpawnErrorCode::AuthorityMismatch));
    }
    Ok(SpawnParentResolution {
        caller_kind,
        parent_request_id: query.caller_request_id.clone(),
        authority: SpawnParentAuthority {
            runtime_id: parent.runtime_id,
            assembly_generation: parent.assembly_generation,
            test_case_capability: parent.test_case_capability,
        },
        origin_runtime_connection: parent.connection,
    })
}

impl FunctionSpawnParentResolver {
    pub fn resolve(&self, query: &ParentQuery) -> Result<SpawnParentResolution, SpawnSubmitError> {
        resolve_parent(self.lookup.as_ref(), query, SpawnCallerKind::Request)
    }
}

impl ActorSpawnParentResolver {
    pub fn resolve(&self, query: &ParentQuery) -> Result<SpawnParentResolution, SpawnSubmitError> {
        resolve_parent(
            self.lookup.as_ref(),
            query,
            SpawnCallerKind::ActorInvocation,
        )
    }
}

#[derive(Debug, Default)]
struct SpawnCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    legacy_rejected: AtomicU64,
    request_accepted: AtomicU64,
    actor_invocation_accepted: AtomicU64,
    next_seq: AtomicU64,
    by_error: Mutex<BTreeMap<SpawnErrorCode, u64>>,
}

/// Stateless spawn submit sink (C-spawn §4.3): exact parent-kind selection,
/// target-kind classification, acceptance minting, shared capacity counter.
#[derive(Debug)]
pub struct SpawnSubmitRouter {
    request_resolver: Arc<FunctionSpawnParentResolver>,
    actor_resolver: Arc<ActorSpawnParentResolver>,
    capacity: Arc<AtomicUsize>,
    capacity_limit: usize,
    counters: Arc<SpawnCounters>,
}

impl SpawnSubmitRouter {
    pub fn new(
        request_resolver: Arc<FunctionSpawnParentResolver>,
        actor_resolver: Arc<ActorSpawnParentResolver>,
        capacity_limit: usize,
    ) -> Result<Self, String> {
        if capacity_limit < 1 {
            return Err("spawn capacityLimit must be >= 1".to_string());
        }
        Ok(Self {
            request_resolver,
            actor_resolver,
            capacity: Arc::new(AtomicUsize::new(0)),
            capacity_limit,
            counters: Arc::new(SpawnCounters::default()),
        })
    }

    /// Canonical entry: consumes a codec-validated
    /// `spawn.submit.request` header plus the exact captured authority.
    pub fn submit(
        &self,
        header: &SpawnSubmitRequestFrameHeaderV2,
        authority: &SpawnAuthorityProbe,
    ) -> Result<SpawnSubmitAcceptance, SpawnSubmitError> {
        let query = ParentQuery {
            caller_request_id: header.caller_request_id.clone(),
            connection: authority.connection.clone(),
            runtime_id: Some(header.runtime_id.clone()).or_else(|| authority.runtime_id.clone()),
            assembly_generation: authority.assembly_generation,
            test_case_capability: authority.test_case_capability.clone(),
        };
        let resolution = match header.caller_kind {
            SpawnCallerKind::Request => self.request_resolver.resolve(&query),
            SpawnCallerKind::ActorInvocation => self.actor_resolver.resolve(&query),
        }
        .map_err(|error| self.record_rejection(error.code()))?;
        match header.target_kind {
            SpawnTargetKind::Function => {
                if header.actor_method.is_some() {
                    return Err(self.record_rejection(SpawnErrorCode::TargetKindMismatch));
                }
            }
            SpawnTargetKind::ActorMethod => {
                if header.actor_method.is_none() {
                    return Err(self.record_rejection(SpawnErrorCode::TargetKindMismatch));
                }
            }
        }
        self.accept(
            header.caller_kind,
            header.target_kind,
            header.spawn_id.clone(),
            &resolution.parent_request_id,
        )
    }

    /// Actor-lane seam entry (W-dispatch `ActorMethodSpawnControl`): the
    /// parent kind is fixed to `actorInvocation` by the dispatcher forwarding
    /// path; exact parent + authority validation still happens here.
    pub fn submit_actor_method(
        &self,
        dispatch: &ActorMethodSpawnDispatch,
        authority: &SpawnAuthorityProbe,
    ) -> Result<SpawnSubmitAcceptance, SpawnSubmitError> {
        let query = ParentQuery {
            caller_request_id: dispatch.caller_request_id.clone(),
            connection: authority.connection.clone(),
            runtime_id: authority.runtime_id.clone(),
            assembly_generation: authority.assembly_generation,
            test_case_capability: authority.test_case_capability.clone(),
        };
        let resolution = self
            .actor_resolver
            .resolve(&query)
            .map_err(|error| self.record_rejection(error.code()))?;
        self.accept(
            SpawnCallerKind::ActorInvocation,
            SpawnTargetKind::ActorMethod,
            Some(dispatch.spawn_request_id.clone()),
            &resolution.parent_request_id,
        )
    }

    /// Legacy-cut fail-closed rejection observed at the sink boundary
    /// (missing/invalid `callerKind`; the canonical codec already rejects the
    /// frame with no compatible reader).
    pub fn reject_legacy(&self, _caller_request_id: &str) -> SpawnSubmitError {
        self.counters
            .legacy_rejected
            .fetch_add(1, Ordering::Relaxed);
        self.record_rejection(SpawnErrorCode::CallerKindRejected)
    }

    /// Releases one accepted spawn's shared capacity (execution owner).
    pub fn release_accepted(&self) {
        self.capacity.fetch_sub(1, Ordering::Relaxed);
    }

    fn accept(
        &self,
        caller_kind: SpawnCallerKind,
        target_kind: SpawnTargetKind,
        spawn_id: Option<String>,
        parent_request_id: &str,
    ) -> Result<SpawnSubmitAcceptance, SpawnSubmitError> {
        if self.capacity.fetch_add(1, Ordering::Relaxed) >= self.capacity_limit {
            self.capacity.fetch_sub(1, Ordering::Relaxed);
            return Err(self.record_rejection(SpawnErrorCode::Saturated));
        }
        let seq = self.counters.next_seq.fetch_add(1, Ordering::Relaxed);
        let acceptance = SpawnSubmitAcceptance {
            spawn_id: spawn_id.unwrap_or_else(|| format!("spawn-{seq}")),
            request_id: format!("spawn-request-{seq}"),
            caller_kind,
            target_kind,
            parent_request_id: parent_request_id.to_string(),
        };
        self.counters.accepted.fetch_add(1, Ordering::Relaxed);
        match caller_kind {
            SpawnCallerKind::Request => {
                self.counters
                    .request_accepted
                    .fetch_add(1, Ordering::Relaxed);
            }
            SpawnCallerKind::ActorInvocation => {
                self.counters
                    .actor_invocation_accepted
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(acceptance)
    }

    fn record_rejection(&self, code: SpawnErrorCode) -> SpawnSubmitError {
        self.counters.rejected.fetch_add(1, Ordering::Relaxed);
        let mut by_error = self
            .counters
            .by_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *by_error.entry(code).or_insert(0) += 1;
        SpawnSubmitError::new(code)
    }

    pub fn health(&self) -> SpawnHealth {
        let by_error = self
            .counters
            .by_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(code, count)| (code.as_str().to_string(), *count))
            .collect();
        SpawnHealth {
            capacity_in_use: self.capacity.load(Ordering::Relaxed),
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            rejected: self.counters.rejected.load(Ordering::Relaxed),
            legacy_rejected: self.counters.legacy_rejected.load(Ordering::Relaxed),
            request_accepted: self.counters.request_accepted.load(Ordering::Relaxed),
            actor_invocation_accepted: self
                .counters
                .actor_invocation_accepted
                .load(Ordering::Relaxed),
            by_error,
        }
    }
}

/// Execution owner of accepted actor-method spawns (E-actor-rust wires the
/// real execution; W-actor only hands the acceptance over).
pub trait ActorMethodSpawnExecutionSink: Send + Sync + fmt::Debug {
    fn on_accept(&self, acceptance: &SpawnSubmitAcceptance);
}

/// W-dispatch actor lane seam implementation: answers exact
/// actorInvocation parent liveness from the relay and routes accepted
/// actor-method spawns through the stateless router.
#[derive(Debug)]
pub struct ActorLaneSpawnControl {
    relay: Arc<ActorInvocationRelay>,
    router: Arc<SpawnSubmitRouter>,
    execution: Arc<dyn ActorMethodSpawnExecutionSink>,
}

impl ActorLaneSpawnControl {
    pub fn new(
        relay: Arc<ActorInvocationRelay>,
        router: Arc<SpawnSubmitRouter>,
        execution: Arc<dyn ActorMethodSpawnExecutionSink>,
    ) -> Self {
        Self {
            relay,
            router,
            execution,
        }
    }
}

impl ActorMethodSpawnControl for ActorLaneSpawnControl {
    fn is_active_invocation_parent(&self, caller_request_id: &str) -> bool {
        self.relay.is_active_parent(caller_request_id)
    }

    fn submit_spawn(&self, spawn: ActorMethodSpawnDispatch) {
        let Some(parent) = self.relay.parent_snapshot(&spawn.caller_request_id) else {
            let _ = self.router.record_rejection(SpawnErrorCode::ParentTerminal);
            return;
        };
        let authority = SpawnAuthorityProbe {
            connection: parent.connection.clone(),
            runtime_id: Some(parent.runtime_id.clone()),
            assembly_generation: parent.assembly_generation,
            test_case_capability: parent.test_case_capability.clone(),
        };
        if let Ok(acceptance) = self.router.submit_actor_method(&spawn, &authority) {
            self.execution.on_accept(&acceptance);
        }
    }
}
