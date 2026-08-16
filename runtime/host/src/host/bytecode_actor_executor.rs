//! Host-owned Actor get/create/method execution for the flat VM child lane.
//!
//! The request crate owns the child mux and the scheduler owns the flat
//! trampoline. This module supplies the exact build/arena fence and keeps the
//! concrete Actor instance heap behind `ActorVmArena` lifecycle facts.

use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{ActorAbiIdentity, LiteralIr, TypeRefIr};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::vm_materialize::{materialize_linked_value, release_boundary_source};
use skiff_runtime_linked_bytecode::{
    FrozenConstantNodeIndex, LinkedActorMethodTarget, LinkedActorStateField,
    LinkedFrozenConstantValue, LinkedServiceBoundaryValue, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    actor_vm_arena::{
        ActorSegmentLease, ActorVmArena, ActorVmArenaEpoch, ActorVmArenaId, ActorVmArenaMemoryLease,
    },
    bytecode_execution_observation::BytecodeExecutionObserver,
    request_heap::RequestHeapLimits,
    runtime_value::ActorRef,
    vm_heap::{VmHeap, VmRecordField},
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};
use skiff_runtime_request::{
    BytecodeActorExecutor, BytecodeChildHeapFactory, RequestMemoryLedger, RequestVmHeap,
};
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildHandoff, BytecodeChildStart, BytecodeHandoff,
    BytecodePortFailure, BytecodeSchedulerError, ChildFinish, ChildFinishError,
    RequestResourceTable,
};
use skiff_runtime_vm::{
    AdapterInvocation, ChildInvocation, ChildTarget, ResumeOutcome, Vm, VmBudget, VmCompletion,
    VmControl, VmFiber, VmLimits, VmOwnedValues, VmResumeToken,
};

const ACTOR_ID_ENCODING_VERSION: &str = "skiff-canonical-v1";

static NEXT_ARENA_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_ARENA_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct ActorHostContext {
    pub(crate) request_heap_limits: RequestHeapLimits,
    pub(crate) arena_lease_root: String,
}

impl ActorHostContext {}

pub(crate) struct ProductionBytecodeActorExecutor {
    context: ActorHostContext,
    instances: ActorInstanceRegistry,
}

impl ProductionBytecodeActorExecutor {
    pub(crate) fn new(context: ActorHostContext) -> Self {
        Self {
            context,
            instances: ActorInstanceRegistry::default(),
        }
    }
}

impl BytecodeActorExecutor for ProductionBytecodeActorExecutor {
    fn arena_lease_root(&self) -> String {
        self.context.arena_lease_root.clone()
    }

