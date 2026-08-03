//! Task lane consumer: `FunctionTaskParentResolver`,
//! `ActorTaskParentResolver` and the stateless `TaskSubmitRouter`
//! (authority design §5.4/§5.5, C-task/C-model-task contracts).
//!
//! Parent correlation is strictly typed `(callerKind, callerRequestId)`:
//! `request` resolves only through the request namespace and
//! `actorInvocation` only through the actor-invocation namespace. There is
//! no string-prefix fallback and no compatible reader for the legacy shape
//! (missing `callerKind`); the canonical task codec rejects that shape and
//! the router counts the fail-closed rejection. The router stores no
//! parent-child mapping: an accepted task is handed to its execution owner
//! and outlives parent terminal/replacement.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::protocol::{
    TaskCallerKind, TaskSubmitRequestFrameHeaderV2, TaskTargetKind,
};

use crate::dispatch::{ActorMethodTaskControl, ActorMethodTaskDispatch};

use super::health::TaskHealth;
use super::invocation::ActorInvocationRelay;

/// Closed task error vocabulary (C-task §3.3; resolver-level
/// `TestCapabilityMismatch` is included so authority drift always fails
/// closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskErrorCode {
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

impl TaskErrorCode {
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

/// One task rejection with a closed error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSubmitError {
    code: TaskErrorCode,
}

impl TaskSubmitError {
    pub fn new(code: TaskErrorCode) -> Self {
        Self { code }
    }

    pub fn code(&self) -> TaskErrorCode {
        self.code
    }
}

impl fmt::Display for TaskSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code.as_str())
    }
}

impl std::error::Error for TaskSubmitError {}

/// Fenced parent snapshot returned by a parent lookup port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskParentSnapshot {
    pub runtime_id: String,
    pub connection: String,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
    pub active: bool,
    pub replaced: bool,
}

/// Exact parent-namespace lookup port. Implementations never own pending:
/// `RequestDispatcher` / `ActorInvocationRelay` answer from their maps.
pub trait TaskParentLookup: Send + Sync + fmt::Debug {
    fn find_parent(&self, caller_request_id: &str) -> Option<TaskParentSnapshot>;
}

