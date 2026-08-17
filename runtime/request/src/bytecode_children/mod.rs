//! X6 child mux for flat-scheduler bytecode children.
//!
//! Service, local/remote interface, callback registration and the K6 DB
//! intrinsic child are the X6 child mux lanes. Actor remains fail-closed until
//! the A6 executor/arena seam supplies an executable callback, and task is
//! wired through the fresh request seam rather than a VM child target.

mod actor;
mod callback;
mod child_stream;
mod db;
mod db_intrinsic;
mod interface;
mod provider_receiver;
mod service;
mod task;

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use skiff_artifact_model::{ContractOperationId, Opcode, ServiceProtocolIdentity};
use skiff_runtime_boundary::service_linkable::ServiceLinkableCapabilityHooks;
use skiff_runtime_boundary::vm_materialize::materialize_linked_value;
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, ServiceDependencySlot};
use skiff_runtime_linked_bytecode::ServiceOperationIndex;
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver, request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_scheduler::{
    BoundaryOwnerRegistration, BytecodeChildHandoff, BytecodePortFailure, BytecodeSchedulerError,
    ChildFinishError, ChildHeapCarrier, ChildHeapOwnerRegistration, OwnerCreationError,
    RequestResourceHandle, RequestResourceTable,
};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, ResumeOutcome, VmBudget, VmCompletion, VmFiber, VmLifecycleSite,
    VmLimits, VmOwnedException, VmResumeToken,
};

use crate::{memory_ledger::MemoryLedgerError, vm_heap::RequestVmHeap, RequestMemoryLedger};

pub use actor::{ActorChildError, BytecodeActorChildComposition, BytecodeActorExecutor};
pub(crate) use callback::execute_callback_child;
pub use callback::{
    BytecodeCallbackChildComposition, BytecodeCallbackChildError, BytecodeCallbackProjector,
    BytecodeCallbackResolver, CallbackExecution,
};
pub use db::{
    BytecodeDbChildComposition, BytecodeDbChildError, DbObjectTargetId, DbTransactionSession,
};
pub(crate) use db::{DbPendingCarrier, DbPendingRoots};
pub(crate) use db_intrinsic::{
    db_argument_runtime_value, db_key_from_runtime, linked_db_target, materialize_db_result_to_vm,
    require_db_operation,
};
pub(crate) use interface::execute_interface_child;
pub(crate) use service::execute_service_child;
pub(crate) use task::{
    actor_method_task_target_control_from_state, encode_durable_task_payload, is_task_request,
    task_arguments, task_submit_message_from_composition, task_target_by_dispatch_index,
};
pub use task::{
    BytecodeTaskChildComposition, BytecodeTaskSubmitError, BytecodeTaskSubmitter,
    FailClosedTaskSubmitter,
};

pub use child_stream::{
    child_stream_next, provider_stream_item, ChildStreamCore, ChildStreamFinish, ChildStreamState,
    ChildStreamSupervisor,
};

/// Routing decision for one VM child target. This is the single X6-owned
/// registration point for the flat child mux; capability lanes either register
/// an executor here or remain explicitly disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BytecodeChildLane {
    Service,
    Interface,
    Db,
    Actor,
    Task,
    Disabled,
}

impl BytecodeChildLane {
    pub(crate) fn for_target(target: ChildTarget) -> Self {
        match target {
            ChildTarget::Service(_) => Self::Service,
            ChildTarget::Interface { .. } => Self::Interface,
            ChildTarget::Db(_) => Self::Db,
            ChildTarget::Actor(_) => Self::Actor,
            ChildTarget::Task(_) => Self::Task,
            ChildTarget::Callback(_) | ChildTarget::StreamNext => Self::Disabled,
        }
    }
}

