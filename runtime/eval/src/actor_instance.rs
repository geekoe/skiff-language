use std::{
    collections::HashMap,
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
    json::RuntimeBoundaryCodec, plan::BoundaryUse, request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedFileUnit,
    UnitAddr,
};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeTypePlan, RuntimeTypePlanLinkedExt,
};
use thiserror::Error;

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
}

#[derive(Debug)]
struct ActorInstance {
    fence: ActorInstanceFence,
    state: Mutex<ActorInstanceState>,
    scheduler: Arc<tokio::sync::Mutex<()>>,
    next_execution_token: AtomicU64,
    upgrading: AtomicBool,
    admitted: AtomicBool,
    admission_notify: tokio::sync::Notify,
}

#[derive(Debug)]
struct ActorInstanceState {
    fields: Vec<ActorFieldValue>,
    heap: RequestHeap,
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

/// One exclusive, rollback-capable execution snapshot.
///
/// The Tokio guard serializes only this incarnation. The field heap is cloned
/// before execution so a failed method cannot leak partial mutations into the
/// live instance.
pub(crate) struct ActorInstanceExecutionLease {
    instance: Arc<ActorInstance>,
    _scheduler_guard: tokio::sync::OwnedMutexGuard<()>,
    token: Arc<ActorExecutionToken>,
    fields: Arc<Mutex<Vec<ActorFieldValue>>>,
    heap: Option<RequestHeap>,
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

impl Drop for ActorInstanceExecutionLease {
    fn drop(&mut self) {
        self.token.active.store(false, Ordering::Release);
    }
}

impl ActorInstanceExecutionLease {
    pub(crate) fn instance_identity(&self) -> usize {
        Arc::as_ptr(&self.instance) as usize
    }

    pub(crate) fn token(&self) -> Arc<ActorExecutionToken> {
        Arc::clone(&self.token)
    }

    pub(crate) fn fields(&self) -> Arc<Mutex<Vec<ActorFieldValue>>> {
        Arc::clone(&self.fields)
    }

    pub(crate) fn take_heap(&mut self) -> RequestHeap {
        self.heap
            .take()
            .expect("Actor execution heap may only be taken once")
    }

