use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    json::RuntimeBoundaryCodec,
    plan::BoundaryUse,
    request_heap::{deep_clone_runtime_values_between_heaps, RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, HeapNode, InterfaceCarrier, RuntimeValue},
};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedFileUnit,
    UnitAddr,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

use crate::error::RuntimeError;

pub const ACTOR_BOOTSTRAP_ENCODING_V1: &str = "skiff-canonical-v1";

/// Canonical registry identity excluding the incarnation epoch.
///
/// `canonical_actor_id_key_bytes` is already decoded from the registry wire
/// field. Keeping the canonical bytes in the key makes equality independent of
/// a diagnostic hash implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorLogicalKey {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes: Vec<u8>,
    pub actor_id_hash: String,
}

impl ActorLogicalKey {
    fn validate(&self) -> Result<(), ActorInstanceStoreError> {
        for (label, value) in [
            ("serviceId", self.service_id.as_str()),
            ("actorTypeIdentity", self.actor_type_identity.as_str()),
            ("actorIdTypeIdentity", self.actor_id_type_identity.as_str()),
            (
                "actorIdEncodingVersion",
                self.actor_id_encoding_version.as_str(),
            ),
            ("actorIdHash", self.actor_id_hash.as_str()),
        ] {
            if value.is_empty() {
                return Err(ActorInstanceStoreError::InvalidLogicalKey {
                    message: format!("{label} must be non-empty"),
                });
            }
        }
        if self.canonical_actor_id_key_bytes.is_empty() {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "canonical actor id key bytes must be non-empty".to_string(),
            });
        }
        if self.actor_id_encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: format!(
                    "unsupported actor id encoding {}",
                    self.actor_id_encoding_version
                ),
            });
        }
        let actor_id: Value =
            serde_json::from_slice(&self.canonical_actor_id_key_bytes).map_err(|error| {
                ActorInstanceStoreError::InvalidLogicalKey {
                    message: format!("actor id key bytes are not JSON: {error}"),
                }
            })?;
        let canonical = canonical_json_bytes(&actor_id).map_err(|error| {
            ActorInstanceStoreError::InvalidLogicalKey {
                message: format!("actor id key cannot be canonicalized: {error}"),
            }
        })?;
        if canonical != self.canonical_actor_id_key_bytes {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "actor id key bytes are not canonical JSON".to_string(),
            });
        }
        let expected_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(&self.canonical_actor_id_key_bytes))
        );
        if self.actor_id_hash != expected_hash {
            return Err(ActorInstanceStoreError::InvalidLogicalKey {
                message: "actor id hash does not match canonical key bytes".to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorIncarnationKey {
    pub logical_key: ActorLogicalKey,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInstanceFence {
    pub incarnation: ActorIncarnationKey,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: LinkedActorDeclarationOwner,
}

impl ActorInstanceFence {
    fn validate(&self) -> Result<(), ActorInstanceStoreError> {
        self.incarnation.logical_key.validate()?;
        if self.incarnation.epoch == 0 {
            return Err(ActorInstanceStoreError::InvalidEpoch);
        }
        Ok(())
    }
}

pub struct ActorActivationRequest<'a> {
    pub fence: ActorInstanceFence,
    pub bootstrap_encoding_version: &'a str,
    pub bootstrap_payload: &'a [u8],
    pub program: ProgramTypeView<'a>,
}

pub(crate) enum ActorActivation {
    Existing(ActorInstanceHandle),
    Materialized(ActorMaterialization),
}

pub(crate) enum ActorMaterialization {
    Store(MaterializedActorActivation),
    Session(SessionMaterializedActorActivation),
}

impl ActorMaterialization {
    pub(crate) fn handle(&self) -> &ActorInstanceHandle {
        match self {
            Self::Store(activation) => activation.handle(),
            Self::Session(activation) => activation.handle(),
        }
    }

    pub(crate) fn admit(
        self,
        authority: &ActorExecutorAuthority,
    ) -> Result<ActorInstanceHandle, ActorInstanceStoreError> {
        match self {
            Self::Store(activation) => activation.admit(authority),
            Self::Session(activation) => activation.admit(authority),
        }
    }
}

/// The only production result that can expose a newly inserted, unadmitted
/// instance. Dropping it removes that exact Arc and wakes admission observers.
pub(crate) struct MaterializedActorActivation {
    store: ActorInstanceStore,
    handle: Option<ActorInstanceHandle>,
}

impl MaterializedActorActivation {
    pub(crate) fn handle(&self) -> &ActorInstanceHandle {
        self.handle
            .as_ref()
            .expect("materialized Actor activation handle is present until admission")
    }

    pub(crate) fn admit(
        mut self,
        authority: &ActorExecutorAuthority,
    ) -> Result<ActorInstanceHandle, ActorInstanceStoreError> {
        let handle = self
            .handle
            .as_ref()
            .expect("materialized Actor activation handle is present until admission");
        self.store.mark_admitted(authority, handle)?;
        Ok(self
            .handle
            .take()
            .expect("successful Actor admission consumes the guarded handle"))
    }

    fn take_for_session(mut self) -> ActorInstanceHandle {
        self.handle
            .take()
            .expect("session claim consumes the guarded store handle")
    }
}

impl Drop for MaterializedActorActivation {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.store.discard_exact(&handle);
        }
    }
}

pub(crate) struct SessionMaterializedActorActivation {
    tracker: Arc<ActorInstanceSessionTracker>,
    session: ActorInstanceSessionLease,
    handle: Option<ActorInstanceHandle>,
}

impl SessionMaterializedActorActivation {
    fn handle(&self) -> &ActorInstanceHandle {
        self.handle
            .as_ref()
            .expect("session Actor materialization is present until admission")
    }

    fn admit(
        mut self,
        authority: &ActorExecutorAuthority,
    ) -> Result<ActorInstanceHandle, ActorInstanceStoreError> {
        let handle = self
            .handle
            .as_ref()
            .expect("session Actor materialization is present until admission");
        self.tracker
            .commit_session_activation(&self.session, authority, handle)?;
        Ok(self
            .handle
            .take()
            .expect("successful session Actor admission consumes the guarded handle"))
    }
}

impl Drop for SessionMaterializedActorActivation {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.tracker
                .discard_provisional_exact(&self.session, &handle);
        }
    }
}

/// Opaque executor-facing identity. It intentionally exposes no field values.
#[derive(Debug, Clone)]
pub struct ActorInstanceHandle {
    fence: ActorInstanceFence,
    instance: Arc<ActorInstance>,
}

impl ActorInstanceHandle {
    pub fn fence(&self) -> &ActorInstanceFence {
        &self.fence
    }

    /// Canonical create input (bootstrap payload bytes) frozen when this
    /// incarnation was materialized. This is the submission-side copy of the
    /// registry entry's creation inputs; it deliberately contains no Actor
    /// memory fields.
    pub(crate) fn activation_create_input(&self) -> &[u8] {
        &self.instance.activation_facts.create_input
    }

    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.instance, &other.instance)
    }
}

/// Immutable activation facts retained by one live incarnation for actor-method
/// task submission (authoritative design Actor-method target). Only the
/// recoverable `create` input is kept; memory fields are never snapshotted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActorActivationFacts {
    pub(crate) create_input: Vec<u8>,
}

