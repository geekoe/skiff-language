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
            return Ok(ActorInstanceHandle {
                fence: request.fence,
                instance: Arc::clone(existing),
            });
        }

        let declaration =
            resolve_actor_declaration(request.program, &request.fence.declaration_owner)?;
        validate_declaration_fence(declaration, &request.fence)?;
        let instance = Arc::new(materialize_instance(&request, declaration)?);

        state
            .latest_epochs
            .entry(request.fence.incarnation.logical_key.clone())
            .and_modify(|epoch| *epoch = (*epoch).max(request.fence.incarnation.epoch))
            .or_insert(request.fence.incarnation.epoch);
        state
            .instances
            .insert(request.fence.incarnation.clone(), Arc::clone(&instance));

        Ok(ActorInstanceHandle {
            fence: request.fence,
            instance,
        })
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

fn materialize_instance(
    request: &ActorActivationRequest<'_>,
    declaration: &LinkedActorDeclaration,
) -> Result<ActorInstance, ActorInstanceStoreError> {
    if request.bootstrap_encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
        return Err(ActorInstanceStoreError::UnsupportedBootstrapEncoding {
            actual: request.bootstrap_encoding_version.to_string(),
        });
    }
    let payload: Value = serde_json::from_slice(request.bootstrap_payload).map_err(|error| {
        ActorInstanceStoreError::BootstrapDecode {
            message: error.to_string(),
        }
    })?;
    let object = payload
        .as_object()
        .ok_or(ActorInstanceStoreError::BootstrapNotRecord)?;

    let mut canonical_field_names = declaration
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    canonical_field_names.sort_unstable();
    let actual_names = object.keys().map(String::as_str).collect::<Vec<_>>();
    if actual_names != canonical_field_names {
        return Err(ActorInstanceStoreError::BootstrapFieldShape {
            expected: canonical_field_names
                .into_iter()
                .map(str::to_string)
                .collect(),
            actual: actual_names.into_iter().map(str::to_string).collect(),
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
        let value = RuntimeBoundaryCodec::new(
            &plan,
            BoundaryUse::NativeArg,
            format!("Actor bootstrap field {}", field.name),
        )
        .from_wire_json(
            object
                .get(&field.name)
                .expect("exact bootstrap field shape checked"),
            &mut heap,
        )
        .map_err(|error| ActorInstanceStoreError::BootstrapFieldDecode {
            field: field.name.clone(),
            message: error.to_string(),
        })?;
        fields.push(ActorFieldValue {
            name: field.name.clone(),
            value,
        });
    }
    Ok(ActorInstance {
        fence: request.fence.clone(),
        state: Mutex::new(ActorInstanceState { fields, heap }),
        scheduler: Arc::new(tokio::sync::Mutex::new(())),
        next_execution_token: AtomicU64::new(1),
        upgrading: AtomicBool::new(false),
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
    #[error("Actor bootstrap payload decode failed: {message}")]
    BootstrapDecode { message: String },
    #[error("Actor bootstrap payload must be a record")]
    BootstrapNotRecord,
    #[error("Actor bootstrap field shape/order mismatch: expected {expected:?}, got {actual:?}")]
    BootstrapFieldShape {
        expected: Vec<String>,
        actual: Vec<String>,
    },
    #[error("Actor field {field} uses an unsupported encoding")]
    UnsupportedFieldEncoding { field: String },
    #[error("Actor field {field} type plan failed: {message}")]
    DeclarationType { field: String, message: String },
    #[error("Actor bootstrap field {field} decode failed: {message}")]
    BootstrapFieldDecode { field: String, message: String },
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
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    use serde_json::json;
    use skiff_runtime_linked_program::{
        ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorField,
        LinkedFileUnit, RuntimeExecutionPackage, RuntimeTypeContext, ServiceSymbolRef,
        SourceMapDto,
    };

    use super::*;
    use crate::actor_executor::ActorExecutionFrame;

    struct ProgramFixture {
        service_files: Vec<Arc<LinkedFileUnit>>,
        packages: Vec<Arc<RuntimeExecutionPackage>>,
        overlay: LinkOverlay,
        types: RuntimeTypeContext,
    }

    impl ProgramFixture {
        fn view(&self) -> ProgramTypeView<'_> {
            ProgramTypeView::new(
                &self.service_files,
                &self.packages,
                &self.overlay,
                &self.types,
            )
        }
    }

    fn owner() -> LinkedActorDeclarationOwner {
        LinkedActorDeclarationOwner {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity("file:actors".to_string()),
            actor_symbol: "DocHub".to_string(),
        }
    }

    fn abi() -> ActorAbiIdentity {
        ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:doc-hub")
    }

    fn implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:doc-hub")
    }

    fn fixture() -> ProgramFixture {
        let declaration_owner = owner();
        ProgramFixture {
            service_files: vec![Arc::new(LinkedFileUnit {
                schema_version: "skiff-file-ir-v3".to_string(),
                file_ir_identity: "file:actors".to_string(),
                source_ast_hash: "source:actors".to_string(),
                module_path: "actors".to_string(),
                ir_format_version: None,
                opcode_table_version: None,
                source_map: SourceMapDto::default(),
                declarations: FileDeclarations::default(),
                link_targets: FileLinkTargets::default(),
                actor_declarations: vec![LinkedActorDeclaration {
                    actor_type: ServiceSymbolRef {
                        module_path: "actors".to_string(),
                        symbol: "DocHub".to_string(),
                    },
                    implementation_owner: Some(declaration_owner),
                    actor_abi_identity: abi(),
                    actor_implementation_identity: implementation(),
                    actor_name: "DocHub".to_string(),
                    actor_id_type: builtin("string"),
                    fields: vec![
                        LinkedActorField {
                            name: "count".to_string(),
                            ty: builtin("integer"),
                            encoding: ActorFieldEncodingIr::CanonicalValueV1,
                        },
                        LinkedActorField {
                            name: "title".to_string(),
                            ty: builtin("string"),
                            encoding: ActorFieldEncodingIr::CanonicalValueV1,
                        },
                    ],
                    public_methods: Vec::new(),
                    actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
                }],
                types: Vec::new(),
                constants: Vec::new(),
                executables: Vec::new(),
                external_refs: ExternalRefTable::default(),
            })],
            packages: Vec::new(),
            overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn builtin(name: &str) -> skiff_runtime_linked_program::LinkedTypeRef {
        skiff_runtime_linked_program::LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn logical_key() -> ActorLogicalKey {
        let canonical_actor_id_key_bytes = br#""doc-1""#.to_vec();
        ActorLogicalKey {
            service_id: "skiff.run/docs".to_string(),
            actor_type_identity: "service-symbol:actors.DocHub".to_string(),
            actor_id_type_identity: "builtin:string".to_string(),
            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            actor_id_hash: format!(
                "sha256:{}",
                hex::encode(Sha256::digest(&canonical_actor_id_key_bytes))
            ),
            canonical_actor_id_key_bytes,
        }
    }

    fn fence(epoch: u64) -> ActorInstanceFence {
        ActorInstanceFence {
            incarnation: ActorIncarnationKey {
                logical_key: logical_key(),
                epoch,
            },
            actor_abi_identity: abi(),
            actor_implementation_identity: implementation(),
            declaration_owner: owner(),
        }
    }

    fn payload() -> Vec<u8> {
        br#"{"count":7,"title":"first"}"#.to_vec()
    }

    fn request<'a>(
        program: ProgramTypeView<'a>,
        fence: ActorInstanceFence,
        payload: &'a [u8],
    ) -> ActorActivationRequest<'a> {
        ActorActivationRequest {
            fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: payload,
            program,
        }
    }

    fn fence_for_id(id: &str, epoch: u64) -> ActorInstanceFence {
        let mut result = fence(epoch);
        let value = serde_json::to_value(id).unwrap();
        let bytes = canonical_json_bytes(&value).unwrap();
        result.incarnation.logical_key.actor_id_hash =
            format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
        result.incarnation.logical_key.canonical_actor_id_key_bytes = bytes;
        result
    }

    #[test]
    fn real_linked_declaration_materializes_field_frame_in_declaration_order() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .expect("valid bootstrap materializes");

        let fields = store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields.to_vec()
            })
            .unwrap();
        assert_eq!(
            fields,
            vec![
                ActorFieldValue {
                    name: "count".to_string(),
                    value: RuntimeValue::Number(7.0),
                },
                ActorFieldValue {
                    name: "title".to_string(),
                    value: RuntimeValue::String("first".to_string()),
                },
            ]
        );
    }

    #[test]
    fn concurrent_activation_publishes_exactly_one_instance() {
        let fixture = fixture();
        let bytes = payload();
        let store = Arc::new(ActorInstanceStore::new());
        let barrier = Arc::new(Barrier::new(12));
        let pointers = thread::scope(|scope| {
            let joins = (0..12)
                .map(|_| {
                    let store = Arc::clone(&store);
                    let barrier = Arc::clone(&barrier);
                    let program = fixture.view();
                    let bytes = bytes.as_slice();
                    scope.spawn(move || {
                        barrier.wait();
                        let handle = store
                            .activate(request(program, fence(1), bytes))
                            .expect("concurrent activation succeeds");
                        Arc::as_ptr(&handle.instance) as usize
                    })
                })
                .collect::<Vec<_>>();
            joins
                .into_iter()
                .map(|join| join.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(pointers.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn execution_lease_serializes_one_instance_but_not_another() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let first = store
            .activate(request(fixture.view(), fence_for_id("first", 1), &bytes))
            .unwrap();
        let second = store
            .activate(request(fixture.view(), fence_for_id("second", 1), &bytes))
            .unwrap();
        let authority = ActorExecutorAuthority::new();

        let first_lease = store.acquire_execution(&authority, &first).await.unwrap();
        assert!(first.instance.scheduler.try_lock().is_err());
        let second_lease = store.acquire_execution(&authority, &second).await.unwrap();
        assert!(second.instance.scheduler.try_lock().is_err());

        drop(first_lease);
        assert!(first.instance.scheduler.try_lock().is_ok());
        assert!(second.instance.scheduler.try_lock().is_err());
        drop(second_lease);
    }

    #[tokio::test]
    async fn failed_execution_snapshot_does_not_change_live_fields() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let authority = ActorExecutorAuthority::new();
        let lease = store.acquire_execution(&authority, &handle).await.unwrap();
        let execution_fields = lease.fields();
        execution_fields.lock().unwrap()[0].value = RuntimeValue::Number(99.0);
        drop(lease);

        let count = store
            .with_fields_for_executor(&authority, &handle, |fields, _| fields[0].value.clone())
            .unwrap();
        assert_eq!(count, RuntimeValue::Number(7.0));
    }

    #[tokio::test]
    async fn execution_frame_rejects_wrong_field_type_and_expires_with_lease() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let authority = ActorExecutorAuthority::new();
        let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
        let mut heap = lease.take_heap();
        let plan = RuntimeTypePlan::from_linked(
            &builtin("integer"),
            &PlanContext::from_type_view(fixture.view(), &ExecutableAddr::service(0, 0)),
        )
        .unwrap();
        let frame = ActorExecutionFrame::new(
            store.clone(),
            handle,
            lease,
            vec![("count".to_string(), plan)],
        );
        let error = frame
            .write_field(
                "count",
                &builtin("integer"),
                fixture.view(),
                &ExecutableAddr::service(0, 0),
                &RuntimeValue::String("wrong".to_string()),
                &mut heap,
            )
            .unwrap_err();
        assert!(error.to_string().contains("Actor self field count"));
        assert_eq!(
            frame.read_field("count").unwrap(),
            RuntimeValue::Number(7.0)
        );
        frame.suspend(&heap).unwrap();
        assert!(frame.read_field("count").is_err());
    }

    #[test]
    fn malformed_field_shapes_and_types_fail_without_caching() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        for malformed in [
            br#"{"title":"missing"}"#.as_slice(),
            br#"{"count":7,"extra":true,"title":"many"}"#.as_slice(),
            br#"{"title":"wrong-order","count":7}"#.as_slice(),
            br#"{"count":"wrong-type","title":"bad"}"#.as_slice(),
        ] {
            assert!(store
                .activate(request(fixture.view(), fence(1), malformed))
                .is_err());
            assert!(store.is_empty());
        }

        let bytes = payload();
        assert!(store
            .activate(request(fixture.view(), fence(1), &bytes))
            .is_ok());
    }

    #[test]
    fn declaration_and_identity_fences_fail_closed() {
        let fixture = fixture();
        let bytes = payload();
        let cases = [
            {
                let mut value = fence(1);
                value.actor_abi_identity = ActorAbiIdentity::new("wrong");
                value
            },
            {
                let mut value = fence(1);
                value.actor_implementation_identity = ActorImplementationIdentity::new("wrong");
                value
            },
            {
                let mut value = fence(1);
                value.declaration_owner.actor_symbol = "Other".to_string();
                value
            },
            fence(0),
        ];
        for bad_fence in cases {
            let store = ActorInstanceStore::new();
            assert!(store
                .activate(request(fixture.view(), bad_fence, &bytes))
                .is_err());
            assert!(store.is_empty());
        }

        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        assert_eq!(
            store
                .activate(request(fixture.view(), fence(1), &bytes))
                .unwrap_err(),
            ActorInstanceStoreError::StaleEpoch {
                requested: 1,
                latest: 2
            }
        );
    }

    #[test]
    fn existing_incarnation_rejects_different_owner_abi_or_implementation() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();

        for bad_fence in [
            {
                let mut value = fence(1);
                value.actor_abi_identity = ActorAbiIdentity::new("different");
                value
            },
            {
                let mut value = fence(1);
                value.actor_implementation_identity = ActorImplementationIdentity::new("different");
                value
            },
            {
                let mut value = fence(1);
                value.declaration_owner.file = FileAddr::FileIrIdentity("file:other".to_string());
                value
            },
        ] {
            assert_eq!(
                store
                    .activate(request(fixture.view(), bad_fence, &bytes))
                    .unwrap_err(),
                ActorInstanceStoreError::FenceMismatch
            );
        }
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn unknown_encoding_and_invalid_json_leave_no_partial_instance() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        assert_eq!(
            store
                .activate(ActorActivationRequest {
                    fence: fence(1),
                    bootstrap_encoding_version: "unknown",
                    bootstrap_payload: &payload(),
                    program: fixture.view(),
                })
                .unwrap_err(),
            ActorInstanceStoreError::UnsupportedBootstrapEncoding {
                actual: "unknown".to_string()
            }
        );
        assert!(store
            .activate(request(fixture.view(), fence(1), b"{"))
            .is_err());
        assert!(store.is_empty());
    }

    #[test]
    fn discard_requires_exact_fence_and_old_fence_cannot_remove_new_incarnation() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let old = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let new = store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        assert_eq!(
            store
                .with_fields_for_executor(&ActorExecutorAuthority::new(), &old, |fields, _| fields
                    .len())
                .unwrap_err(),
            ActorInstanceStoreError::StaleEpoch {
                requested: 1,
                latest: 2
            }
        );

        let mut forged = new.clone();
        forged.fence.actor_implementation_identity = ActorImplementationIdentity::new("different");
        assert!(!store.discard_exact(&forged));
        assert!(store.discard_exact(&old));
        assert_eq!(store.len(), 1);
        assert!(!store.discard_exact(&old));
        assert!(store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &new, |fields, _| fields
                .len())
            .is_ok());
    }

    #[tokio::test]
    async fn upgrade_fence_allows_owned_sync_segment_to_commit_but_blocks_next_acquire() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let authority = ActorExecutorAuthority::new();
        let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
        lease.fields().lock().unwrap()[0].value = RuntimeValue::Number(12.0);
        let heap = lease.take_heap();

        assert!(store.begin_upgrade_exact(&handle));
        store.commit_execution(&handle, lease, heap).unwrap();
        assert!(matches!(
            store.acquire_execution(&authority, &handle).await,
            Err(ActorInstanceStoreError::InstanceReplaced)
        ));
        assert_eq!(
            store
                .with_fields_for_executor(&authority, &handle, |fields, _| {
                    fields[0].value.clone()
                })
                .unwrap(),
            RuntimeValue::Number(12.0)
        );
    }

    #[tokio::test]
    async fn suspended_continuation_cannot_resume_after_upgrade_fence() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let authority = ActorExecutorAuthority::new();
        let mut lease = store.acquire_execution(&authority, &handle).await.unwrap();
        let heap = lease.take_heap();
        store.commit_execution(&handle, lease, heap).unwrap();

        assert!(store.begin_upgrade_exact(&handle));
        assert!(matches!(
            store.acquire_execution(&authority, &handle).await,
            Err(ActorInstanceStoreError::InstanceReplaced)
        ));
    }

    #[test]
    fn upgrade_discard_is_exact_idempotent_and_new_epoch_rebuilds_from_bootstrap() {
        let fixture = fixture();
        let original_bootstrap = payload();
        let replacement_bootstrap = br#"{"count":3,"title":"replacement"}"#.to_vec();
        let store = ActorInstanceStore::new();
        let old = store
            .activate(request(fixture.view(), fence(1), &original_bootstrap))
            .unwrap();
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &old, |fields, _| {
                fields[0].value = RuntimeValue::Number(99.0);
            })
            .unwrap();

        let mut forged = old.clone();
        forged.fence.actor_implementation_identity = ActorImplementationIdentity::new("different");
        assert!(!store.begin_upgrade_exact(&forged));
        assert!(!store.discard_upgrading_exact(&old));
        assert!(store.begin_upgrade_exact(&old));
        assert!(store.discard_upgrading_exact(&old));
        assert!(!store.discard_upgrading_exact(&old));
        assert_eq!(
            store
                .activate(request(fixture.view(), fence(1), &original_bootstrap))
                .unwrap_err(),
            ActorInstanceStoreError::StaleEpoch {
                requested: 1,
                latest: 2
            }
        );

        let replacement = store
            .activate(request(fixture.view(), fence(2), &replacement_bootstrap))
            .unwrap();
        let fields = store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &replacement, |fields, _| {
                fields.to_vec()
            })
            .unwrap();
        assert_eq!(fields[0].value, RuntimeValue::Number(3.0));
        assert_eq!(
            fields[1].value,
            RuntimeValue::String("replacement".to_string())
        );
    }

    #[test]
    fn stale_cleanup_handle_cannot_remove_same_epoch_rematerialization() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        let old = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        assert!(store.discard_exact(&old));
        let current = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        assert!(!store.discard_exact(&old));
        assert!(store.discard_exact(&current));
    }

    #[test]
    fn stale_session_cleanup_cannot_remove_same_epoch_rematerialization() {
        let fixture = fixture();
        let bytes = payload();
        let store = Arc::new(ActorInstanceStore::new());
        let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
        let old = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let delayed_old_handle = old.clone();
        tracker.track("old-session", old).unwrap();

        assert_eq!(tracker.discard_session("old-session"), 1);
        assert_eq!(
            tracker
                .track("new-session", delayed_old_handle)
                .unwrap_err(),
            ActorInstanceSessionTrackError::AlreadyTracked {
                owner_session_id: "old-session".to_string()
            }
        );
        let current = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        tracker.track("new-session", current.clone()).unwrap();

        assert_eq!(tracker.discard_session("old-session"), 0);
        assert_eq!(store.len(), 1);
        assert!(store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &current, |fields, _| fields
                .len())
            .is_ok());
    }

    #[test]
    fn session_tracker_rejects_duplicate_ownership_and_shutdown_discards_all() {
        let fixture = fixture();
        let bytes = payload();
        let store = Arc::new(ActorInstanceStore::new());
        let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
        let first = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        tracker.track("session-a", first.clone()).unwrap();
        assert_eq!(
            tracker.track("session-b", first).unwrap_err(),
            ActorInstanceSessionTrackError::AlreadyTracked {
                owner_session_id: "session-a".to_string()
            }
        );

        let second = store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        tracker.track("session-b", second).unwrap();
        assert_eq!(tracker.discard_all(), 2);
        assert_eq!(tracker.discard_all(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn session_upgrade_control_is_exact_and_stale_notifications_are_inert() {
        let fixture = fixture();
        let bytes = payload();
        let store = Arc::new(ActorInstanceStore::new());
        let tracker = ActorInstanceSessionTracker::new(Arc::clone(&store));
        let handle = store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        tracker.track("owner-session", handle.clone()).unwrap();

        let mut wrong_epoch = handle.fence().clone();
        wrong_epoch.incarnation.epoch = 2;
        assert!(!tracker.begin_upgrade_exact("stale-session", handle.fence()));
        assert!(!tracker.begin_upgrade_exact("owner-session", &wrong_epoch));
        assert!(tracker.begin_upgrade_exact("owner-session", handle.fence()));
        assert!(tracker.discard_upgrading_exact("owner-session", handle.fence()));
        assert!(!tracker.discard_upgrading_exact("owner-session", handle.fence()));
        assert!(!tracker.discard_upgrading_exact("stale-session", handle.fence()));
        assert!(store.is_empty());
        {
            let tracked = tracker.state.lock().unwrap();
            assert!(!tracked.by_session.contains_key("owner-session"));
            assert!(tracked.handle_owners.is_empty());
        }

        let replacement = store
            .activate(request(fixture.view(), fence(2), &bytes))
            .unwrap();
        tracker
            .track("replacement-session", replacement)
            .expect("upgrade discard releases old tracker ownership");
    }

    #[test]
    fn live_field_mutation_never_changes_registry_bootstrap_bytes() {
        let fixture = fixture();
        let registry_payload = payload();
        let original = registry_payload.clone();
        let store = ActorInstanceStore::new();
        let handle = store
            .activate(request(fixture.view(), fence(1), &registry_payload))
            .unwrap();
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields[0].value = RuntimeValue::Number(99.0);
            })
            .unwrap();
        assert_eq!(registry_payload, original);
        let count = store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _heap| {
                fields[0].value.clone()
            })
            .unwrap();
        assert_eq!(count, RuntimeValue::Number(99.0));
    }

    #[test]
    fn logical_key_is_part_of_the_incarnation_identity() {
        let fixture = fixture();
        let bytes = payload();
        let store = ActorInstanceStore::new();
        store
            .activate(request(fixture.view(), fence(1), &bytes))
            .unwrap();
        let mut other = fence(1);
        other.incarnation.logical_key.canonical_actor_id_key_bytes =
            serde_json::to_vec(&json!("doc-2")).unwrap();
        other.incarnation.logical_key.actor_id_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                &other.incarnation.logical_key.canonical_actor_id_key_bytes
            ))
        );
        store
            .activate(request(fixture.view(), other, &bytes))
            .unwrap();
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn empty_logical_key_components_are_rejected() {
        let fixture = fixture();
        let bytes = payload();
        let mut invalid = fence(1);
        invalid.incarnation.logical_key.service_id.clear();
        assert!(matches!(
            ActorInstanceStore::new().activate(request(fixture.view(), invalid, &bytes)),
            Err(ActorInstanceStoreError::InvalidLogicalKey { .. })
        ));
    }

    #[test]
    fn bootstrap_object_order_is_canonical_not_declaration_storage_order() {
        let fixture = fixture();
        let store = ActorInstanceStore::new();
        let mut object = serde_json::Map::new();
        object.insert("title".to_string(), json!("late"));
        object.insert("count".to_string(), json!(1));
        let non_canonical = serde_json::to_vec(&Value::Object(object)).unwrap();
        assert!(matches!(
            store.activate(request(fixture.view(), fence(1), &non_canonical)),
            Err(ActorInstanceStoreError::BootstrapFieldShape { .. })
        ));
    }

    #[test]
    fn declaration_without_exact_owner_is_rejected() {
        let mut fixture = fixture();
        fixture.service_files[0] = Arc::new({
            let mut file = fixture.service_files[0].as_ref().clone();
            file.actor_declarations[0].implementation_owner = None;
            file
        });
        let bytes = payload();
        assert_eq!(
            ActorInstanceStore::new()
                .activate(request(fixture.view(), fence(1), &bytes))
                .unwrap_err(),
            ActorInstanceStoreError::DeclarationMissing
        );
    }
}