    pub(crate) fn heap_mut(&mut self) -> &mut RequestHeap {
        self.heap
            .as_mut()
            .expect("Actor execution heap is present until taken")
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActorInstanceStore {
    state: Arc<Mutex<ActorInstanceStoreState>>,
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
    by_session: HashMap<String, Vec<ActorInstanceHandle>>,
    handle_owners: HashMap<usize, (String, Weak<ActorInstance>)>,
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

    /// A materialized handle has exactly one Router-session owner.
    pub fn track(
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
        if let Some((owner, tracked)) = state.handle_owners.get(&identity) {
            if tracked
                .upgrade()
                .is_some_and(|tracked| Arc::ptr_eq(&tracked, &handle.instance))
            {
                return Err(ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id: owner.clone(),
                });
            }
        }
        state.handle_owners.insert(
            identity,
            (
                router_session_id.to_string(),
                Arc::downgrade(&handle.instance),
            ),
        );
        state
            .by_session
            .entry(router_session_id.to_string())
            .or_default()
            .push(handle);
        Ok(())
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
        let handles = {
            let mut state = self
                .state
                .lock()
                .expect("actor instance session tracker lock poisoned");
            let handles = state
                .by_session
                .remove(router_session_id)
                .unwrap_or_default();
            handles
        };
        self.store.discard_exact_batch(&handles)
    }

    /// Runtime shutdown discards all volatile state, never registry bootstrap.
    pub fn discard_all(&self) -> usize {
        let handles = {
            let mut state = self
                .state
                .lock()
                .expect("actor instance session tracker lock poisoned");
            state
                .by_session
                .drain()
                .flat_map(|(_, handles)| handles)
                .collect::<Vec<_>>()
        };
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
    #[error("Actor instance handle is already tracked by Router session {owner_session_id}")]
    AlreadyTracked { owner_session_id: String },
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
    pub fn activate_with_created(
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
        let materialized = materialize_instance(&request, declaration)?;
        let instance = Arc::new(materialized.instance);
        instance
            .state
            .lock()
            .expect("actor instance state lock poisoned")
            .heap = materialized.heap;

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
    /// A synchronous segment that already owns an execution lease is allowed to
    /// finish and commit. Once that segment reaches a real suspension point, its
    /// next acquire observes this fence and exits instead of resuming.
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
        let matches = state
            .instances
            .get(&handle.fence.incarnation)
            .is_some_and(|instance| {
                ensure_instance_fence(instance, &handle.fence).is_ok()
                    && Arc::ptr_eq(instance, &handle.instance)
                    && instance.upgrading.load(Ordering::Acquire)
            });
        if !matches {
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
    /// again.
    pub fn discard_exact_batch(&self, handles: &[ActorInstanceHandle]) -> usize {
        let mut state = self
            .state
            .lock()
            .expect("actor instance store lock poisoned");
        let mut removed = 0;
        for handle in handles {
            let matches = state
                .instances
                .get(&handle.fence.incarnation)
                .is_some_and(|instance| {
                    ensure_instance_fence(instance, &handle.fence).is_ok()
                        && Arc::ptr_eq(instance, &handle.instance)
                });
            if matches {
                state.instances.remove(&handle.fence.incarnation);
                handle.instance.admission_notify.notify_waiters();
                removed += 1;
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

    /// The later Actor executor enters field state through this exact-fence
    /// gate. The method is crate-private so ordinary host/request consumers
    /// cannot turn an opaque handle into field access.
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
        let ActorInstanceState { fields, heap } = &mut *state;
        Ok(operation(fields, heap))
    }

    /// Waits for this exact incarnation's scheduler without holding a standard
    /// mutex across the await, then revalidates every fence before snapshotting.
    pub(crate) async fn acquire_execution(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<ActorInstanceExecutionLease, ActorInstanceStoreError> {
        let instance = self.wait_until_admitted(handle).await?;
        let scheduler_guard = Arc::clone(&instance.scheduler).lock_owned().await;
        let instance = self.resolve_active_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let nonce = instance
            .next_execution_token
            .fetch_add(1, Ordering::Relaxed);
        let token = Arc::new(ActorExecutionToken {
            nonce,
            active: AtomicBool::new(true),
        });
        let fields = Arc::new(Mutex::new(state.fields.clone()));
        let heap = state.heap.clone();
        drop(state);
        Ok(ActorInstanceExecutionLease {
            instance,
            _scheduler_guard: scheduler_guard,
            token,
            fields,
            heap: Some(heap),
        })
    }

    /// Scheduler acquisition for the platform activation path (`create`).
    /// The instance is intentionally not admitted yet, so this path skips the
    /// admission gate that ordinary method execution waits on.
    pub(crate) async fn acquire_execution_for_activation(
        &self,
        _authority: &ActorExecutorAuthority,
        handle: &ActorInstanceHandle,
    ) -> Result<ActorInstanceExecutionLease, ActorInstanceStoreError> {
        let instance = self.resolve_active_instance(handle)?;
        let scheduler_guard = Arc::clone(&instance.scheduler).lock_owned().await;
        let instance = self.resolve_active_instance(handle)?;
        let state = instance
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        let nonce = instance
            .next_execution_token
            .fetch_add(1, Ordering::Relaxed);
        let token = Arc::new(ActorExecutionToken {
            nonce,
            active: AtomicBool::new(true),
        });
        let fields = Arc::new(Mutex::new(state.fields.clone()));
        let heap = state.heap.clone();
        drop(state);
        Ok(ActorInstanceExecutionLease {
            instance,
            _scheduler_guard: scheduler_guard,
            token,
            fields,
            heap: Some(heap),
        })
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
            let instance = self.resolve_active_instance(handle)?;
            if instance.admitted.load(Ordering::Acquire) {
                return Ok(instance);
            }
            notified.await;
        }
    }

    pub(crate) fn commit_execution(
        &self,
        handle: &ActorInstanceHandle,
        lease: ActorInstanceExecutionLease,
        heap: RequestHeap,
    ) -> Result<(), ActorInstanceStoreError> {
        lease.token.ensure_active()?;
        let current = self.resolve_current_instance(handle)?;
        if !Arc::ptr_eq(&current, &lease.instance) {
            return Err(ActorInstanceStoreError::InstanceReplaced);
        }
        let fields = lease
            .fields
            .lock()
            .expect("actor execution fields lock poisoned")
            .clone();
        let mut state = current
            .state
            .lock()
            .expect("actor instance state lock poisoned");
        // The nonce is private and monotonically issued by this instance. This
        // check also makes accidental cross-instance frame reuse fail closed.
        if lease.token.nonce == 0
            || lease.token.nonce >= current.next_execution_token.load(Ordering::Acquire)
        {
            return Err(ActorInstanceStoreError::ExecutionTokenInvalid);
        }
        state.fields = fields;
        state.heap = heap;
        lease.token.active.store(false, Ordering::Release);
        Ok(())
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
    heap: RequestHeap,
}

fn materialize_instance(
    request: &ActorActivationRequest<'_>,
    declaration: &LinkedActorDeclaration,
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
    let mut heap = RequestHeap::default();
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
                &request.fence.incarnation.logical_key.canonical_actor_id_key_bytes,
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
            state: Mutex::new(ActorInstanceState {
                fields,
                heap: RequestHeap::default(),
            }),
            scheduler: Arc::new(tokio::sync::Mutex::new(())),
            next_execution_token: AtomicU64::new(1),
            upgrading: AtomicBool::new(false),
            admitted: AtomicBool::new(false),
            admission_notify: tokio::sync::Notify::new(),
        },
        heap,
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
    #[error("Actor execution token is expired")]
    ExecutionTokenExpired,
    #[error("Actor execution token is invalid for this instance")]
    ExecutionTokenInvalid,
}

#[cfg(test)]
mod tests;