#[derive(Debug)]
struct ActorInstance {
    fence: ActorInstanceFence,
    activation_facts: ActorActivationFacts,
    state: Mutex<ActorInstanceState>,
    next_execution_token: AtomicU64,
    upgrading: AtomicBool,
    admitted: AtomicBool,
    admission_notify: tokio::sync::Notify,
    #[cfg(test)]
    admission_wait_before_poll_action: Mutex<Option<AdmissionWaitBeforePollTestAction>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
enum AdmissionWaitBeforePollTestAction {
    Admit,
    Discard,
}

/// The shared arena backing one Actor instance.
///
/// The arena is the instance scheduler: exactly one segment holds the guard at
/// a time, and the guard must never survive a `Pending` poll (see
/// `ActorExecutionFrame::await_if_pending`). Field roots in `ActorInstanceState`
/// are handles into this arena.
pub(crate) type SharedArena = Arc<tokio::sync::Mutex<RequestHeap>>;

#[derive(Debug)]
struct ActorInstanceState {
    fields: Vec<ActorFieldValue>,
    arena: SharedArena,
    /// Epoch stamped into every handle allocated by `arena`. Compaction
    /// replaces the arena with a fresh heap at epoch + 1 so stale handles fail
    /// closed in `RequestHeap::slot`/`slot_mut`.
    arena_epoch: u32,
    /// Per-instance arena limits (nodes / bytes / depth).
    limits: RequestHeapLimits,
    /// Active segments currently holding (or about to hold) the arena guard.
    active_segments: usize,
    /// Continuations parked at a real suspension point.
    suspended_segments: usize,
    /// Discard was requested while a segment was in flight; the instance is
    /// reclaimed once the last segment ends.
    pending_discard: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActorFieldValue {
    pub name: String,
    pub value: RuntimeValue,
    /// Platform-written key fields and fields assigned by `create` are
    /// admitted. Fields still awaiting `create` assignment are not readable.
    pub assigned: bool,
}

impl ActorFieldValue {
    pub(crate) fn assigned(name: impl Into<String>, value: RuntimeValue) -> Self {
        Self {
            name: name.into(),
            value,
            assigned: true,
        }
    }

    pub(crate) fn unassigned(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: RuntimeValue::Null,
            assigned: false,
        }
    }
}

/// Capability token reserved for the Actor executor in this crate.
///
/// Host request paths cannot construct this token and an instance handle does
/// not itself grant field access.
pub(crate) struct ActorExecutorAuthority(());

impl ActorExecutorAuthority {
    pub(crate) fn new() -> Self {
        Self(())
    }
}

/// One actor method segment.
///
/// Acquisition locks the instance arena, revalidates the instance fence and
/// arena epoch, and increments the active continuation counter. Suspension
/// moves the segment to the suspended counter; resume revalidates and moves it
/// back. Commit and abandon release the segment with no fields/heap clone:
/// field mutations were already applied directly to the shared arena.
pub(crate) struct SegmentLease {
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
    token: Arc<ActorExecutionToken>,
    arena: SharedArena,
    guard: Option<tokio::sync::OwnedMutexGuard<RequestHeap>>,
    expected_epoch: u32,
    state: SegmentState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SegmentState {
    Active,
    Suspended,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SegmentStateError {
    AlreadyFinished,
    NotSuspended,
    AlreadySuspended,
}

impl std::fmt::Display for SegmentStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyFinished => formatter.write_str("Actor segment already finished"),
            Self::NotSuspended => formatter.write_str("Actor segment is not suspended"),
            Self::AlreadySuspended => formatter.write_str("Actor segment is already suspended"),
        }
    }
}

pub(crate) struct ActorExecutionToken {
    nonce: u64,
    active: AtomicBool,
}

impl ActorExecutionToken {
    pub(crate) fn ensure_active(&self) -> Result<(), ActorInstanceStoreError> {
        if self.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ActorInstanceStoreError::ExecutionTokenExpired)
        }
    }
}

impl Drop for SegmentLease {
    fn drop(&mut self) {
        if self.state == SegmentState::Finished {
            return;
        }
        let store = self.store.clone();
        let handle = self.handle.clone();
        store.abandon_segment(&handle, self);
    }
}

impl SegmentLease {
    pub(crate) fn instance_identity(&self) -> usize {
        Arc::as_ptr(&self.handle.instance) as usize
    }

    pub(crate) fn token(&self) -> Arc<ActorExecutionToken> {
        Arc::clone(&self.token)
    }

    pub(crate) fn arena(&self) -> &SharedArena {
        &self.arena
    }

    pub(crate) fn expected_epoch(&self) -> u32 {
        self.expected_epoch
    }

    pub(crate) fn is_suspended(&self) -> bool {
        self.state == SegmentState::Suspended
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.state == SegmentState::Finished
    }

    pub(crate) fn take_guard(&mut self) -> tokio::sync::OwnedMutexGuard<RequestHeap> {
        self.guard
            .take()
            .expect("Actor segment arena guard may only be taken once")
    }