/// Central Actor child routing seam.
///
/// The A6 leaf supplies exact-build/arena composition facts, but the concrete
/// K6/A6 executor is not installed yet. Registration therefore routes the lane
/// explicitly and keeps every reachable Actor child fail-closed instead of
/// falling through to `UnsupportedChild` with no owner diagnostic.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_actor_child(
    invocation: ChildInvocation,
    heap: &mut dyn VmHeap,
    budget: &mut dyn VmBudget,
    actor_composition: &BytecodeActorChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    resources: RequestResourceTable,
    memory_ledger: Arc<RequestMemoryLedger>,
    observer: BytecodeExecutionObserver,
    limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Actor(index) = invocation.target() else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };
    let _ = index;
    if let Some(executor) = actor_composition.executor.as_ref() {
        let build_id = invocation.resume().image().owner().build_id().as_str();
        if let Err(error) = actor_composition.require_exact_build(build_id) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(error.to_string()),
                invocation,
            ));
        }
        if let Err(error) = actor_composition.require_arena_lease() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(error.to_string()),
                invocation,
            ));
        }
        return executor.execute_method(
            invocation,
            heap,
            budget,
            child_heap_factory,
            resources,
            memory_ledger,
            observer,
            limits,
        );
    }
    let reason = if !actor_composition.is_available() {
        "Actor child requires exact build and arena lease facts before execution".to_string()
    } else {
        "Actor child executor seam is not installed by K6/A6".to_string()
    };
    Err(BytecodePortFailure::input(
        BytecodeSchedulerError::Port(reason),
        invocation,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeServiceChildError {
    #[error("provider service {service_id}@{contract_version} is missing or drifted")]
    ProviderMissing {
        service_id: String,
        contract_version: String,
    },
    #[error("provider protocol identity mismatch: expected {expected}, got {actual}")]
    ProtocolMismatch {
        expected: ServiceProtocolIdentity,
        actual: ServiceProtocolIdentity,
    },
    #[error("provider deployment no longer matches the caller-required deployment")]
    DeploymentDrift,
    #[error("provider operation {operation} is missing")]
    OperationMissing { operation: ContractOperationId },
    #[error("provider resolution failed: {message}")]
    Load { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeChildError {
    #[error("child heap owner creation failed: {0}")]
    Owner(#[from] OwnerCreationError),
    #[error("request memory ledger rejected child heap: {0}")]
    Memory(#[from] MemoryLedgerError),
    #[error("request ledger domain does not fit the concrete heap domain")]
    DomainOverflow,
    #[error("child heap construction failed: {message}")]
    Construction { message: String },
}

/// Cross-image service-throw materialization authority.
///
/// The concrete implementation materializes the provider exception payload
/// through the linked service error plan and mints a caller-owned
/// [`VmOwnedException`] bound to the exact service call resume site.
pub trait ServiceChildThrowMaterializer: Send + Sync + 'static {
    fn materialize_throw(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        parent_image: &DeploymentExecutionImage,
        boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>>;
}

/// Fail-closed service throw materializer retained for unconfigured
/// compositions and negative tests.
#[derive(Default)]
pub struct FailClosedServiceChildThrowMaterializer;

impl ServiceChildThrowMaterializer for FailClosedServiceChildThrowMaterializer {
    fn materialize_throw(
        &self,
        _resume: &VmResumeToken,
        child_result: VmCompletion,
        _child_heap: &mut ChildHeapCarrier,
        _parent_heap: &mut dyn VmHeap,
        _parent_image: &DeploymentExecutionImage,
        _boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        Err(ChildFinishError::result_retained(
            BytecodeSchedulerError::Port(
                "service child ordinary throw requires K6 cross-image VmOwnedException API"
                    .to_string(),
            ),
            child_result,
        ))
    }
}

/// Production cross-image service throw materializer.
#[derive(Default)]
pub struct CrossImageServiceChildThrowMaterializer;

impl ServiceChildThrowMaterializer for CrossImageServiceChildThrowMaterializer {
    fn materialize_throw(
        &self,
        resume: &VmResumeToken,
        child_result: VmCompletion,
        child_heap: &mut ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        parent_image: &DeploymentExecutionImage,
        boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>> {
        let diagnostic = child_result
            .thrown_diagnostic()
            .expect("throw branch is selected only when a thrown diagnostic exists")
            .clone();
        let (outcome, mut residual) = match child_result.into_resume() {
            Ok(parts) => parts,
            Err(_) => {
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "service throw completion lacks the exact owned exception".to_string(),
                )));
            }
        };
        let ResumeOutcome::Throw(mut child_exception) = outcome else {
            let _ = residual.release_all(child_heap.heap_mut());
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "service thrown completion did not carry an owned exception".to_string(),
            )));
        };
        let Some(source_payload) = child_exception.vm_local_payload() else {
            let _ = child_exception.release_all(child_heap.heap_mut());
            let _ = residual.release_all(child_heap.heap_mut());
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                "service thrown completion has no VM-local payload".to_string(),
            )));
        };
        let fallback = boundary_plan.error().fallback();
        let caller_type = fallback.caller_type();
        let caller_payload = match materialize_linked_value(
            child_heap.heap_mut(),
            &source_payload,
            parent_heap,
            parent_image,
            caller_type,
            fallback,
        ) {
            Ok(payload) => payload,
            Err(error) => {
                let _ = child_exception.release_all(child_heap.heap_mut());
                let _ = residual.release_all(child_heap.heap_mut());
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    format!("service throw materialization failed: {error}"),
                )));
            }
        };
        if let Err(error) = child_exception.release_all(child_heap.heap_mut()) {
            let _ = parent_heap.release_snapshot(&caller_payload);
            let _ = residual.release_all(child_heap.heap_mut());
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
        }
        let plan = match parent_image.type_plan(caller_type).cloned() {
            Some(plan) => plan,
            None => {
                let _ = parent_heap.release_snapshot(&caller_payload);
                let _ = residual.release_all(child_heap.heap_mut());
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    "service throw caller payload has no linked transfer plan".to_string(),
                )));
            }
        };
        let site = VmLifecycleSite {
            function: resume.function(),
            instruction: resume.instruction(),
            opcode: Opcode::CallService,
        };
        let caller_exception = match VmOwnedException::try_from_caller_resume(
            Arc::clone(resume.image()),
            resume,
            parent_heap,
            Some(caller_payload),
            &diagnostic,
            plan,
            site,
        ) {
            Ok(exception) => exception,
            Err(rejected) => {
                let (error, payload) = rejected.into_parts();
                if let Some(payload) = payload {
                    let _ = parent_heap.release_snapshot(&payload);
                }
                let _ = residual.release_all(child_heap.heap_mut());
                return Err(ChildFinishError::failure(BytecodeSchedulerError::Port(
                    error.to_string(),
                )));
            }
        };
        if let Err(error) = residual.release_all(child_heap.heap_mut()) {
            return Err(ChildFinishError::failure(BytecodeSchedulerError::Vm(error)));
        }
        Ok(ResumeOutcome::Throw(caller_exception))
    }
}