    fn execute_get(
        &self,
        invocation: AdapterInvocation,
        parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
        _observer: BytecodeExecutionObserver,
        _limits: VmLimits,
    ) -> Result<
        BytecodeAdapterHandoff<VmFiber>,
        BytecodePortFailure<AdapterInvocation, VmResumeToken>,
    > {
        let image = Arc::clone(invocation.resume().image());
        let adapter = match image.host_effect_target(invocation.adapter()) {
            Some(adapter) => adapter,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor get adapter row is absent from the pinned image".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let signature = adapter.signature().clone();
        let [_key_type] = signature.parameter_types() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor get signature must carry exactly one key parameter".to_string(),
                ),
                invocation,
            ));
        };
        let [result_type] = signature.result_types() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor get signature must carry exactly one actor result".to_string(),
                ),
                invocation,
            ));
        };
        let source_arguments = invocation.arguments().values().to_vec();
        let key_value = match source_arguments.first() {
            Some(value) => *value,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "typed Actor get invocation is missing its key".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let key = match string_slot_value(&image, parent_heap, &key_value) {
            Ok(key) => key,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor get key materialization failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        let method = match self.exact_actor_method_for_result_type(&image, *result_type) {
            Some(method) => method,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor get result type has no exact actor method table row".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let actor_ref = match self.new_actor_ref(&image, method, &key) {
            Ok(actor_ref) => actor_ref,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor get identity construction failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        let actor_key = ActorKey::new(
            &actor_ref,
            image.owner().build_id().as_str(),
            method.actor_implementation(),
        );
        if let Err(error) = self
            .instances
            .get_or_create(actor_key, self.context.request_heap_limits.clone())
        {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(format!(
                    "Actor get instance creation failed: {error}"
                )),
                invocation,
            ));
        }

        let resume_type = match exact_resume_result_type(&image, &invocation.resume()) {
            Ok(resume_type) => resume_type,
            Err(error) => {
                return Err(BytecodePortFailure::input(error, invocation));
            }
        };
        let tag = match CompactTypeTag::try_from_type_index(resume_type.get()) {
            Some(tag) => tag,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor get resume type does not fit the compact type tag".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let parent_heap_typed = match parent_heap
            .as_any_mut()
            .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
        {
            Some(heap) => heap,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor get parent heap is not a request VM heap".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let actor_state_ref =
            match parent_heap_typed.alloc_actor_state_ref(actor_ref, tag, ValueFlags::new(0)) {
                Ok(actor_state_ref) => actor_state_ref,
                Err(error) => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(format!(
                            "Actor state ref allocation failed: {error}"
                        )),
                        invocation,
                    ));
                }
            };

        let (_, arguments, resume) = invocation.into_parts();
        if let Err(failure) = arguments.release(parent_heap) {
            return Err(BytecodePortFailure::terminal_host_arguments_release(
                failure,
            ));
        }
        let outcome =
            match VmOwnedValues::try_from_resume(&resume, vec![actor_state_ref].into_boxed_slice())
            {
                Ok(values) => ResumeOutcome::Values(values),
                Err(rejected) => {
                    let (error, escrow) = rejected.into_terminal_escrow();
                    let mut escrow = escrow;
                    let _ = escrow.release_all(parent_heap);
                    return Err(BytecodePortFailure::continuation(
                        BytecodeSchedulerError::Vm(error),
                        resume,
                    ));
                }
            };
        Ok(BytecodeAdapterHandoff::Ready(BytecodeHandoff {
            resume,
            outcome,
        }))
    }

    fn execute_method(
        &self,
        invocation: ChildInvocation,
        parent_heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
        child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
        resources: RequestResourceTable,
        memory_ledger: Arc<RequestMemoryLedger>,
        observer: BytecodeExecutionObserver,
        limits: VmLimits,
    ) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>>
    {
        let ChildTarget::Actor(index) = invocation.target() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                invocation,
            ));
        };
        let image = Arc::clone(invocation.resume().image());
        let target = match image
            .actor_methods()
            .get(index.get() as usize)
            .filter(|row| row.index() == index)
        {
            Some(target) => target,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor method table row is absent from the pinned image".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let [actor_type] = target.signature().parameter_types() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor method signature must carry its self parameter".to_string(),
                ),
                invocation,
            ));
        };
        let source_arguments = invocation.arguments().values().to_vec();
        let actor_value = match source_arguments.first() {
            Some(value) => *value,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor method invocation is missing its receiver".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let parent_heap_typed = match parent_heap
            .as_any()
            .and_then(|heap| heap.downcast_ref::<RequestVmHeap>())
        {
            Some(heap) => heap,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor method parent heap is not a request VM heap".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let actor_ref = match parent_heap_typed.actor_state_ref_value(&actor_value) {
            Ok(actor_ref) => actor_ref,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor method receiver identity read failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        let actor_key = ActorKey::new(
            &actor_ref,
            image.owner().build_id().as_str(),
            target.actor_implementation(),
        );
        let instance = match self.instances.get(&actor_key) {
            Some(instance) => instance,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor method receiver has no admitted instance".to_string(),
                    ),
                    invocation,
                ));
            }
        };

        let mut child_heap = match child_heap_factory.create_child_heap(
            image.owner(),
            self.context.request_heap_limits.clone(),
            resources,
            memory_ledger,
        ) {
            Ok(child_heap) => child_heap,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor child heap creation failed: {error}"
                    )),
                    invocation,
                ));
            }
        };

        let key_field = target.actor_implementation().key_field().to_string();
        let needs_create = instance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .state_root
            .is_none();
        let state_slot = if needs_create {
            match self.run_create_if_needed(
                &image,
                &target,
                &actor_ref,
                &key_field,
                &mut child_heap,
                &instance,
                *actor_type,
                budget,
                observer.clone(),
                limits,
            ) {
                Ok(state_slot) => state_slot,
                Err(error) => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(format!(
                            "Actor create execution failed: {error}"
                        )),
                        invocation,
                    ));
                }
            }
        } else {
            let mut guard = instance
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let state_root = guard
                .state_root
                .expect("actor state root is present when create is not needed");
            match materialize_actor_state(
                &mut guard.heap,
                &state_root,
                child_heap.heap_mut(),
                &image,
                *actor_type,
                target.actor_implementation().state_fields(),
            ) {
                Ok(state_slot) => state_slot,
                Err(error) => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(format!(
                            "Actor state materialization failed: {error}"
                        )),
                        invocation,
                    ));
                }
            }
        };
        let mut method_args = Vec::with_capacity(target.signature().parameter_types().len());
        method_args.push(state_slot);
        let parameter_boundaries = target.parameter_boundaries();
        if parameter_boundaries.len() != target.signature().parameter_types().len() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor method parameter boundary count drifts from its linked signature"
                        .to_string(),
                ),
                invocation,
            ));
        }
        for (index, ((source, parameter_type), boundary)) in source_arguments
            .iter()
            .skip(1)
            .zip(target.signature().parameter_types().iter().skip(1))
            .zip(parameter_boundaries.iter().skip(1))
            .enumerate()
        {
            let value = match materialize_linked_value(
                parent_heap,
                source,
                child_heap.heap_mut(),
                &image,
                *parameter_type,
                boundary,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(format!(
                            "Actor method parameter {index} materialization failed: {error}"
                        )),
                        invocation,
                    ));
                }
            };
            if let Err(error) = child_heap.publish_staging_root(value) {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor method parameter {index} staging failed: {error}"
                    )),
                    invocation,
                ));
            }
            method_args.push(value);
        }

        let entry = match image.function_entry(target.function()) {
            Ok(entry) => entry,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor method entry lookup failed: {error}"
                    )),
                    invocation,
                ));
            }
        };
        let fiber = match Vm::start_with_retained_parameter(
            entry,
            method_args.into_boxed_slice(),
            limits,
            observer.clone(),
        ) {
            Ok(fiber) => fiber,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Vm(error),
                    invocation,
                ));
            }
        };
        let segment_lease = match instance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .arena
            .acquire_segment()
        {
            Ok(segment_lease) => segment_lease,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!(
                        "Actor segment lease acquisition failed: {error}"
                    )),
                    invocation,
                ));
            }
        };

        if invocation.stream_endpoint().is_some() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor method child must not carry a stream endpoint".to_string(),
                ),
                invocation,
            ));
        }
        let resume_site = match image
            .resume_sites()
            .get(invocation.resume().resume_site())
            .filter(|site| {
                site.function() == invocation.resume().function()
                    && site.site() == invocation.resume().instruction()
            }) {
            Some(resume_site) => resume_site,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "Actor method resume site is absent from the pinned image".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let result_boundaries = target.result_boundaries();
        if result_boundaries.len() != resume_site.result_types().len() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "Actor method result boundary count drifts from its linked resume site"
                        .to_string(),
                ),
                invocation,
            ));
        }
        let (_, arguments, _endpoint, resume) = invocation.into_parts();
        for value in arguments.values() {
            if let Err(error) = release_boundary_source(parent_heap, value) {
                return Err(BytecodePortFailure::continuation(
                    BytecodeSchedulerError::Port(format!(
                        "Actor method argument source release failed: {error}"
                    )),
                    resume,
                ));
            }
        }
        let finish = ActorMethodChildFinish {
            registry: self.instances.clone(),
            actor_key,
            image: Arc::clone(&image),
            actor_type: *actor_type,
            state_slot,
            result_boundaries: result_boundaries.to_vec(),
            state_fields: target.actor_implementation().state_fields().to_vec(),
            segment_lease: Some(segment_lease),
        };
        Ok(BytecodeChildHandoff::Ready(BytecodeChildStart {
            unit: fiber,
            resume,
            child_heap,
            finish: Box::new(finish),
        }))
    }
}