    pub(crate) fn heap_mut(&mut self) -> &mut RequestHeap {
        self.guard
            .as_mut()
            .expect("Actor segment arena guard is present until taken")
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActorInstanceStore {
    state: Arc<Mutex<ActorInstanceStoreState>>,
    /// Limits applied to arenas of instances materialized by this store.
    /// Test-only overrides make the arena-limits and compaction thresholds
    /// observable without changing the production default.
    pub(crate) arena_limits: RequestHeapLimits,
}

#[derive(Debug, Default)]
struct ActorInstanceStoreState {
    instances: HashMap<ActorIncarnationKey, Arc<ActorInstance>>,
    latest_epochs: HashMap<ActorLogicalKey, u64>,
}

/// Connection-scoped ownership of live Actor instances in one Runtime process.
#[derive(Debug)]
pub struct ActorInstanceSessionTracker {
    store: Arc<ActorInstanceStore>,
    state: Mutex<ActorInstanceSessionTrackerState>,
}

#[derive(Debug, Default)]
struct ActorInstanceSessionTrackerState {
    open_sessions: HashMap<String, Arc<ActorInstanceSessionState>>,
    by_session: HashMap<String, Vec<ActorInstanceHandle>>,
    handle_owners: HashMap<usize, ActorInstanceOwner>,
}

#[derive(Debug)]
struct ActorInstanceOwner {
    session_id: String,
    session: Weak<ActorInstanceSessionState>,
    instance: Weak<ActorInstance>,
}

impl ActorInstanceOwner {
    fn matches(
        &self,
        session_id: &str,
        session: &Arc<ActorInstanceSessionState>,
        instance: &Arc<ActorInstance>,
    ) -> bool {
        self.session_id == session_id
            && self
                .session
                .upgrade()
                .is_some_and(|owner_session| Arc::ptr_eq(&owner_session, session))
            && self
                .instance
                .upgrade()
                .is_some_and(|owner_instance| Arc::ptr_eq(&owner_instance, instance))
    }
}

#[derive(Debug)]
struct ActorInstanceSessionState {
    open: AtomicBool,
    closed: tokio::sync::Notify,
}

/// Opaque capability for one exact Router connection generation.
#[derive(Debug, Clone)]
pub struct ActorInstanceSessionLease {
    router_session_id: String,
    state: Arc<ActorInstanceSessionState>,
}

impl ActorInstanceSessionLease {
    pub fn router_session_id(&self) -> &str {
        &self.router_session_id
    }

    pub async fn wait_closed(&self) {
        loop {
            let notified = self.state.closed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.state.open.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

impl ActorInstanceSessionTracker {
    pub fn new(store: Arc<ActorInstanceStore>) -> Self {
        Self {
            store,
            state: Mutex::new(ActorInstanceSessionTrackerState::default()),
        }
    }

    pub fn store(&self) -> &Arc<ActorInstanceStore> {
        &self.store
    }

    /// Opens the ownership scope for one Router connection.
    ///
    /// Router session ids are generated once per WebSocket connection. The open-session set is
    /// intentionally independent from tracked handles: evicting the last Actor must not close the
    /// connection, while absence means an unknown or already-closed connection cannot publish a
    /// late Actor instance.
    pub fn open_session(
        &self,
        router_session_id: &str,
    ) -> Result<(), ActorInstanceSessionTrackError> {
        if router_session_id.is_empty() {
            return Err(ActorInstanceSessionTrackError::EmptySessionId);
        }
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        if state.open_sessions.contains_key(router_session_id) {
            return Err(ActorInstanceSessionTrackError::SessionAlreadyOpen {
                router_session_id: router_session_id.to_string(),
            });
        }
        state.open_sessions.insert(
            router_session_id.to_string(),
            Arc::new(ActorInstanceSessionState {
                open: AtomicBool::new(true),
                closed: tokio::sync::Notify::new(),
            }),
        );
        Ok(())
    }

    pub fn session_lease(
        &self,
        router_session_id: &str,
    ) -> Result<ActorInstanceSessionLease, ActorInstanceSessionTrackError> {
        let state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let session = state.open_sessions.get(router_session_id).ok_or_else(|| {
            ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            }
        })?;
        Ok(ActorInstanceSessionLease {
            router_session_id: router_session_id.to_string(),
            state: Arc::clone(session),
        })
    }

    pub(crate) fn begin_activation(
        self: &Arc<Self>,
        session: &ActorInstanceSessionLease,
        request: ActorActivationRequest<'_>,
    ) -> Result<ActorActivation, ActorInstanceSessionTrackError> {
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let router_session_id = session.router_session_id();
        let Some(current_session) = state.open_sessions.get(router_session_id).cloned() else {
            return Err(ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            });
        };
        if !Arc::ptr_eq(&current_session, &session.state)
            || !session.state.open.load(Ordering::Acquire)
        {
            return Err(ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            });
        }
        let activation = self.store.begin_activation(request).map_err(|error| {
            ActorInstanceSessionTrackError::ActivationFailed {
                message: error.to_string(),
            }
        })?;
        let handle = match &activation {
            ActorActivation::Existing(handle) => handle,
            ActorActivation::Materialized(materialized) => materialized.handle(),
        };
        let identity = Arc::as_ptr(&handle.instance) as usize;
        if let Some(owner) = state.handle_owners.get(&identity) {
            if owner
                .instance
                .upgrade()
                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &handle.instance))
            {
                if owner.matches(router_session_id, &current_session, &handle.instance) {
                    return Ok(activation);
                }
                return Err(ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id: owner.session_id.clone(),
                });
            }
        }
        if matches!(&activation, ActorActivation::Existing(_)) {
            return Err(ActorInstanceSessionTrackError::NotPublishable {
                message: "existing Actor instance has no session owner".to_string(),
            });
        }
        state.handle_owners.insert(
            identity,
            ActorInstanceOwner {
                session_id: router_session_id.to_string(),
                session: Arc::downgrade(&current_session),
                instance: Arc::downgrade(&handle.instance),
            },
        );
        state
            .by_session
            .entry(router_session_id.to_string())
            .or_default()
            .push(handle.clone());
        let ActorActivation::Materialized(ActorMaterialization::Store(store_activation)) =
            activation
        else {
            unreachable!("new session activation must hold the store materialization guard")
        };
        let handle = Some(store_activation.take_for_session());
        Ok(ActorActivation::Materialized(
            ActorMaterialization::Session(SessionMaterializedActorActivation {
                tracker: Arc::clone(self),
                session: session.clone(),
                handle,
            }),
        ))
    }

    fn commit_session_activation(
        &self,
        session: &ActorInstanceSessionLease,
        authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<(), ActorInstanceStoreError> {
        let state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let current_session = state
            .open_sessions
            .get(session.router_session_id())
            .filter(|current| Arc::ptr_eq(current, &session.state))
            .ok_or(ActorInstanceStoreError::InstanceReplaced)?;
        if !current_session.open.load(Ordering::Acquire) {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let owned = state.handle_owners.get(&identity).is_some_and(|owner| {
            owner.matches(
                session.router_session_id(),
                current_session,
                &handle.instance,
            )
        });
        if !owned {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        self.store.mark_admitted(authority, handle)
    }

    fn discard_provisional_exact(
        &self,
        session: &ActorInstanceSessionLease,
        handle: &ActorInstanceHandle,
    ) -> bool {
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let owned = state.handle_owners.get(&identity).is_some_and(|owner| {
            owner.session.upgrade().is_some_and(|owner_session| {
                owner.matches(
                    session.router_session_id(),
                    &owner_session,
                    &handle.instance,
                ) && Arc::ptr_eq(&owner_session, &session.state)
            })
        });
        if !owned {
            return false;
        }
        let discarded = self.store.discard_exact(handle);
        if let Some(handles) = state.by_session.get_mut(session.router_session_id()) {
            handles.retain(|candidate| !Arc::ptr_eq(&candidate.instance, &handle.instance));
            if handles.is_empty() {
                state.by_session.remove(session.router_session_id());
            }
        }
        state.handle_owners.remove(&identity);
        discarded
    }

    pub fn is_session_open(&self, router_session_id: &str) -> bool {
        self.state
            .lock()
            .expect("actor instance session tracker lock poisoned")
            .open_sessions
            .contains_key(router_session_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn tracked_owner_count_for_test(&self) -> usize {
        self.state
            .lock()
            .expect("actor instance session tracker lock poisoned")
            .handle_owners
            .len()
    }

    /// A materialized handle has exactly one Router-session owner.
    #[cfg(test)]
    pub(crate) fn track(
        &self,
        router_session_id: &str,
        handle: ActorInstanceHandle,
    ) -> Result<(), ActorInstanceSessionTrackError> {
        if router_session_id.is_empty() {
            return Err(ActorInstanceSessionTrackError::EmptySessionId);
        }
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let current_session = state
            .open_sessions
            .get(router_session_id)
            .cloned()
            .ok_or_else(|| ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            })?;
        self.store
            .validate_publishable_exact(&handle)
            .map_err(|error| ActorInstanceSessionTrackError::NotPublishable {
                message: error.to_string(),
            })?;
        if let Some(owner) = state.handle_owners.get(&identity) {
            if owner
                .instance
                .upgrade()
                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &handle.instance))
            {
                return Err(ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id: owner.session_id.clone(),
                });
            }
        }
        state.handle_owners.insert(
            identity,
            ActorInstanceOwner {
                session_id: router_session_id.to_string(),
                session: Arc::downgrade(&current_session),
                instance: Arc::downgrade(&handle.instance),
            },
        );
        state
            .by_session
            .entry(router_session_id.to_string())
            .or_default()
            .push(handle);
        Ok(())
    }

    /// Publishes only for the exact connection generation that admitted the
    /// detached activation task.
    pub fn track_with_lease(
        &self,
        lease: &ActorInstanceSessionLease,
        handle: ActorInstanceHandle,
    ) -> Result<(), ActorInstanceSessionTrackError> {
        let router_session_id = lease.router_session_id();
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        let Some(current_session) = state.open_sessions.get(router_session_id).cloned() else {
            return Err(ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            });
        };
        if !Arc::ptr_eq(&current_session, &lease.state) || !lease.state.open.load(Ordering::Acquire)
        {
            return Err(ActorInstanceSessionTrackError::SessionNotOpen {
                router_session_id: router_session_id.to_string(),
            });
        }
        self.store
            .validate_publishable_exact(&handle)
            .map_err(|error| ActorInstanceSessionTrackError::NotPublishable {
                message: error.to_string(),
            })?;
        if let Some(owner) = state.handle_owners.get(&identity) {
            if owner
                .instance
                .upgrade()
                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &handle.instance))
            {
                return Err(ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id: owner.session_id.clone(),
                });
            }
        }
        state.handle_owners.insert(
            identity,
            ActorInstanceOwner {
                session_id: router_session_id.to_string(),
                session: Arc::downgrade(&current_session),
                instance: Arc::downgrade(&handle.instance),
            },
        );
        state
            .by_session
            .entry(router_session_id.to_string())
            .or_default()
            .push(handle);
        Ok(())
    }

    /// Discards an activation which could not be published because its Router session closed.
    ///
    /// Holding the ownership lock across the exact store discard prevents a concurrent successful
    /// `track` from turning this cleanup into removal of a newly owned instance.
    pub fn discard_if_untracked(&self, handle: &ActorInstanceHandle) -> bool {
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        if state.handle_owners.get(&identity).is_some_and(|owner| {
            owner
                .instance
                .upgrade()
                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &handle.instance))
        }) {
            return false;
        }
        self.store.discard_exact(handle)
    }

    /// Applies the Runtime-side upgrade fence only to the exact instance owned
    /// by this Router session. Stale sessions and duplicate old fences are inert.
    pub fn begin_upgrade_exact(&self, router_session_id: &str, fence: &ActorInstanceFence) -> bool {
        self.exact_session_handle(router_session_id, fence)
            .is_some_and(|handle| self.store.begin_upgrade_exact(&handle))
    }

    /// Retires the exact upgrading instance after the Router's active ledger
    /// reaches zero. Repeated or stale completion notifications are inert.
    pub fn discard_upgrading_exact(
        &self,
        router_session_id: &str,
        fence: &ActorInstanceFence,
    ) -> bool {
        let Some(handle) = self.exact_session_handle(router_session_id, fence) else {
            return false;
        };
        if !self.store.discard_upgrading_exact(&handle) {
            return false;
        }
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        if let Some(handles) = state.by_session.get_mut(router_session_id) {
            handles.retain(|candidate| !Arc::ptr_eq(&candidate.instance, &handle.instance));
            if handles.is_empty() {
                state.by_session.remove(router_session_id);
            }
        }
        state.handle_owners.remove(&identity);
        true
    }

    /// Evicts one exact live incarnation without requiring an upgrade marker.
    pub fn discard_exact(&self, router_session_id: &str, fence: &ActorInstanceFence) -> bool {
        let Some(handle) = self.exact_session_handle(router_session_id, fence) else {
            return false;
        };
        if !self.store.discard_exact(&handle) {
            return false;
        }
        let identity = Arc::as_ptr(&handle.instance) as usize;
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        if let Some(handles) = state.by_session.get_mut(router_session_id) {
            handles.retain(|candidate| !Arc::ptr_eq(&candidate.instance, &handle.instance));
            if handles.is_empty() {
                state.by_session.remove(router_session_id);
            }
        }
        state.handle_owners.remove(&identity);
        true
    }

    /// Takes a session before discarding, so repeated/stale cleanup is inert.
    pub fn discard_session(&self, router_session_id: &str) -> usize {
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        if let Some(session) = state.open_sessions.remove(router_session_id) {
            session.open.store(false, Ordering::Release);
            session.closed.notify_waiters();
        }
        let handles = state
            .by_session
            .remove(router_session_id)
            .unwrap_or_default();
        for handle in &handles {
            state
                .handle_owners
                .remove(&(Arc::as_ptr(&handle.instance) as usize));
        }
        let discarded = self.store.discard_exact_batch(&handles);
        discarded
    }

    /// Runtime shutdown discards all volatile state, never registry bootstrap.
    pub fn discard_all(&self) -> usize {
        let mut state = self
            .state
            .lock()
            .expect("actor instance session tracker lock poisoned");
        for (_, session) in state.open_sessions.drain() {
            session.open.store(false, Ordering::Release);
            session.closed.notify_waiters();
        }
        state.handle_owners.clear();
        let handles = state
            .by_session
            .drain()
            .flat_map(|(_, handles)| handles)
            .collect::<Vec<_>>();
        self.store.discard_exact_batch(&handles)
    }

    fn exact_session_handle(
        &self,
        router_session_id: &str,
        fence: &ActorInstanceFence,
    ) -> Option<ActorInstanceHandle> {
        self.state
            .lock()
            .expect("actor instance session tracker lock poisoned")
            .by_session
            .get(router_session_id)?
            .iter()
            .find(|handle| handle.fence == *fence)
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActorInstanceSessionTrackError {
    #[error("router session id must be non-empty")]
    EmptySessionId,
    #[error("Router session {router_session_id} is already open")]
    SessionAlreadyOpen { router_session_id: String },
    #[error("Router session {router_session_id} is not open")]
    SessionNotOpen { router_session_id: String },
    #[error("Actor instance handle is already tracked by Router session {owner_session_id}")]
    AlreadyTracked { owner_session_id: String },
    #[error("Actor instance handle is not publishable: {message}")]
    NotPublishable { message: String },
    #[error("Actor activation failed: {message}")]
    ActivationFailed { message: String },
}

impl ActorInstanceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically reuses or materializes one exact incarnation.
    ///
    /// Decoding runs while the store mutex is held. Activation is intentionally
    /// uncommon, and this makes publication atomic: concurrent callers can
    /// never observe or duplicate a partially initialized instance.
    #[cfg(test)]
    pub fn activate(
        &self,
        request: ActorActivationRequest<'_>,
    ) -> Result<ActorInstanceHandle, ActorInstanceStoreError> {
        self.activate_with_created(request)
            .map(|(handle, _)| handle)
    }

    /// [`Self::activate`] plus whether this call materialized a new instance
    /// (as opposed to reusing an existing incarnation). Only the materializer
    /// may run `create` and mark the instance admitted.
    #[cfg(test)]
    pub fn activate_with_created(
        &self,
        request: ActorActivationRequest<'_>,
    ) -> Result<(ActorInstanceHandle, bool), ActorInstanceStoreError> {
        self.activate_with_created_raw(request)
    }

    pub(crate) fn begin_activation(
        &self,
        request: ActorActivationRequest<'_>,
    ) -> Result<ActorActivation, ActorInstanceStoreError> {
        let (handle, materialized) = self.activate_with_created_raw(request)?;
        if materialized {
            Ok(ActorActivation::Materialized(ActorMaterialization::Store(
                MaterializedActorActivation {
                    store: self.clone(),
                    handle: Some(handle),
                },
            )))
        } else {
            Ok(ActorActivation::Existing(handle))
        }
    }

    fn activate_with_created_raw(
        &self,
        request: ActorActivationRequest<'_>,
    ) -> Result<(ActorInstanceHandle, bool), ActorInstanceStoreError> {
        request.fence.validate()?;
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");

        if let Some(latest_epoch) = state
            .latest_epochs
            .get(&request.fence.incarnation.logical_key)
            .copied()
        {
            if request.fence.incarnation.epoch < latest_epoch {
                return Err(ActorInstanceStoreError::StaleEpoch {
                    requested: request.fence.incarnation.epoch,
                    latest: latest_epoch,
                });
            }
        }
        if let Some(existing) = state.instances.get(&request.fence.incarnation) {
            ensure_instance_fence(existing, &request.fence)?;
            if existing.upgrading.load(Ordering::Acquire) {
                return Err(ActorInstanceStoreError::InstanceReplaced);
            }
            let instance_state = existing
                .state
                .lock()
                .expect("actor instance state lock poisoned");
            if instance_state.pending_discard {
                return Err(ActorInstanceStoreError::InstanceReplaced);
            }
            return Ok((
                ActorInstanceHandle {
                    fence: request.fence,
                    instance: Arc::clone(existing),
                },
                false,
            ));
        }

        let declaration =
            resolve_actor_declaration(request.program, &request.fence.declaration_owner)?;
        validate_declaration_fence(declaration, &request.fence)?;
        let materialized = materialize_instance(&request, declaration, &self.arena_limits)?;
        let instance = Arc::new(materialized.instance);

        state
            .latest_epochs
            .entry(request.fence.incarnation.logical_key.clone())
            .and_modify(|epoch| *epoch = (*epoch).max(request.fence.incarnation.epoch))
            .or_insert(request.fence.incarnation.epoch);
        state
            .instances
            .insert(request.fence.incarnation.clone(), Arc::clone(&instance));

        Ok((
            ActorInstanceHandle {
                fence: request.fence,
                instance,
            },
            true,
        ))
    }

    /// Removes only the exact materialized instance represented by `handle`.
    ///
    /// Pointer identity closes the same-epoch cleanup race: an old cleanup
    /// handle cannot remove a later re-materialization with otherwise identical
    /// logical and declaration fences.
    pub fn discard_exact(&self, handle: &ActorInstanceHandle) -> bool {
        self.discard_exact_batch(std::slice::from_ref(handle)) == 1
    }

    /// Closes this exact incarnation to continuation resume and activation reuse.
    ///
    /// Upgrade requires the instance to be quiescent (active and suspended
    /// continuation counters both zero). Because every new segment revalidates
    /// the upgrading fence after acquiring the arena, no segment can start
    /// after this call succeeds.
    pub fn begin_upgrade_exact(&self, handle: &ActorInstanceHandle) -> bool {
        let state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let Some(instance) = state.instances.get(&handle.fence.incarnation) else {
            return false;
        };
        if ensure_instance_fence(instance, &handle.fence).is_err()
            || !Arc::ptr_eq(instance, &handle.instance)
        {
            return false;
        }
        let instance_state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        if instance_state.active_segments != 0 || instance_state.suspended_segments != 0 {
            return false;
        }
        instance.upgrading.store(true, Ordering::Release);
        true
    }

    /// Retires and removes only the exact incarnation previously fenced for upgrade.
    ///
    /// Advancing the local epoch floor prevents delayed activation traffic from
    /// recreating the old epoch after the Router has advanced the registry.
    pub fn discard_upgrading_exact(&self, handle: &ActorInstanceHandle) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let Some(instance) = state.instances.get(&handle.fence.incarnation).cloned() else {
            return false;
        };
        if ensure_instance_fence(&instance, &handle.fence).is_err()
            || !Arc::ptr_eq(&instance, &handle.instance)
            || !instance.upgrading.load(Ordering::Acquire)
        {
            return false;
        }
        let instance_state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        if instance_state.active_segments != 0 || instance_state.suspended_segments != 0 {
            return false;
        }
        let Some(next_epoch) = handle.fence.incarnation.epoch.checked_add(1) else {
            return false;
        };
        state.instances.remove(&handle.fence.incarnation);
        handle.instance.admission_notify.notify_waiters();
        state
            .latest_epochs
            .entry(handle.fence.incarnation.logical_key.clone())
            .and_modify(|epoch| *epoch = (*epoch).max(next_epoch))
            .or_insert(next_epoch);
        true
    }

    /// Atomically removes the live instances represented by the exact handles.
    ///
    /// Both the complete Actor fence and the materialized instance identity
    /// must still match. This makes repeated or delayed disconnect cleanup
    /// harmless even when the same incarnation has since been materialized
    /// again. Removal requires quiescence: an instance with in-flight segments
    /// is marked `pending_discard` instead and reclaimed by the segment that
    /// brings the counters back to zero.
    pub fn discard_exact_batch(&self, handles: &[ActorInstanceHandle]) -> usize {
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let mut removed = 0;
        for handle in handles {
            let Some(instance) = state.instances.get(&handle.fence.incarnation).cloned() else {
                continue;
            };
            if ensure_instance_fence(&instance, &handle.fence).is_err()
                || !Arc::ptr_eq(&instance, &handle.instance)
            {
                continue;
            }
            let mut instance_state = instance
                .state
                .lock()
                .expect("actor instance state lock poisoned");
            if instance_state.active_segments == 0 && instance_state.suspended_segments == 0 {
                state.instances.remove(&handle.fence.incarnation);
                handle.instance.admission_notify.notify_waiters();
                removed += 1;
            } else {
                instance_state.pending_discard = true;
            }
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .expect("actor instance store lock poisoned")
            .instances
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resolves the exact live incarnation for one authenticated Actor
    /// reference (logical key + pinned epoch). Returns `None` when the
    /// incarnation is absent, replaced, upgrading, or not yet admitted —
    /// callers must treat that as a submission-side definite rejection with
    /// no task created.
    pub(crate) fn handle_for_actor_ref(
        &self,
        actor_ref: &ActorRef,
    ) -> Option<ActorInstanceHandle> {
        let epoch = actor_ref.epoch()?;
        let incarnation = ActorIncarnationKey {
            logical_key: ActorLogicalKey {
                service_id: actor_ref.service_id().to_string(),
                actor_type_identity: actor_ref.actor_type_identity().to_string(),
                actor_id_type_identity: actor_ref.actor_id_type_identity().to_string(),
                actor_id_encoding_version: actor_ref.actor_id_encoding_version().to_string(),
                canonical_actor_id_key_bytes: actor_ref.canonical_actor_id_key_bytes().to_vec(),
                actor_id_hash: actor_ref.actor_id_hash().to_string(),
            },
            epoch,
        };
        let state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        if state
            .latest_epochs
            .get(&incarnation.logical_key)
            .is_some_and(|latest| *latest != epoch)
        {
            return None;
        }
        let instance = state.instances.get(&incarnation)?;
        if instance.upgrading.load(Ordering::Acquire) {
            return None;
        }
        let instance_state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        if instance_state.pending_discard || !instance.admitted.load(Ordering::Acquire) {
            return None;
        }
        Some(ActorInstanceHandle {
            fence: instance.fence.clone(),
            instance: Arc::clone(instance),
        })
    }

    /// The later Actor executor enters field state through this exact-fence
    /// gate. The method is crate-private so ordinary host/request consumers
    /// cannot turn an opaque handle into field access. The arena guard must be
    /// quiescent (test helper; it fails fast instead of waiting on a live
    /// segment).
    pub(crate) fn with_fields_for_executor<T>(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
        operation: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap) -> T,
    ) -> Result<T, ActorInstanceStoreError> {
        let instance = {
            let state = self
                .state
                .lock()
                .expect("actor instance store lock poisoned");
            let current = state
                .instances
                .get(&handle.fence.incarnation)
                .ok_or(ActorInstanceStoreError::InstanceNotFound)?;
            if state
                .latest_epochs
                .get(&handle.fence.incarnation.logical_key)
                .is_some_and(|latest| *latest != handle.fence.incarnation.epoch)
            {
                return Err(ActorInstanceStoreError::StaleEpoch {
                    requested: handle.fence.incarnation.epoch,
                    latest: state.latest_epochs[&handle.fence.incarnation.logical_key],
                });
            }
            ensure_instance_fence(current, &handle.fence)?;
            if !Arc::ptr_eq(current, &handle.instance) {
                return Err(ActorInstanceStoreError::InstanceReplaced);
            }
            Arc::clone(current)
        };
        let mut state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let mut heap = state
            .arena
            .clone()
            .try_lock_owned()
            .expect("with_fields_for_executor requires a quiescent Actor arena");
        Ok(operation(&mut state.fields, &mut heap))
    }

    /// Reads one field root (an arena handle or scalar) for the active segment.
    pub(crate) fn field_root(
        &self,
        handle: &ActorInstanceHandle,
        field: &str,
    ) -> Result<Option<ActorFieldValue>, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        Ok(state
            .fields
            .iter()
            .find(|candidate| candidate.name == field)
            .cloned())
    }

    /// Writes one field root directly into the shared arena (no wire roundtrip).
    ///
    /// Returns `Ok(false)` when the field is absent from the instance frame.
    pub(crate) fn set_field_root(
        &self,
        handle: &ActorInstanceHandle,
        field: &str,
        value: RuntimeValue,
    ) -> Result<bool, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let mut state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let Some(target) = state
            .fields
            .iter_mut()
            .find(|candidate| candidate.name == field)
        else {
            return Ok(false);
        };
        target.value = value;
        target.assigned = true;
        Ok(true)
    }

    /// Acquires one actor method segment.
    ///
    /// Waits for admission (ordinary methods), then locks the instance arena
    /// and revalidates the instance fence, discard state and arena identity
    /// after the lock. The returned lease holds the arena guard; the guard must
    /// be released before any real `Pending` and reacquired after wake.
    pub(crate) async fn acquire_segment(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<SegmentLease, ActorInstanceStoreError> {
        let instance = self.wait_until_admitted(handle).await?;
        self.acquire_segment_inner(instance, handle).await
    }

    /// Segment acquisition for the platform activation path (`create`).
    /// The instance is intentionally not admitted yet, so this path skips the
    /// admission gate that ordinary method execution waits on.
    pub(crate) async fn acquire_segment_for_activation(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<SegmentLease, ActorInstanceStoreError> {
        let instance = self.resolve_active_instance(handle)?;
        self.acquire_segment_inner(instance, handle).await
    }

    async fn acquire_segment_inner(
        &self,
        instance: Arc<ActorInstance>,
        handle: &ActorInstanceHandle,
    ) -> Result<SegmentLease, ActorInstanceStoreError> {
        loop {
            let (arena, epoch) = {
                let state = instance
                    .state
                    .lock()
                    .expect("actor instance state lock poisoned");
                if state.pending_discard {
                    return Err(ActorInstanceStoreError::InstanceReplaced);
                }
                (Arc::clone(&state.arena), state.arena_epoch)
            };
            let guard = arena.clone().lock_owned().await;
            // Revalidate after acquiring the arena: the instance may have been
            // upgraded, discarded, or compacted while this future waited.
            let instance = self.resolve_active_instance(handle)?;
            let mut state = instance
                .state
                .lock()
                .expect("actor instance state lock poisoned");
            if state.pending_discard {
                return Err(ActorInstanceStoreError::InstanceReplaced);
            }
            if !Arc::ptr_eq(&state.arena, &arena) {
                // A compaction swapped the arena between our snapshot and the
                // lock; retry against the current arena.
                drop(state);
                continue;
            }
            debug_assert_eq!(state.arena_epoch, epoch);
            let nonce = instance
                .next_execution_token
                .fetch_add(1, Ordering::Relaxed);
            let token = Arc::new(ActorExecutionToken {
                nonce,
                active: AtomicBool::new(true),
            });
            state.active_segments += 1;
            return Ok(SegmentLease {
                store: self.clone(),
                handle: handle.clone(),
                token,
                arena,
                guard: Some(guard),
                expected_epoch: epoch,
                state: SegmentState::Active,
            });
        }
    }

    async fn wait_until_admitted(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<Arc<ActorInstance>, ActorInstanceStoreError> {
        loop {
            let instance = self.resolve_active_instance(handle)?;
            if instance.admitted.load(Ordering::Acquire) {
                return Ok(instance);
            }
            let notified = instance.admission_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let instance = self.resolve_active_instance(handle)?;
            if instance.admitted.load(Ordering::Acquire) {
                return Ok(instance);
            }
            #[cfg(test)]
            self.run_admission_wait_before_poll_test_action(handle);
            notified.await;
        }
    }

    #[cfg(test)]
    fn install_admission_wait_before_poll_test_action(
        &self,
        handle: &ActorInstanceHandle,
        action: AdmissionWaitBeforePollTestAction,
    ) {
        let mut pending = handle
            .instance
            .admission_wait_before_poll_action
            .lock()
            .expect("Actor admission wait test action lock poisoned");
        assert!(pending.replace(action).is_none());
    }

    #[cfg(test)]
    fn run_admission_wait_before_poll_test_action(&self, handle: &ActorInstanceHandle) {
        let action = handle
            .instance
            .admission_wait_before_poll_action
            .lock()
            .expect("Actor admission wait test action lock poisoned")
            .take();
        match action {
            Some(AdmissionWaitBeforePollTestAction::Admit) => self
                .mark_admitted(&ActorExecutorAuthority::new(), handle)
                .expect("test admission action must target the exact Actor instance"),
            Some(AdmissionWaitBeforePollTestAction::Discard) => {
                assert!(self.discard_exact(handle));
            }
            None => {}
        }
    }

    /// Waits until this exact materialization is admitted. Exact discard wakes
    /// the waiter and is observed as `InstanceNotFound`/`InstanceReplaced`.
    pub(crate) async fn await_admission(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<(), ActorInstanceStoreError> {
        self.wait_until_admitted(handle).await.map(drop)
    }

    /// Publication fence used by Router-session tracking. The tracker holds its
    /// ownership mutex while this check runs, so a disconnect and a late track
    /// cannot register a stale or half-created handle.
    fn validate_publishable_exact(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<(), ActorInstanceStoreError> {
        let instance = self.resolve_active_instance(handle)?;
        if !instance.admitted.load(Ordering::Acquire) {
            return Err(ActorInstanceStoreError::InstanceNotAdmitted);
        }
        Ok(())
    }

    /// Commits an active (or resumed) segment.
    ///
    /// Field mutations are already live in the shared arena; commit only
    /// revalidates the instance fence and arena epoch, retires the execution
    /// token and releases the continuation counter. A segment that hits a
    /// discard fence still ends (counters are released and a pending discard is
    /// reclaimed) but surfaces the replacement error.
    pub(crate) fn commit_segment(
        &self,
        handle: &ActorInstanceHandle,
        lease: &mut SegmentLease,
    ) -> Result<(), ActorInstanceStoreError> {
        if lease.is_finished() {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        let current = self.current_instance_for_execution_lease(handle, lease)?;
        let mut state = current
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let result = if state.pending_discard {
            Err(ActorInstanceStoreError::InstanceReplaced)
        } else if lease.expected_epoch() != state.arena_epoch {
            Err(ActorInstanceStoreError::InstanceReplaced)
        } else {
            Ok(())
        };
        match lease.state {
            SegmentState::Active => {
                debug_assert!(state.active_segments > 0);
                state.active_segments = state.active_segments.saturating_sub(1);
            }
            SegmentState::Suspended => {
                debug_assert!(state.suspended_segments > 0);
                state.suspended_segments = state.suspended_segments.saturating_sub(1);
            }
            SegmentState::Finished => {}
        }
        lease.token.active.store(false, Ordering::Release);
        lease.state = SegmentState::Finished;
        let reclaim =
            state.active_segments == 0 && state.suspended_segments == 0 && state.pending_discard;
        drop(state);
        if reclaim {
            let _ = self.discard_exact(handle);
        }
        result
    }

    /// Moves an active segment into the suspended counter at a real `Pending`.
    pub(crate) fn suspend_segment(
        &self,
        handle: &ActorInstanceHandle,
        lease: &mut SegmentLease,
    ) -> Result<(), ActorInstanceStoreError> {
        if lease.is_finished() {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        if lease.is_suspended() {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        let current = self.current_instance_for_execution_lease(handle, lease)?;
        let mut state = current
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        debug_assert!(state.active_segments > 0);
        state.active_segments = state.active_segments.saturating_sub(1);
        state.suspended_segments += 1;
        lease.state = SegmentState::Suspended;
        Ok(())
    }

    /// Revalidates a resumed segment (after the arena guard was reacquired)
    /// and moves it back into the active counter.
    pub(crate) fn resume_segment(
        &self,
        handle: &ActorInstanceHandle,
        lease: &mut SegmentLease,
    ) -> Result<(), ActorInstanceStoreError> {
        if lease.is_finished() {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        if !lease.is_suspended() {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        let current = self.current_instance_for_execution_lease(handle, lease)?;
        let mut state = current
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        if state.pending_discard {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        if lease.expected_epoch() != state.arena_epoch {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        debug_assert!(state.suspended_segments > 0);
        state.suspended_segments = state.suspended_segments.saturating_sub(1);
        state.active_segments += 1;
        lease.state = SegmentState::Active;
        Ok(())
    }

    /// Releases a segment that ended without commit (error path or drop).
    ///
    /// Already-executed field mutations stay in the shared arena (design §3.4);
    /// only the continuation counters are released. If a discard was pending,
    /// the instance is reclaimed once the last segment ends.
    pub(crate) fn abandon_segment(&self, handle: &ActorInstanceHandle, lease: &mut SegmentLease) {
        if lease.is_finished() {
            return;
        }
        let instance = &lease.handle.instance;
        let mut state = instance
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match lease.state {
            SegmentState::Active => {
                state.active_segments = state.active_segments.saturating_sub(1);
            }
            SegmentState::Suspended => {
                state.suspended_segments = state.suspended_segments.saturating_sub(1);
            }
            SegmentState::Finished => {}
        }
        lease.token.active.store(false, Ordering::Release);
        lease.state = SegmentState::Finished;
        let reclaim =
            state.active_segments == 0 && state.suspended_segments == 0 && state.pending_discard;
        drop(state);
        if reclaim {
            let _ = self.discard_exact(handle);
        }
    }

    fn current_instance_for_execution_lease(
        &self,
        handle: &ActorInstanceHandle,
        lease: &SegmentLease,
    ) -> Result<Arc<ActorInstance>, ActorInstanceStoreError> {
        lease.token.ensure_active()?;
        let current = self.resolve_current_instance(handle)?;
        if !Arc::ptr_eq(&current, &lease.handle.instance) {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        // The nonce is private and monotonically issued by this instance. This
        // also makes accidental cross-instance frame reuse fail closed.
        if lease.token.nonce == 0
            || lease.token.nonce >= current.next_execution_token.load(Ordering::Acquire)
        {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        Ok(current)
    }

    fn resolve_current_instance(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<Arc<ActorInstance>, ActorInstanceStoreError> {
        let state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let current = state
            .instances
            .get(&handle.fence.incarnation)
            .ok_or(ActorInstanceStoreError::InstanceNotFound)?;
        if state
            .latest_epochs
            .get(&handle.fence.incarnation.logical_key)
            .is_some_and(|latest| *latest != handle.fence.incarnation.epoch)
        {
            return Err(ActorInstanceStoreError::StaleEpoch {
                requested: handle.fence.incarnation.epoch,
                latest: state.latest_epochs[&handle.fence.incarnation.logical_key],
            });
        }
        ensure_instance_fence(current, &handle.fence)?;
        if !Arc::ptr_eq(current, &handle.instance) {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        Ok(Arc::clone(current))
    }

    fn resolve_active_instance(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<Arc<ActorInstance>, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        if instance.upgrading.load(Ordering::Acquire) {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        Ok(instance)
    }

    /// Marks the exact incarnation as admitted after a successful `create`
    /// (or immediately when no `create` is declared). Wakes all waiters.
    pub(crate) fn mark_admitted(
        &self,
        authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<(), ActorInstanceStoreError> {
        let _ = authority;
        let instance = self.resolve_active_instance(handle)?;
        instance.admitted.store(true, Ordering::Release);
        instance.admission_notify.notify_waiters();
        Ok(())
    }

    /// Whole-arena replacement at quiescence.
    ///
    /// When the active and suspended continuation counters are both zero, no
    /// upgrade or discard is pending, and the arena exceeds its compaction
    /// threshold, the live field roots are cloned as one graph into a fresh
    /// arena at epoch + 1 and the store arena is swapped atomically under the
    /// instance state lock. Dead invocation nodes are dropped; old handles fail
    /// closed because their epoch no longer matches the arena.
    pub(crate) async fn compact_if_quiescent(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<bool, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let (arena, limits) = {
            let state = instance
                .state
                .lock()
                .expect("actor instance state lock poisoned");
            if !quiescent_for_replacement(&state) || instance.upgrading.load(Ordering::Acquire) {
                return Ok(false);
            }
            (Arc::clone(&state.arena), state.limits.clone())
        };
        let guard = arena.clone().lock_owned().await;
        let compact_threshold = (limits.max_nodes / 2).max(1);
        if guard.len() < compact_threshold {
            return Ok(false);
        }
        // Revalidate quiescence and arena identity after the arena lock.
        let current = self.resolve_current_instance(handle)?;
        if !Arc::ptr_eq(&current, &instance) {
            return Ok(false);
        }
        let (mut fields, old_epoch) = {
            let state = current
                .state
                .lock()
                .expect("actor instance state lock poisoned");
            if !quiescent_for_replacement(&state)
                || current.upgrading.load(Ordering::Acquire)
                || !Arc::ptr_eq(&state.arena, &arena)
            {
                return Ok(false);
            }
            (state.fields.clone(), state.arena_epoch)
        };
        let new_epoch =
            old_epoch
                .checked_add(1)
                .ok_or_else(|| ActorInstanceStoreError::CompactionFailed {
                    message: "Actor arena epoch space exhausted".to_string(),
                })?;
        let roots = fields
            .iter()
            .map(|field| field.value.clone())
            .collect::<Vec<_>>();
        reject_request_scoped_actor_field_values(&roots, &guard).map_err(|error| {
            ActorInstanceStoreError::CompactionFailed {
                message: error.to_string(),
            }
        })?;
        let mut fresh = RequestHeap::new_with_epoch(new_epoch, limits.clone());
        let cloned = deep_clone_runtime_values_between_heaps(&guard, &mut fresh, &roots).map_err(
            |error| ActorInstanceStoreError::CompactionFailed {
                message: error.to_string(),
            },
        )?;
        for (field, value) in fields.iter_mut().zip(cloned) {
            field.value = value;
        }
        let mut state = current
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        if !quiescent_for_replacement(&state)
            || current.upgrading.load(Ordering::Acquire)
            || !Arc::ptr_eq(&state.arena, &arena)
        {
            return Ok(false);
        }
        state.fields = fields;
        state.arena = Arc::new(tokio::sync::Mutex::new(fresh));
        state.arena_epoch = new_epoch;
        Ok(true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn arena_epoch_for_test(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<u32, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        Ok(state.arena_epoch)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn arena_ptr_for_test(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<usize, ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        Ok(Arc::as_ptr(&state.arena) as usize)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn segment_counters_for_test(
        &self,
        handle: &ActorInstanceHandle,
    ) -> Result<(usize, usize), ActorInstanceStoreError> {
        let instance = self.resolve_current_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        Ok((state.active_segments, state.suspended_segments))
    }
}

fn quiescent_for_replacement(state: &ActorInstanceState) -> bool {
    state.active_segments == 0 && state.suspended_segments == 0 && !state.pending_discard
}

/// Request-scoped values cannot enter persistent Actor state (design §3.5).
///
/// Validation runs on the field write path and again during compaction root
/// collection. Callback capabilities and request-local exceptions are
/// rejected; stream-typed fields are rejected separately on the write path by
/// the linked type plan.
pub(crate) fn reject_request_scoped_actor_field_values(
    roots: &[RuntimeValue],
    heap: &RequestHeap,
) -> Result<(), RuntimeError> {
    let mut visited = HashSet::new();
    let mut pending = roots.iter().map(|root| (root, 0_usize)).collect::<Vec<_>>();
    while let Some((value, depth)) = pending.pop() {
        if depth > heap.limits().max_clone_depth {
            return Err(RuntimeError::ResourceLimitExceeded {
                resource: "requestHeap".to_string(),
                reason: "max persistent Actor graph depth".to_string(),
                limit: heap.limits().max_clone_depth,
                current: depth,
                requested_delta: depth.saturating_sub(heap.limits().max_clone_depth),
            });
        }
        let RuntimeValue::Heap(handle) = value else {
            continue;
        };
        if !visited.insert(*handle) {
            continue;
        }
        match heap.get(*handle)? {
            HeapNode::Bytes(_) => {}
            HeapNode::Array(items) => {
                pending.extend(items.iter().map(|item| (item, depth + 1)));
            }
            HeapNode::Object(object) => {
                pending.extend(object.fields().values().map(|item| (item, depth + 1)));
            }
            HeapNode::Map(map) => {
                pending.extend(map.values().map(|item| (item, depth + 1)));
            }
            HeapNode::Interface(interface) => match interface.carrier() {
                InterfaceCarrier::Local { payload, .. } => pending.push((payload, depth + 1)),
                InterfaceCarrier::CallbackCapability(_) => {
                    return Err(RuntimeError::Decode(
                        "request-scoped callback capability cannot enter persistent Actor state"
                            .to_string(),
                    ));
                }
            },
            HeapNode::Exception(_) => {
                return Err(RuntimeError::Decode(
                    "request-local exception cannot enter persistent Actor state".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn ensure_instance_fence(
    instance: &ActorInstance,
    requested: &ActorInstanceFence,
) -> Result<(), ActorInstanceStoreError> {
    if instance.fence == *requested {
        Ok(())
    } else {
        Err(ActorInstanceStoreError::FenceMismatch)
    }
}

struct MaterializedActorInstance {
    instance: ActorInstance,
}

fn materialize_instance(
    request: &ActorActivationRequest<'_>,
    declaration: &LinkedActorDeclaration,
    limits: &RequestHeapLimits,
) -> Result<MaterializedActorInstance, ActorInstanceStoreError> {
    if request.bootstrap_encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
        return Err(ActorInstanceStoreError::UnsupportedBootstrapEncoding {
            actual: request.bootstrap_encoding_version.to_string(),
        });
    }
    let payload: Value = serde_json::from_slice(request.bootstrap_payload).map_err(|error| {
        ActorInstanceStoreError::CreationInputsDecode {
            message: error.to_string(),
        }
    })?;
    let create_args = payload
        .as_array()
        .ok_or(ActorInstanceStoreError::CreationInputsNotArray)?;
    let expected_create_args = declaration
        .create
        .as_ref()
        .map(|create| create.parameters.len())
        .unwrap_or(0);
    if create_args.len() != expected_create_args {
        return Err(ActorInstanceStoreError::CreationInputCount {
            expected: expected_create_args,
            actual: create_args.len(),
        });
    }

    let current_addr = ExecutableAddr {
        unit: request.fence.declaration_owner.unit.clone(),
        file: request.fence.declaration_owner.file.clone(),
        executable: 0,
    };
    let context = PlanContext::from_type_view(request.program, &current_addr);
    let mut heap = RequestHeap::new_with_epoch(0, limits.clone());
    let mut fields = Vec::with_capacity(declaration.fields.len());
    for field in &declaration.fields {
        if field.encoding != ActorFieldEncodingIr::CanonicalValueV1 {
            return Err(ActorInstanceStoreError::UnsupportedFieldEncoding {
                field: field.name.clone(),
            });
        }
        let plan = RuntimeTypePlan::from_linked(&field.ty, &context).map_err(|error| {
            ActorInstanceStoreError::DeclarationType {
                field: field.name.clone(),
                message: error.to_string(),
            }
        })?;
        if field.name == declaration.key_field {
            let wire: Value = serde_json::from_slice(
                &request
                    .fence
                    .incarnation
                    .logical_key
                    .canonical_actor_id_key_bytes,
            )
            .map_err(|error| ActorInstanceStoreError::KeyFieldDecode {
                field: field.name.clone(),
                message: error.to_string(),
            })?;
            let value = RuntimeBoundaryCodec::new(
                &plan,
                BoundaryUse::NativeArg,
                format!("Actor key field {}", field.name),
            )
            .from_wire_json(&wire, &mut heap)
            .map_err(|error| ActorInstanceStoreError::KeyFieldDecode {
                field: field.name.clone(),
                message: error.to_string(),
            })?;
            fields.push(ActorFieldValue::assigned(field.name.clone(), value));
        } else {
            fields.push(ActorFieldValue::unassigned(field.name.clone()));
        }
    }
    Ok(MaterializedActorInstance {
        instance: ActorInstance {
            fence: request.fence.clone(),
            activation_facts: ActorActivationFacts {
                create_input: request.bootstrap_payload.to_vec(),
            },
            state: Mutex::new(ActorInstanceState {
                fields,
                arena: Arc::new(tokio::sync::Mutex::new(heap)),
                arena_epoch: 0,
                limits: limits.clone(),
                active_segments: 0,
                suspended_segments: 0,
                pending_discard: false,
            }),
            next_execution_token: AtomicU64::new(1),
            upgrading: AtomicBool::new(false),
            admitted: AtomicBool::new(false),
            admission_notify: tokio::sync::Notify::new(),
            #[cfg(test)]
            admission_wait_before_poll_action: Mutex::new(None),
        },
    })
}

pub(crate) fn validate_declaration_fence(
    declaration: &LinkedActorDeclaration,
    fence: &ActorInstanceFence,
) -> Result<(), ActorInstanceStoreError> {
    if declaration.actor_runtime_abi_version != ACTOR_RUNTIME_ABI_VERSION_V1 {
        return Err(ActorInstanceStoreError::UnsupportedActorRuntimeAbi {
            actual: declaration.actor_runtime_abi_version.clone(),
        });
    }
    if declaration.implementation_owner.as_ref() != Some(&fence.declaration_owner) {
        return Err(ActorInstanceStoreError::DeclarationOwnerMismatch);
    }
    if declaration.actor_abi_identity != fence.actor_abi_identity {
        return Err(ActorInstanceStoreError::ActorAbiMismatch);
    }
    if declaration.actor_implementation_identity != fence.actor_implementation_identity {
        return Err(ActorInstanceStoreError::ActorImplementationMismatch);
    }
    Ok(())
}

pub(crate) fn resolve_actor_declaration<'a>(
    program: ProgramTypeView<'a>,
    owner: &LinkedActorDeclarationOwner,
) -> Result<&'a LinkedActorDeclaration, ActorInstanceStoreError> {
    let files = match owner.unit {
        UnitAddr::Service => program.service_files,
        UnitAddr::Package(slot) => program
            .packages
            .get(slot)
            .map(|package| package.files())
            .ok_or(ActorInstanceStoreError::DeclarationFileMissing)?,
    };
    let file = match &owner.file {
        FileAddr::LoadedFileIndex(index) => files.get(*index),
        FileAddr::FileIrIdentity(identity) => {
            files.iter().find(|file| file.file_ir_identity == *identity)
        }
    }
    .ok_or(ActorInstanceStoreError::DeclarationFileMissing)?;
    exact_declaration_in_file(file, owner)
}

fn exact_declaration_in_file<'a>(
    file: &'a Arc<LinkedFileUnit>,
    owner: &LinkedActorDeclarationOwner,
) -> Result<&'a LinkedActorDeclaration, ActorInstanceStoreError> {
    let mut matches = file.actor_declarations.iter().filter(|declaration| {
        declaration.implementation_owner.as_ref() == Some(owner)
            && declaration.actor_type.symbol == owner.actor_symbol
    });
    let declaration = matches
        .next()
        .ok_or(ActorInstanceStoreError::DeclarationMissing)?;
    if matches.next().is_some() {
        return Err(ActorInstanceStoreError::DeclarationAmbiguous);
    }
    Ok(declaration)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActorInstanceStoreError {
    #[error("invalid Actor logical key: {message}")]
    InvalidLogicalKey { message: String },
    #[error("Actor epoch must be positive")]
    InvalidEpoch,
    #[error("stale Actor epoch {requested}; latest materialized epoch is {latest}")]
    StaleEpoch { requested: u64, latest: u64 },
    #[error("Actor instance fence does not match the materialized instance")]
    FenceMismatch,
    #[error("Actor declaration owner does not match")]
    DeclarationOwnerMismatch,
    #[error("Actor ABI identity does not match")]
    ActorAbiMismatch,
    #[error("Actor implementation identity does not match")]
    ActorImplementationMismatch,
    #[error("Actor declaration file is missing")]
    DeclarationFileMissing,
    #[error("Actor declaration is missing")]
    DeclarationMissing,
    #[error("Actor declaration owner is ambiguous")]
    DeclarationAmbiguous,
    #[error("unsupported Actor runtime ABI {actual}")]
    UnsupportedActorRuntimeAbi { actual: String },
    #[error("unsupported Actor bootstrap encoding {actual}")]
    UnsupportedBootstrapEncoding { actual: String },
    #[error("Actor creation inputs decode failed: {message}")]
    CreationInputsDecode { message: String },
    #[error("Actor creation inputs must be a JSON array")]
    CreationInputsNotArray,
    #[error("Actor creation inputs count mismatch: expected {expected}, got {actual}")]
    CreationInputCount { expected: usize, actual: usize },
    #[error("Actor field {field} uses an unsupported encoding")]
    UnsupportedFieldEncoding { field: String },
    #[error("Actor field {field} type plan failed: {message}")]
    DeclarationType { field: String, message: String },
    #[error("Actor key field {field} decode failed: {message}")]
    KeyFieldDecode { field: String, message: String },
    #[error("Actor instance is not materialized")]
    InstanceNotFound,
    #[error("Actor instance handle was replaced")]
    InstanceReplaced,
    #[error("Actor instance is not admitted")]
    InstanceNotAdmitted,
    #[error("Actor execution token is expired")]
    ExecutionTokenExpired,
    #[error("Actor execution token is invalid for this instance")]
    ExecutionTokenInvalid,
    #[error("Actor arena compaction failed: {message}")]
    CompactionFailed { message: String },
}

#[cfg(test)]
mod tests;