/// Host/provider-owned service child resolver.
pub trait BytecodeServiceResolver: Send + Sync + 'static {
    fn resolve_service(
        &self,
        slot: &ServiceDependencySlot,
        operation: &ContractOperationId,
        expected_protocol: &ServiceProtocolIdentity,
    ) -> Result<Arc<DeploymentExecutionImage>, BytecodeServiceChildError>;
}

/// Request-owned child heap factory.
pub trait BytecodeChildHeapFactory: Send + Sync + 'static {
    fn create_child_heap(
        &self,
        owner: &DeploymentOwnerIdentity,
        limits: RequestHeapLimits,
        resources: RequestResourceTable,
        ledger: Arc<RequestMemoryLedger>,
    ) -> Result<ChildHeapCarrier, BytecodeChildError>;
}

#[cfg(test)]
pub(crate) struct FailClosedChildHeapFactory;

#[cfg(test)]
impl BytecodeChildHeapFactory for FailClosedChildHeapFactory {
    fn create_child_heap(
        &self,
        _owner: &DeploymentOwnerIdentity,
        _limits: RequestHeapLimits,
        _resources: RequestResourceTable,
        _ledger: Arc<RequestMemoryLedger>,
    ) -> Result<ChildHeapCarrier, BytecodeChildError> {
        Err(BytecodeChildError::Construction {
            message: "child heap factory is not configured".to_string(),
        })
    }
}