/// Typed parent query (C-task §3).
#[derive(Debug, Clone)]
pub struct ParentQuery {
    pub caller_request_id: String,
    pub connection: String,
    pub runtime_id: Option<String>,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Resolved parent authority (C-task §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskParentAuthority {
    pub runtime_id: String,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Exact parent resolution (C-task §3.1/§3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskParentResolution {
    pub caller_kind: TaskCallerKind,
    pub parent_request_id: String,
    pub authority: TaskParentAuthority,
    pub origin_runtime_connection: String,
}

/// Captured authority probe for one task submit (exact connection +
/// captured routing authority).
#[derive(Debug, Clone)]
pub struct TaskAuthorityProbe {
    pub connection: String,
    pub runtime_id: Option<String>,
    pub assembly_generation: u64,
    pub test_case_capability: Option<String>,
}

/// Accepted task handed to the execution owner (C-task §3.3/§4.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSubmitAcceptance {
    pub task_id: String,
    pub request_id: String,
    pub caller_kind: TaskCallerKind,
    pub target_kind: TaskTargetKind,
    pub parent_request_id: String,
}

/// `callerKind=request` parent resolver (C-task §4.1).
#[derive(Debug)]
pub struct FunctionTaskParentResolver {
    lookup: Arc<dyn TaskParentLookup>,
}

impl FunctionTaskParentResolver {
    pub fn new(lookup: Arc<dyn TaskParentLookup>) -> Self {
        Self { lookup }
    }
}

/// `callerKind=actorInvocation` parent resolver (C-task §4.2).
#[derive(Debug)]
pub struct ActorTaskParentResolver {
    lookup: Arc<dyn TaskParentLookup>,
}

impl ActorTaskParentResolver {
    pub fn new(lookup: Arc<dyn TaskParentLookup>) -> Self {
        Self { lookup }
    }
}

/// Production `ActorTaskParentLookup` adapter: answers exact fenced
/// snapshots from the `ActorInvocationRelay` pending map (C-task §4.2).
#[derive(Debug)]
pub struct RelayTaskParentLookup {
    relay: Arc<ActorInvocationRelay>,
}

impl RelayTaskParentLookup {
    pub fn new(relay: Arc<ActorInvocationRelay>) -> Self {
        Self { relay }
    }
}

impl TaskParentLookup for RelayTaskParentLookup {
    fn find_parent(&self, caller_request_id: &str) -> Option<TaskParentSnapshot> {
        self.relay.parent_snapshot(caller_request_id)
    }
}

fn resolve_parent(
    lookup: &dyn TaskParentLookup,
    query: &ParentQuery,
    caller_kind: TaskCallerKind,
) -> Result<TaskParentResolution, TaskSubmitError> {
    let Some(parent) = lookup.find_parent(&query.caller_request_id) else {
        return Err(TaskSubmitError::new(TaskErrorCode::ParentNotFound));
    };
    if !parent.active {
        return Err(TaskSubmitError::new(TaskErrorCode::ParentTerminal));
    }
    if parent.replaced {
        return Err(TaskSubmitError::new(TaskErrorCode::ParentReplaced));
    }
    if parent.connection != query.connection {
        return Err(TaskSubmitError::new(
            TaskErrorCode::ParentConnectionMismatch,
        ));
    }
    if let Some(runtime_id) = &query.runtime_id {
        if runtime_id != &parent.runtime_id {
            return Err(TaskSubmitError::new(
                TaskErrorCode::ParentConnectionMismatch,
            ));
        }
    }
    if parent.test_case_capability.is_some()
        && parent.test_case_capability != query.test_case_capability
    {
        return Err(TaskSubmitError::new(
            TaskErrorCode::TestCapabilityMismatch,
        ));
    }
    if parent.assembly_generation != query.assembly_generation {
        return Err(TaskSubmitError::new(TaskErrorCode::AuthorityMismatch));
    }
    Ok(TaskParentResolution {
        caller_kind,
        parent_request_id: query.caller_request_id.clone(),
        authority: TaskParentAuthority {
            runtime_id: parent.runtime_id,
            assembly_generation: parent.assembly_generation,
            test_case_capability: parent.test_case_capability,
        },
        origin_runtime_connection: parent.connection,
    })
}

impl FunctionTaskParentResolver {
    pub fn resolve(&self, query: &ParentQuery) -> Result<TaskParentResolution, TaskSubmitError> {
        resolve_parent(self.lookup.as_ref(), query, TaskCallerKind::Request)
    }
}

impl ActorTaskParentResolver {
    pub fn resolve(&self, query: &ParentQuery) -> Result<TaskParentResolution, TaskSubmitError> {
        resolve_parent(
            self.lookup.as_ref(),
            query,
            TaskCallerKind::ActorInvocation,
        )
    }
}

#[derive(Debug, Default)]
struct TaskSubmitCounters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    legacy_rejected: AtomicU64,
    request_accepted: AtomicU64,
    actor_invocation_accepted: AtomicU64,
    next_seq: AtomicU64,
    by_error: Mutex<BTreeMap<TaskErrorCode, u64>>,
}

/// Stateless task submit sink (C-task §4.3): exact parent-kind selection,
/// target-kind classification, acceptance minting, shared capacity counter.
#[derive(Debug)]
pub struct TaskSubmitRouter {
    request_resolver: Arc<FunctionTaskParentResolver>,
    actor_resolver: Arc<ActorTaskParentResolver>,
    capacity: Arc<AtomicUsize>,
    capacity_limit: usize,
    counters: Arc<TaskSubmitCounters>,
}

impl TaskSubmitRouter {
    pub fn new(
        request_resolver: Arc<FunctionTaskParentResolver>,
        actor_resolver: Arc<ActorTaskParentResolver>,
        capacity_limit: usize,
    ) -> Result<Self, String> {
        if capacity_limit < 1 {
            return Err("task capacityLimit must be >= 1".to_string());
        }
        Ok(Self {
            request_resolver,
            actor_resolver,
            capacity: Arc::new(AtomicUsize::new(0)),
            capacity_limit,
            counters: Arc::new(TaskSubmitCounters::default()),
        })
    }

    /// Canonical entry: consumes a codec-validated
    /// `task.submit.request` header plus the exact captured authority.
    pub fn submit(
        &self,
        header: &TaskSubmitRequestFrameHeaderV2,
        authority: &TaskAuthorityProbe,
    ) -> Result<TaskSubmitAcceptance, TaskSubmitError> {
        let query = ParentQuery {
            caller_request_id: header.caller_request_id.clone(),
            connection: authority.connection.clone(),
            runtime_id: Some(header.runtime_id.clone()).or_else(|| authority.runtime_id.clone()),
            assembly_generation: authority.assembly_generation,
            test_case_capability: authority.test_case_capability.clone(),
        };
        let resolution = match header.caller_kind {
            TaskCallerKind::Request => self.request_resolver.resolve(&query),
            TaskCallerKind::ActorInvocation => self.actor_resolver.resolve(&query),
        }
        .map_err(|error| self.record_rejection(error.code()))?;
        match header.target_kind {
            TaskTargetKind::Function => {
                if header.actor_method.is_some() {
                    return Err(self.record_rejection(TaskErrorCode::TargetKindMismatch));
                }
            }
            TaskTargetKind::ActorMethod => {
                if header.actor_method.is_none() {
                    return Err(self.record_rejection(TaskErrorCode::TargetKindMismatch));
                }
            }
        }
        self.accept(
            header.caller_kind,
            header.target_kind,
            header.task_id.clone(),
            &resolution.parent_request_id,
        )
    }

    /// Actor-lane seam entry (W-dispatch `ActorMethodTaskControl`): the
    /// parent kind is fixed to `actorInvocation` by the dispatcher forwarding
    /// path; exact parent + authority validation still happens here.
    pub fn submit_actor_method(
        &self,
        dispatch: &ActorMethodTaskDispatch,
        authority: &TaskAuthorityProbe,
    ) -> Result<TaskSubmitAcceptance, TaskSubmitError> {
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
            TaskCallerKind::ActorInvocation,
            TaskTargetKind::ActorMethod,
            Some(dispatch.task_request_id.clone()),
            &resolution.parent_request_id,
        )
    }

    /// Legacy-cut fail-closed rejection observed at the sink boundary
    /// (missing/invalid `callerKind`; the canonical codec already rejects the
    /// frame with no compatible reader).
    pub fn reject_legacy(&self, _caller_request_id: &str) -> TaskSubmitError {
        self.counters
            .legacy_rejected
            .fetch_add(1, Ordering::Relaxed);
        self.record_rejection(TaskErrorCode::CallerKindRejected)
    }

    /// Releases one accepted task's shared capacity (execution owner).
    pub fn release_accepted(&self) {
        self.capacity.fetch_sub(1, Ordering::Relaxed);
    }

    fn accept(
        &self,
        caller_kind: TaskCallerKind,
        target_kind: TaskTargetKind,
        task_id: Option<String>,
        parent_request_id: &str,
    ) -> Result<TaskSubmitAcceptance, TaskSubmitError> {
        if self.capacity.fetch_add(1, Ordering::Relaxed) >= self.capacity_limit {
            self.capacity.fetch_sub(1, Ordering::Relaxed);
            return Err(self.record_rejection(TaskErrorCode::Saturated));
        }
        let seq = self.counters.next_seq.fetch_add(1, Ordering::Relaxed);
        let acceptance = TaskSubmitAcceptance {
            task_id: task_id.unwrap_or_else(|| format!("task-{seq}")),
            request_id: format!("task-request-{seq}"),
            caller_kind,
            target_kind,
            parent_request_id: parent_request_id.to_string(),
        };
        self.counters.accepted.fetch_add(1, Ordering::Relaxed);
        match caller_kind {
            TaskCallerKind::Request => {
                self.counters
                    .request_accepted
                    .fetch_add(1, Ordering::Relaxed);
            }
            TaskCallerKind::ActorInvocation => {
                self.counters
                    .actor_invocation_accepted
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(acceptance)
    }

    fn record_rejection(&self, code: TaskErrorCode) -> TaskSubmitError {
        self.counters.rejected.fetch_add(1, Ordering::Relaxed);
        let mut by_error = self
            .counters
            .by_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *by_error.entry(code).or_insert(0) += 1;
        TaskSubmitError::new(code)
    }

    pub fn health(&self) -> TaskHealth {
        let by_error = self
            .counters
            .by_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(code, count)| (code.as_str().to_string(), *count))
            .collect();
        TaskHealth {
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

/// Execution owner of accepted actor-method tasks (E-actor-rust wires the
/// real execution; W-actor only hands the acceptance over).
pub trait ActorMethodTaskExecutionSink: Send + Sync + fmt::Debug {
    fn on_accept(&self, acceptance: &TaskSubmitAcceptance);
}

/// W-dispatch actor lane seam implementation: answers exact
/// actorInvocation parent liveness from the relay and routes accepted
/// actor-method tasks through the stateless router.
#[derive(Debug)]
pub struct ActorLaneTaskControl {
    relay: Arc<ActorInvocationRelay>,
    router: Arc<TaskSubmitRouter>,
    execution: Arc<dyn ActorMethodTaskExecutionSink>,
}

impl ActorLaneTaskControl {
    pub fn new(
        relay: Arc<ActorInvocationRelay>,
        router: Arc<TaskSubmitRouter>,
        execution: Arc<dyn ActorMethodTaskExecutionSink>,
    ) -> Self {
        Self {
            relay,
            router,
            execution,
        }
    }
}

impl ActorMethodTaskControl for ActorLaneTaskControl {
    fn is_active_invocation_parent(&self, caller_request_id: &str) -> bool {
        self.relay.is_active_parent(caller_request_id)
    }

    fn submit_task(&self, task: ActorMethodTaskDispatch) {
        let Some(parent) = self.relay.parent_snapshot(&task.caller_request_id) else {
            let _ = self.router.record_rejection(TaskErrorCode::ParentTerminal);
            return;
        };
        let authority = TaskAuthorityProbe {
            connection: parent.connection.clone(),
            runtime_id: Some(parent.runtime_id.clone()),
            assembly_generation: parent.assembly_generation,
            test_case_capability: parent.test_case_capability.clone(),
        };
        if let Ok(acceptance) = self.router.submit_actor_method(&task, &authority) {
            self.execution.on_accept(&acceptance);
        }
    }
}