struct ActorMethodChildFinish {
    registry: ActorInstanceRegistry,
    actor_key: ActorKey,
    image: Arc<DeploymentExecutionImage>,
    actor_type: TypeIndex,
    state_slot: ValueSlot,
    result_boundaries: Vec<LinkedServiceBoundaryValue>,
    state_fields: Vec<LinkedActorStateField>,
    segment_lease: Option<ActorSegmentLease>,
}

impl ChildFinish<VmFiber, VmResumeToken> for ActorMethodChildFinish {
    fn finish(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        let instance = self.registry.get(&self.actor_key).ok_or_else(|| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "Actor instance vanished before method finish".to_string(),
            ))
        })?;
        let mut guard = instance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let new_state = materialize_actor_state(
            child_heap.heap_mut(),
            &self.state_slot,
            &mut guard.heap,
            &self.image,
            self.actor_type,
            &self.state_fields,
        )
        .map_err(|error| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(format!(
                "Actor state writeback failed: {error}"
            )))
        })?;
        guard
            .replace_state_root(new_state)
            .map_err(ChildFinishError::failure)?;

        let (outcome, mut residual) = child_result.into_resume().map_err(|_| {
            ChildFinishError::failure(BytecodeSchedulerError::Port(
                "Actor method terminal failure cannot materialize to the caller".to_string(),
            ))
        })?;
        let outcome = match outcome {
            ResumeOutcome::Values(child_values) => {
                let mut caller_values = Vec::with_capacity(child_values.len());
                for (index, (source, plan)) in child_values
                    .values()
                    .iter()
                    .zip(self.result_boundaries.iter())
                    .enumerate()
                {
                    match materialize_linked_value(
                        child_heap.heap_mut(),
                        source,
                        parent_heap,
                        &self.image,
                        plan.caller_type(),
                        plan,
                    ) {
                        Ok(value) => caller_values.push(value),
                        Err(error) => {
                            for root in &caller_values {
                                let _ = parent_heap.release_snapshot(root);
                            }
                            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                                format!(
                                    "Actor method result {index} materialization failed: {error}"
                                ),
                            )));
                        }
                    }
                }
                let caller_values_owned =
                    VmOwnedValues::try_from_resume(resume, caller_values.into_boxed_slice())
                        .map_err(|rejected| {
                            let message = rejected.error().to_string();
                            for root in rejected.values() {
                                let _ = parent_heap.release_snapshot(root);
                            }
                            ChildFinishError::failure(BytecodeSchedulerError::Port(message))
                        })?;
                child_values
                    .into_terminal_escrow()
                    .release_all(child_heap.heap_mut())
                    .map_err(|error| {
                        ChildFinishError::failure(BytecodeSchedulerError::Vm(error))
                    })?;
                ResumeOutcome::Values(caller_values_owned)
            }
            other => other,
        };
        residual
            .release_all(child_heap.heap_mut())
            .map_err(|error| ChildFinishError::failure(BytecodeSchedulerError::Vm(error)))?;
        Ok(outcome)
    }
}