#[derive(Clone)]
pub struct BytecodeRequestChildComposition {
    pub memory_ledger: Arc<RequestMemoryLedger>,
    pub service_resolver: Arc<dyn BytecodeServiceResolver>,
    pub child_heap_factory: Option<Arc<dyn BytecodeChildHeapFactory>>,
    pub heap_limits: RequestHeapLimits,
    /// Cross-image throw materialization seam.
    pub throw_materializer: Arc<dyn ServiceChildThrowMaterializer>,
    /// Set only after a service child has successfully materialized its result
    /// back into the caller. The host uses this to classify a successful unary
    /// service response as start/chunk/end instead of a bare unary end.
    pub unary_response_start: Arc<AtomicBool>,
    /// C6 host projection hooks for same-Runtime callback capabilities. The
    /// concrete host type is injected by the host composition; request code
    /// only retains the boundary trait.
    pub callback_hooks: Option<Arc<dyn ServiceLinkableCapabilityHooks>>,
    /// C6 child resolver. It stays fail-closed until the host can provide an
    /// exact same-Runtime provider entry from the F6 callback table.
    pub callback_child: BytecodeCallbackChildComposition,
    /// C6 service-boundary VM projector. The concrete host type registers the
    /// exact caller image/function facts with the callback table.
    pub callback_projector: Option<Arc<dyn BytecodeCallbackProjector>>,
    /// A6 child composition. It stays fail-closed until exact build and arena
    /// lease facts are joined.
    pub actor_child: BytecodeActorChildComposition,
    /// D6R capability/recoverable contexts registered by X6. The exact target
    /// stays fail-closed until F6 emits `DbObjectTargetId` and K6 owns the
    /// transaction token.
    pub db_child: BytecodeDbChildComposition,
    /// Fresh durable task submission seam. Task children are not VM child
    /// heaps: the parent dispatches a fresh request through the same task
    /// control-plane writer and remains independent of the task attempt.
    pub task_child: BytecodeTaskChildComposition,
    /// Child stream state registry shared by the request ingress and boundary
    /// child executors. `StreamNext` fails closed when the exact handle is
    /// absent from this registry.
    pub child_streams: Arc<Mutex<HashMap<RequestResourceHandle, ChildStreamState>>>,
}

