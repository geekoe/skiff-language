//! X6 child mux for flat-scheduler bytecode children.
//!
//! Service, local/remote interface, callback registration and the K6 DB
//! intrinsic child are the X6 child mux lanes. Actor remains fail-closed until
//! the A6 executor/arena seam supplies an executable callback, and task is
//! wired through the fresh request seam rather than a VM child target.

mod actor;
mod callback;
mod db;
mod db_intrinsic;
mod interface;
mod service;
mod task;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use skiff_artifact_model::{ContractOperationId, ServiceProtocolIdentity};
use skiff_runtime_boundary::service_linkable::ServiceLinkableCapabilityHooks;
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, ServiceDependencySlot};
use skiff_runtime_linked_bytecode::ServiceOperationIndex;
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    bytecode_execution_observation::BytecodeExecutionObserver, request_heap::RequestHeapLimits,
    vm_heap::VmHeap,
};
use skiff_runtime_scheduler::{
    BytecodeChildHandoff, BytecodePortFailure, BytecodeSchedulerError, ChildFinishError,
    ChildHeapCarrier, ChildHeapOwnerRegistration, OwnerCreationError, RequestResourceTable,
};
use skiff_runtime_vm::{
    ChildInvocation, ChildTarget, ResumeOutcome, VmBudget, VmCompletion, VmFiber, VmLimits,
    VmResumeToken,
};

use crate::{memory_ledger::MemoryLedgerError, vm_heap::RequestVmHeap, RequestMemoryLedger};

pub use actor::{ActorChildError, BytecodeActorChildComposition};
pub(crate) use callback::execute_callback_child;
pub use callback::{
    BytecodeCallbackChildComposition, BytecodeCallbackChildError, BytecodeCallbackProjector,
    BytecodeCallbackResolver, CallbackExecution,
};
pub use db::{
    BytecodeDbChildComposition, BytecodeDbChildError, DbObjectTargetId, DbTransactionSession,
};
pub(crate) use db_intrinsic::{
    linked_db_target, materialize_db_result_to_vm, require_db_operation,
};
pub(crate) use interface::execute_interface_child;
pub(crate) use service::execute_service_child;
pub(crate) use task::{execute_task_child, is_task_request, task_arguments};
pub use task::{
    BytecodeTaskChildComposition, BytecodeTaskSubmitError, BytecodeTaskSubmitter,
    FailClosedTaskSubmitter,
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
    _heap: &mut dyn VmHeap,
    _budget: &mut dyn VmBudget,
    actor_composition: &BytecodeActorChildComposition,
    _child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    _resources: RequestResourceTable,
    _observer: BytecodeExecutionObserver,
    _limits: VmLimits,
) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>> {
    let ChildTarget::Actor(index) = invocation.target() else {
        return Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ));
    };
    let _ = index;
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
/// K6's cross-image `VmOwnedException` mint is not available to X6 yet.
/// The production composition therefore keeps a fail-closed default that
/// preserves the child completion and lets the scheduler retain the exact
/// exception owner. A later K6-backed implementation can replace this trait
/// without changing the request authority path.
pub trait ServiceChildThrowMaterializer: Send + Sync + 'static {
    fn materialize_throw(
        &self,
        child_result: VmCompletion,
        child_heap: &mut ChildHeapCarrier,
        parent_heap: &mut dyn VmHeap,
        parent_image: &DeploymentExecutionImage,
        boundary_plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryPlan,
    ) -> Result<ResumeOutcome, ChildFinishError<VmFiber>>;
}

/// Fail-closed service throw materializer used until K6 provides the
/// cross-image exception mint.
#[derive(Default)]
pub struct FailClosedServiceChildThrowMaterializer;

impl ServiceChildThrowMaterializer for FailClosedServiceChildThrowMaterializer {
    fn materialize_throw(
        &self,
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
    /// Cross-image throw materialization seam. The default is fail-closed and
    /// preserves the child completion until K6 supplies a real mint.
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
}

impl Default for BytecodeRequestChildComposition {
    fn default() -> Self {
        Self {
            memory_ledger: Arc::new(RequestMemoryLedger::new(usize::MAX)),
            service_resolver: Arc::new(FailClosedServiceResolver),
            child_heap_factory: None,
            heap_limits: RequestHeapLimits::default(),
            throw_materializer: Arc::new(FailClosedServiceChildThrowMaterializer),
            unary_response_start: Arc::new(AtomicBool::new(false)),
            callback_hooks: None,
            callback_child: BytecodeCallbackChildComposition::default(),
            callback_projector: None,
            actor_child: BytecodeActorChildComposition::default(),
            db_child: BytecodeDbChildComposition::default(),
            task_child: BytecodeTaskChildComposition::default(),
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
}

impl RequestChildHeapFactory {
    pub fn new(child_heap_registration: ChildHeapOwnerRegistration) -> Self {
        Self {
            child_heap_registration,
        }
    }
}

impl BytecodeChildHeapFactory for RequestChildHeapFactory {
    fn create_child_heap(
        &self,
        _owner: &DeploymentOwnerIdentity,
        limits: RequestHeapLimits,
        _resources: RequestResourceTable,
        ledger: Arc<RequestMemoryLedger>,
    ) -> Result<ChildHeapCarrier, BytecodeChildError> {
        let (domain, epoch, memory_lease) = ledger.mint_child_heap(limits.max_estimated_bytes)?;
        let domain_u8 =
            u8::try_from(domain.get()).map_err(|_| BytecodeChildError::DomainOverflow)?;
        let heap = RequestVmHeap::with_domain(domain_u8, epoch.get(), limits);
        let owner_lease = self.child_heap_registration.mint_lease()?;
        Ok(ChildHeapCarrier::new(
            Box::new(heap),
            domain,
            epoch,
            memory_lease,
            owner_lease,
        ))
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