impl Drop for ActorMethodChildFinish {
    fn drop(&mut self) {
        if let Some(lease) = self.segment_lease.take() {
            lease.release();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ActorKey {
    service_id: String,
    actor_type_identity: String,
    actor_id_type_identity: String,
    actor_id_encoding_version: String,
    canonical_actor_id_key_bytes: Vec<u8>,
    actor_id_hash: String,
    build_id: String,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: skiff_artifact_model::ActorImplementationIdentity,
}

impl ActorKey {
    fn new(
        actor_ref: &ActorRef,
        build_id: &str,
        implementation: &skiff_runtime_linked_bytecode::LinkedActorImplementationRef,
    ) -> Self {
        Self {
            service_id: actor_ref.service_id().to_string(),
            actor_type_identity: actor_ref.actor_type_identity().to_string(),
            actor_id_type_identity: actor_ref.actor_id_type_identity().to_string(),
            actor_id_encoding_version: actor_ref.actor_id_encoding_version().to_string(),
            canonical_actor_id_key_bytes: actor_ref.canonical_actor_id_key_bytes().to_vec(),
            actor_id_hash: actor_ref.actor_id_hash().to_string(),
            build_id: build_id.to_string(),
            actor_abi_identity: implementation.actor_abi_identity().clone(),
            actor_implementation_identity: implementation.actor_implementation_identity().clone(),
        }
    }
}

struct ActorInstance {
    arena: ActorVmArena,
    heap: RequestVmHeap,
    state_root: Option<ValueSlot>,
    _memory_lease: ActorVmArenaMemoryLease,
}

impl ActorInstance {
    fn replace_state_root(&mut self, value: ValueSlot) -> Result<(), BytecodeSchedulerError> {
        if let Some(old) = self.state_root.take() {
            self.heap.release_snapshot(&old).map_err(|error| {
                BytecodeSchedulerError::Port(format!(
                    "Actor previous state release failed: {error}"
                ))
            })?;
        }
        self.state_root = Some(value);
        Ok(())
    }
}

#[derive(Clone, Default)]
struct ActorInstanceRegistry {
    inner: Arc<Mutex<HashMap<ActorKey, Arc<Mutex<ActorInstance>>>>>,
}

impl ActorInstanceRegistry {
    fn get(&self, key: &ActorKey) -> Option<Arc<Mutex<ActorInstance>>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn get_or_create(
        &self,
        key: ActorKey,
        limits: RequestHeapLimits,
    ) -> Result<Arc<Mutex<ActorInstance>>, String> {
        let mut instances = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(instance) = instances.get(&key) {
            return Ok(Arc::clone(instance));
        }
        let id = ActorVmArenaId::new(
            NonZeroU64::new(NEXT_ARENA_ID.fetch_add(1, Ordering::Relaxed))
                .ok_or_else(|| "actor arena id exhausted".to_string())?,
        );
        let epoch = ActorVmArenaEpoch::new(
            NonZeroU64::new(NEXT_ARENA_EPOCH.fetch_add(1, Ordering::Relaxed))
                .ok_or_else(|| "actor arena epoch exhausted".to_string())?,
        );
        let arena = ActorVmArena::new(id, id.get(), epoch, limits.max_estimated_bytes);
        let memory_lease = arena
            .reserve(limits.max_estimated_bytes)
            .map_err(|error| error.to_string())?;
        let instance = Arc::new(Mutex::new(ActorInstance {
            arena,
            heap: RequestVmHeap::new(limits),
            state_root: None,
            _memory_lease: memory_lease,
        }));
        instances.insert(key, Arc::clone(&instance));
        Ok(instance)
    }

    #[allow(dead_code)]
    fn remove(&self, key: &ActorKey) -> Result<(), String> {
        let instance = {
            let mut instances = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            instances
                .remove(key)
                .ok_or_else(|| "Actor instance is absent from the registry".to_string())?
        };
        let guard = instance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.arena.discard().map_err(|error| error.to_string())?;
        Ok(())
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl ProductionBytecodeActorExecutor {
    fn exact_actor_method_for_result_type<'a>(
        &self,
        image: &'a DeploymentExecutionImage,
        result_type: TypeIndex,
    ) -> Option<&'a LinkedActorMethodTarget> {
        let result_ref = image
            .types()
            .get(result_type.get() as usize)
            .filter(|entry| entry.index() == result_type)
            .map(|entry| entry.type_ref())?;
        let mut matches = image.actor_methods().iter().filter(|row| {
            row.signature().parameter_types().first().and_then(|ty| {
                image
                    .types()
                    .get(ty.get() as usize)
                    .filter(|entry| entry.index() == *ty)
                    .map(|entry| entry.type_ref())
            }) == Some(result_ref)
        });
        let first = matches.next()?;
        let implementation = first.actor_implementation();
        matches
            .all(|row| {
                row.actor_implementation().actor_abi_identity()
                    == implementation.actor_abi_identity()
                    && row.actor_implementation().actor_implementation_identity()
                        == implementation.actor_implementation_identity()
            })
            .then_some(first)
    }

    fn new_actor_ref(
        &self,
        image: &DeploymentExecutionImage,
        method: &LinkedActorMethodTarget,
        key: &str,
    ) -> Result<ActorRef, String> {
        let canonical_bytes = canonical_json_bytes(&Value::String(key.to_string()))
            .map_err(|error| error.to_string())?;
        let actor_id_hash = format!("sha256:{}", hex::encode(Sha256::digest(&canonical_bytes)));
        let implementation = method.actor_implementation();
        Ok(ActorRef::new(
            image.owner().deployment().service_id.clone(),
            implementation.actor_type_identity(),
            implementation.actor_id_type_identity(),
            ACTOR_ID_ENCODING_VERSION,
            canonical_bytes,
            actor_id_hash,
            Some(1),
        ))
    }

    fn run_create_if_needed(
        &self,
        image: &Arc<DeploymentExecutionImage>,
        method: &LinkedActorMethodTarget,
        actor_ref: &ActorRef,
        key_field: &str,
        child_heap: &mut skiff_runtime_scheduler::ChildHeapCarrier,
        instance: &Arc<Mutex<ActorInstance>>,
        actor_type: TypeIndex,
        budget: &mut dyn VmBudget,
        observer: BytecodeExecutionObserver,
        limits: VmLimits,
    ) -> Result<ValueSlot, String> {
        let create = image
            .actor_creates()
            .iter()
            .find(|row| {
                row.actor_implementation().actor_implementation_identity()
                    == method
                        .actor_implementation()
                        .actor_implementation_identity()
            })
            .ok_or_else(|| "actor create row is absent".to_string())?;
        let state = self.allocate_provisional_state(
            child_heap.heap_mut(),
            image.as_ref(),
            actor_type,
            key_field,
            actor_ref,
            method.actor_implementation().state_fields(),
        )?;
        let entry = image
            .function_entry(create.function())
            .map_err(|error| error.to_string())?;
        let (completion, state_after_create) = run_sync_fiber(
            entry,
            vec![state].into_boxed_slice(),
            child_heap.heap_mut(),
            budget,
            observer,
            limits,
        )?;
        let (outcome, mut residual) = completion
            .into_resume()
            .map_err(|_| "actor create terminal failure".to_string())?;
        let _ = residual.retain_root(&state_after_create);
        match outcome {
            ResumeOutcome::Values(values) => {
                values
                    .into_terminal_escrow()
                    .release_all(child_heap.heap_mut())
                    .map_err(|error| error.to_string())?;
            }
            ResumeOutcome::Empty => {}
            ResumeOutcome::Throw(_) => return Err("actor create threw".to_string()),
            ResumeOutcome::Failure(error) => {
                return Err(format!("actor create failed: {error}"));
            }
            ResumeOutcome::StreamEnd => {
                return Err("actor create returned a stream end".to_string())
            }
            ResumeOutcome::InternalTerminal(_) => {
                return Err("actor create hit an internal terminal".to_string());
            }
        }
        let mut guard = instance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let committed_state = materialize_actor_state(
            child_heap.heap_mut(),
            &state_after_create,
            &mut guard.heap,
            image.as_ref(),
            actor_type,
            method.actor_implementation().state_fields(),
        )
        .map_err(|error| error.to_string())?;
        guard
            .replace_state_root(committed_state)
            .map_err(|error| error.to_string())?;
        let state_root = guard.state_root.expect("state root was just committed");
        let fresh_state = materialize_actor_state(
            &mut guard.heap,
            &state_root,
            child_heap.heap_mut(),
            image.as_ref(),
            actor_type,
            method.actor_implementation().state_fields(),
        )
        .map_err(|error| error.to_string())?;
        drop(guard);
        residual
            .release_all(child_heap.heap_mut())
            .map_err(|error| error.to_string())?;
        Ok(fresh_state)
    }

    fn allocate_provisional_state(
        &self,
        heap: &mut dyn VmHeap,
        image: &DeploymentExecutionImage,
        actor_type: TypeIndex,
        key_field: &str,
        actor_ref: &ActorRef,
        state_fields: &[LinkedActorStateField],
    ) -> Result<ValueSlot, String> {
        let key_json: Value = serde_json::from_slice(actor_ref.canonical_actor_id_key_bytes())
            .map_err(|error| error.to_string())?;
        let mut fields = Vec::with_capacity(state_fields.len());
        for field in state_fields {
            let value = if field.name() == key_field {
                materialize_json_key(heap, image, field.boundary().caller_type(), &key_json)?
            } else {
                ValueSlot::null()
            };
            fields.push(VmRecordField {
                name: field.name().to_string(),
                value,
            });
        }
        let tag = CompactTypeTag::try_from_type_index(actor_type.get())
            .ok_or_else(|| "actor state type does not fit compact tag".to_string())?;
        heap.allocate_record(&fields, tag, ValueFlags::new(0))
            .map_err(|error| error.to_string())
    }
}

fn materialize_actor_state(
    source_heap: &mut dyn VmHeap,
    source: &ValueSlot,
    destination_heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    actor_type: TypeIndex,
    state_fields: &[LinkedActorStateField],
) -> Result<ValueSlot, String> {
    let mut fields = Vec::with_capacity(state_fields.len());
    for field in state_fields {
        let source_value = source_heap
            .record_field(source, field.name())
            .map_err(|error| error.to_string())?;
        let materialized = materialize_linked_value(
            source_heap,
            &source_value,
            destination_heap,
            image,
            field.boundary().caller_type(),
            field.boundary(),
        )
        .map_err(|error| error.to_string())?;
        fields.push(VmRecordField {
            name: field.name().to_string(),
            value: materialized,
        });
    }
    let tag = CompactTypeTag::try_from_type_index(actor_type.get())
        .ok_or_else(|| "actor state type does not fit compact tag".to_string())?;
    destination_heap
        .allocate_record(&fields, tag, ValueFlags::new(0))
        .map_err(|error| error.to_string())
}

fn string_slot_value(
    image: &DeploymentExecutionImage,
    heap: &dyn VmHeap,
    value: &ValueSlot,
) -> Result<String, String> {
    match value.kind() {
        Some(skiff_runtime_model::vm_value::ValueKind::ConstRef) => {
            let handle = value
                .as_const_ref()
                .ok_or_else(|| "actor key constant handle is invalid".to_string())?;
            let index = FrozenConstantNodeIndex::new(
                u32::try_from(handle.get())
                    .map_err(|_| "actor key constant handle does not fit u32".to_string())?,
            );
            let node = image
                .frozen_constant_nodes()
                .get(index.get() as usize)
                .filter(|node| node.index() == index)
                .ok_or_else(|| "actor key constant node is absent".to_string())?;
            match node.value() {
                LinkedFrozenConstantValue::Literal(LiteralIr::String { value }) => {
                    Ok(value.clone())
                }
                _ => Err("actor key constant is not a string literal".to_string()),
            }
        }
        _ => heap.string_value(value).map_err(|error| error.to_string()),
    }
}

fn materialize_json_key(
    heap: &mut dyn VmHeap,
    image: &DeploymentExecutionImage,
    type_index: TypeIndex,
    value: &Value,
) -> Result<ValueSlot, String> {
    let position =
        usize::try_from(type_index.get()).map_err(|_| "key type index overflow".to_string())?;
    let entry = image
        .types()
        .get(position)
        .filter(|entry| entry.index() == type_index)
        .ok_or_else(|| "key linked type is absent".to_string())?;
    let tag = CompactTypeTag::try_from_type_index(type_index.get())
        .ok_or_else(|| "key type does not fit compact tag".to_string())?;
    match (entry.type_ref(), value) {
        (TypeRefIr::Builtin { name, .. }, Value::String(value)) if name == "string" => heap
            .alloc_typed_string(value.clone(), tag, ValueFlags::new(0))
            .map_err(|error| error.to_string()),
        (TypeRefIr::Builtin { name, .. }, Value::Number(value)) if name == "integer" => value
            .as_i64()
            .map(ValueSlot::integer)
            .ok_or_else(|| "actor key integer is out of range".to_string()),
        (TypeRefIr::Builtin { name, .. }, Value::Number(value)) if name == "number" => {
            Ok(ValueSlot::number(value.as_f64().ok_or_else(|| {
                "actor key number is not finite".to_string()
            })?))
        }
        (TypeRefIr::Builtin { name, .. }, Value::Bool(value)) if name == "bool" => {
            Ok(ValueSlot::bool(*value))
        }
        (TypeRefIr::Builtin { name, .. }, Value::Null) if name == "null" => Ok(ValueSlot::null()),
        _ => Err(format!(
            "actor key JSON {:?} is not admitted for linked type {:?}",
            value,
            entry.type_ref()
        )),
    }
}

fn exact_resume_result_type(
    image: &DeploymentExecutionImage,
    resume: &VmResumeToken,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    let resume_site = image
        .resume_sites()
        .get(resume.resume_site())
        .filter(|site| site.function() == resume.function() && site.site() == resume.instruction())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "Actor get resume site is absent from the pinned image".to_string(),
            )
        })?;
    let [result_type] = resume_site.result_types() else {
        return Err(BytecodeSchedulerError::Port(
            "Actor get resume site has no exact single result".to_string(),
        ));
    };
    Ok(*result_type)
}