impl Default for BytecodeRequestChildComposition {
    fn default() -> Self {
        Self {
            memory_ledger: Arc::new(RequestMemoryLedger::new(usize::MAX)),
            service_resolver: Arc::new(FailClosedServiceResolver),
            child_heap_factory: None,
            heap_limits: RequestHeapLimits::default(),
            throw_materializer: Arc::new(CrossImageServiceChildThrowMaterializer),
            unary_response_start: Arc::new(AtomicBool::new(false)),
            callback_hooks: None,
            callback_child: BytecodeCallbackChildComposition::default(),
            callback_projector: None,
            actor_child: BytecodeActorChildComposition::default(),
            db_child: BytecodeDbChildComposition::default(),
            task_child: BytecodeTaskChildComposition::default(),
            child_streams: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl BytecodeRequestChildComposition {
    pub fn unary_response_started(&self) -> bool {
        self.unary_response_start.load(Ordering::Acquire)
    }
}

#[derive(Default)]
struct FailClosedServiceResolver;

impl BytecodeServiceResolver for FailClosedServiceResolver {
    fn resolve_service(
        &self,
        slot: &ServiceDependencySlot,
        _operation: &ContractOperationId,
        _expected_protocol: &ServiceProtocolIdentity,
    ) -> Result<Arc<DeploymentExecutionImage>, BytecodeServiceChildError> {
        Err(BytecodeServiceChildError::ProviderMissing {
            service_id: slot.contract().service_id.clone(),
            contract_version: slot.contract().contract_version.clone(),
        })
    }
}

/// Concrete child heap factory bound to the request owner inventory.
#[derive(Clone)]
pub struct RequestChildHeapFactory {
    child_heap_registration: ChildHeapOwnerRegistration,
    boundary_registration: BoundaryOwnerRegistration,
}

impl RequestChildHeapFactory {
    pub fn new(
        child_heap_registration: ChildHeapOwnerRegistration,
        boundary_registration: BoundaryOwnerRegistration,
    ) -> Self {
        Self {
            child_heap_registration,
            boundary_registration,
        }
    }
}

impl BytecodeChildHeapFactory for RequestChildHeapFactory {
    fn create_child_heap(
        &self,
        _owner: &DeploymentOwnerIdentity,
        limits: RequestHeapLimits,
        resources: RequestResourceTable,
        ledger: Arc<RequestMemoryLedger>,
    ) -> Result<ChildHeapCarrier, BytecodeChildError> {
        let (domain, epoch) = ledger.mint_heap_identity()?;
        let memory_lease = ledger.zero_lease()?;
        let heap = RequestVmHeap::with_ledger_and_resources(
            Arc::clone(&ledger),
            domain.get(),
            epoch.get(),
            limits,
            Some(resources),
            memory_lease,
        );
        let owner_lease = self.child_heap_registration.mint_lease()?;
        let mut carrier = ChildHeapCarrier::new(
            Box::new(heap),
            domain,
            epoch,
            ledger.zero_lease()?,
            owner_lease,
        );
        carrier.attach_boundary_registration(self.boundary_registration.clone());
        Ok(carrier)
    }
}

/// Checks a service child target index against the caller image table.
pub(crate) fn service_operation_by_index(
    image: &DeploymentExecutionImage,
    index: ServiceOperationIndex,
) -> Option<&skiff_runtime_linked_bytecode::LinkedServiceOperationTarget> {
    let position = usize::try_from(index.get()).ok()?;
    image
        .service_operations()
        .get(position)
        .filter(|target| target.index() == index)
}

#[cfg(test)]
mod tests {
    use skiff_runtime_linked_bytecode::{
        ActorMethodIndex, InterfaceTableIndex, IntrinsicIndex, SyntheticCallbackIndex,
    };
    use skiff_runtime_vm::{ChildTarget, TaskDispatchIndex};

    use super::db::db_child_required_fact;
    use super::*;

    #[test]
    fn child_lane_registers_service_interface_actor_and_disables_remaining_targets() {
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Service(ServiceOperationIndex::new(0))),
            BytecodeChildLane::Service
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Interface {
                table: InterfaceTableIndex::new(0),
                method_ordinal: 0,
            }),
            BytecodeChildLane::Interface
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Db(IntrinsicIndex::new(0))),
            BytecodeChildLane::Db
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Actor(ActorMethodIndex::new(0))),
            BytecodeChildLane::Actor
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Task(
                TaskDispatchIndex::try_new(1).expect("one is valid")
            )),
            BytecodeChildLane::Task
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::Callback(SyntheticCallbackIndex::new(0))),
            BytecodeChildLane::Disabled
        );
        assert_eq!(
            BytecodeChildLane::for_target(ChildTarget::StreamNext),
            BytecodeChildLane::Disabled
        );
    }

    #[test]
    fn db_child_registration_defaults_fail_closed() {
        let composition = BytecodeRequestChildComposition::default();
        assert!(!composition.db_child.is_available());
        assert!(
            db_child_required_fact().contains("DbObjectTargetId"),
            "DB registration must require the exact target identity"
        );
    }

    #[test]
    fn callback_and_actor_compositions_default_fail_closed() {
        let composition = BytecodeRequestChildComposition::default();
        assert!(composition.callback_hooks.is_none());
        assert!(!composition.callback_child.is_available());
        assert!(!composition.actor_child.is_available());
    }

    #[test]
    fn unary_response_start_signal_precedes_end_framing() {
        let composition = BytecodeRequestChildComposition::default();
        assert!(
            !composition.unary_response_started(),
            "a request with no service child result must keep ordinary unary end framing"
        );
        composition
            .unary_response_start
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(composition.unary_response_started());
    }
}
