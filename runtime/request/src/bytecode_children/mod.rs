//! X6 child mux for flat-scheduler bytecode children.
//!
//! Service is the first accepted child lane. Every other target remains
//! fail-closed until its capability lane lands.

mod service;

use std::sync::Arc;

use skiff_artifact_model::{ContractOperationId, ServiceProtocolIdentity};
use skiff_runtime_deployment_image::{DeploymentOwnerIdentity, ServiceDependencySlot};
use skiff_runtime_linked_bytecode::ServiceOperationIndex;
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_scheduler::{
    ChildHeapCarrier, ChildHeapOwnerRegistration, OwnerCreationError, RequestResourceTable,
};

use crate::{memory_ledger::MemoryLedgerError, vm_heap::RequestVmHeap, RequestMemoryLedger};

pub(crate) use service::execute_service_child;

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

#[derive(Clone)]
pub struct BytecodeRequestChildComposition {
    pub memory_ledger: Arc<RequestMemoryLedger>,
    pub service_resolver: Arc<dyn BytecodeServiceResolver>,
    pub child_heap_factory: Option<Arc<dyn BytecodeChildHeapFactory>>,
    pub heap_limits: RequestHeapLimits,
}

impl Default for BytecodeRequestChildComposition {
    fn default() -> Self {
        Self {
            memory_ledger: Arc::new(RequestMemoryLedger::new(usize::MAX)),
            service_resolver: Arc::new(FailClosedServiceResolver),
            child_heap_factory: None,
            heap_limits: RequestHeapLimits::default(),
        }
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