fn run_sync_fiber(
    entry: skiff_runtime_linker::DeploymentExecutionEntry,
    arguments: Box<[ValueSlot]>,
    heap: &mut dyn VmHeap,
    budget: &mut dyn VmBudget,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
) -> Result<(VmCompletion, ValueSlot), String> {
    let tracked = *arguments
        .first()
        .ok_or_else(|| "actor create tracked slot is absent".to_string())?;
    let mut fiber = Vm::start_with_retained_parameter(entry, arguments, limits, observer)
        .map_err(|error| error.to_string())?;
    let mut tracked = tracked;
    loop {
        match fiber.run_segment(heap, budget) {
            VmControl::Continue => {
                if let Ok(value) =
                    fiber.frame_slot_value(skiff_runtime_linked_bytecode::FrameSlotIndex::new(0))
                {
                    tracked = value;
                }
            }
            VmControl::Complete(completion) => {
                return Ok((completion, tracked));
            }
            _ => return Err("Actor create requires a coroutine or host handoff".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_artifact_model::ActorImplementationIdentity;

    #[test]
    fn actor_key_includes_build_and_abi_identity() {
        let base = ActorKey {
            service_id: "svc".to_string(),
            actor_type_identity: "Counter".to_string(),
            actor_id_type_identity: "string".to_string(),
            actor_id_encoding_version: ACTOR_ID_ENCODING_VERSION.to_string(),
            canonical_actor_id_key_bytes: br#""phase6""#.to_vec(),
            actor_id_hash: "sha256:actor-id".to_string(),
            build_id: "build-1".to_string(),
            actor_abi_identity: ActorAbiIdentity::new("abi-1"),
            actor_implementation_identity: ActorImplementationIdentity::new("impl-1"),
        };
        let stale_build = ActorKey {
            build_id: "build-0".to_string(),
            ..base.clone()
        };
        let stale_abi = ActorKey {
            actor_abi_identity: ActorAbiIdentity::new("abi-0"),
            ..base.clone()
        };
        let stale_implementation = ActorKey {
            actor_implementation_identity: ActorImplementationIdentity::new("impl-0"),
            ..base.clone()
        };
        assert_eq!(
            base,
            base.clone(),
            "the logical actor key itself must remain epoch-free"
        );
        assert_ne!(base, stale_build);
        assert_ne!(base, stale_abi);
        assert_ne!(base, stale_implementation);
    }

    #[test]
    fn instance_registry_remove_discards_quiescent_arena() {
        let registry = ActorInstanceRegistry::default();
        let key = ActorKey {
            service_id: "svc".to_string(),
            actor_type_identity: "Counter".to_string(),
            actor_id_type_identity: "string".to_string(),
            actor_id_encoding_version: ACTOR_ID_ENCODING_VERSION.to_string(),
            canonical_actor_id_key_bytes: br#""phase6""#.to_vec(),
            actor_id_hash: "sha256:actor-id".to_string(),
            build_id: "build-1".to_string(),
            actor_abi_identity: ActorAbiIdentity::new("abi-1"),
            actor_implementation_identity: ActorImplementationIdentity::new("impl-1"),
        };
        let instance = registry
            .get_or_create(key.clone(), RequestHeapLimits::default())
            .expect("instance creation");
        assert_eq!(registry.len(), 1);
        assert!(!instance.lock().unwrap().arena.snapshot().discarded);

        registry.remove(&key).expect("quiescent instance removes");
        assert_eq!(registry.len(), 0);
        assert!(instance.lock().unwrap().arena.snapshot().discarded);
    }
}
