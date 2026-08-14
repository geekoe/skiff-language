use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::{
    ContractOperationId, DeploymentIngressBinding, GatewayAdapterPlan, GatewayEntryIdentity,
    GatewayEntryKey, HostEffectExecutorIdentity, IngressSelector, ServiceProtocolIdentity,
    ServiceRequirementKey,
};
use skiff_runtime_bytecode_verifier::{
    verify_executable_facts, ExecutableFacts, VerificationError, VerificationLimits,
    VerifiedCallableEffects, VerifiedConstantHeap, VerifiedFunctionEffects, VerifiedResumeSites,
    VerifiedStatementSchedule,
};
use skiff_runtime_deployment_image::{
    DeploymentCacheValue, DeploymentOwnerIdentity, ServiceDependencySlot,
    ServiceDependencySlotError,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, HostEffectAdapterIndex, LinkedActorMethodTarget, LinkedBytecodeCandidate,
    LinkedCallableSignature, LinkedCallbackCaptureLayout, LinkedConstantEntry, LinkedConstantRoot,
    LinkedExactLocalTarget, LinkedFrozenConstantNode, LinkedInterfaceTable, LinkedIntrinsicTarget,
    LinkedPackageBytecodeProvenance, LinkedServiceOperationTarget, LinkedShapeEntry,
    LinkedSyntheticCallbackTarget, LinkedTypeEntry, LinkedWritablePathEntry,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{entry::link_deployment, BytecodeLinkError, LinkLimits};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentExecutionLimits {
    link: LinkLimits,
    verification: VerificationLimits,
}

impl DeploymentExecutionLimits {
    pub const fn new(link: LinkLimits, verification: VerificationLimits) -> Self {
        Self { link, verification }
    }

    pub const fn link(&self) -> &LinkLimits {
        &self.link
    }

    pub const fn verification(&self) -> &VerificationLimits {
        &self.verification
    }
}

/// The sole immutable execution authority for one exact deployment build.
#[derive(Debug)]
pub struct DeploymentExecutionImage {
    linked: LinkedBytecodeCandidate,
    owner: DeploymentOwnerIdentity,
    service_protocol_identity: ServiceProtocolIdentity,
    ingress_bindings: Box<[DeploymentIngressBinding]>,
    dependency_slots: BTreeMap<ServiceRequirementKey, ServiceDependencySlot>,
    operation_entries: BTreeMap<ContractOperationId, CallableEntryFacts>,
    http_gateway_entries: BTreeMap<GatewayEntryKey, HttpGatewayEntryFacts>,
    constant_heap: VerifiedConstantHeap,
    statement_schedule: VerifiedStatementSchedule,
    callable_effects: VerifiedCallableEffects,
    resume_sites: VerifiedResumeSites,
}

#[derive(Debug)]
struct CallableEntryFacts {
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

#[derive(Debug)]
struct HttpGatewayEntryFacts {
    identity: GatewayEntryIdentity,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
    adapter_plan: GatewayAdapterPlan,
}

/// Opaque, verifier-backed execution authority for one indexed host target.
///
/// Target text, binding strings and required context deliberately do not cross
/// the execution-image boundary: runtime consumers dispatch exhaustively on
/// the registry-owned identity.
#[derive(Clone, Copy)]
pub struct DeploymentHostEffectTarget<'image> {
    target: &'image skiff_runtime_linked_bytecode::LinkedHostEffectAdapterTarget,
}

impl DeploymentHostEffectTarget<'_> {
    pub const fn executor_identity(&self) -> HostEffectExecutorIdentity {
        self.target.executor_identity()
    }

    pub const fn signature(&self) -> &skiff_runtime_linked_bytecode::LinkedNativeCallableSignature {
        self.target.signature()
    }
}

impl DeploymentExecutionImage {
    pub const fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    pub fn functions(&self) -> &[skiff_runtime_linked_bytecode::LinkedFunction] {
        self.linked.functions()
    }

    pub fn packages(&self) -> &[LinkedPackageBytecodeProvenance] {
        self.linked.packages()
    }

    pub fn exact_local_targets(&self) -> &[LinkedExactLocalTarget] {
        self.linked.exact_local_targets()
    }

    pub fn constants(&self) -> &[LinkedConstantEntry] {
        self.linked.constants()
    }

    pub fn constant_roots(&self) -> &[LinkedConstantRoot] {
        self.linked.constant_roots()
    }

    pub fn dependency_slots(&self) -> impl ExactSizeIterator<Item = &ServiceDependencySlot> {
        self.dependency_slots.values()
    }

    pub fn dependency_slot(&self, key: &ServiceRequirementKey) -> Option<&ServiceDependencySlot> {
        self.dependency_slots.get(key)
    }

    pub fn service_protocol_identity(&self) -> &ServiceProtocolIdentity {
        &self.service_protocol_identity
    }

    pub fn ingress_bindings(&self) -> &[DeploymentIngressBinding] {
        &self.ingress_bindings
    }

    pub fn gateway_adapter_plan(&self, key: &GatewayEntryKey) -> Option<&GatewayAdapterPlan> {
        self.http_gateway_entries
            .get(key)
            .map(|entry| &entry.adapter_plan)
    }

    pub const fn constant_heap(&self) -> &VerifiedConstantHeap {
        &self.constant_heap
    }

    pub const fn statement_schedule(&self) -> &VerifiedStatementSchedule {
        &self.statement_schedule
    }

    pub const fn resume_sites(&self) -> &VerifiedResumeSites {
        &self.resume_sites
    }

    pub fn function_effects(&self, function: FunctionIndex) -> Option<&VerifiedFunctionEffects> {
        self.callable_effects.function(function)
    }

    pub fn types(&self) -> &[LinkedTypeEntry] {
        self.linked.types()
    }

    pub fn shapes(&self) -> &[LinkedShapeEntry] {
        self.linked.shapes()
    }

    pub fn writable_paths(&self) -> &[LinkedWritablePathEntry] {
        self.linked.writable_paths()
    }

    pub fn intrinsics(&self) -> &[LinkedIntrinsicTarget] {
        self.linked.intrinsics()
    }

    pub fn interface_tables(&self) -> &[LinkedInterfaceTable] {
        self.linked.interface_tables()
    }

    pub fn service_operations(&self) -> &[LinkedServiceOperationTarget] {
        self.linked.service_operations()
    }

    pub fn actor_methods(&self) -> &[LinkedActorMethodTarget] {
        self.linked.actor_methods()
    }

    pub fn host_effect_target(
        &self,
        index: HostEffectAdapterIndex,
    ) -> Option<DeploymentHostEffectTarget<'_>> {
        self.linked
            .host_effect_adapters()
            .get(index.get() as usize)
            .filter(|target| target.index() == index)
            .map(|target| DeploymentHostEffectTarget { target })
    }

    pub fn frozen_constant_nodes(&self) -> &[LinkedFrozenConstantNode] {
        self.linked.frozen_constant_nodes()
    }

    pub fn synthetic_callbacks(&self) -> &[LinkedSyntheticCallbackTarget] {
        self.linked.synthetic_callbacks()
    }

    pub fn callback_capture_layouts(&self) -> &[LinkedCallbackCaptureLayout] {
        self.linked.callback_capture_layouts()
    }

    pub fn operation_entry(
        self: &Arc<Self>,
        operation: &ContractOperationId,
    ) -> Result<DeploymentExecutionEntry, CodeEntryLookupError> {
        let entry = self.operation_entries.get(operation).ok_or_else(|| {
            CodeEntryLookupError::OperationNotFound {
                contract_operation_id: operation.clone(),
            }
        })?;
        Ok(DeploymentExecutionEntry {
            image: Arc::clone(self),
            _kind: DeploymentExecutionEntryKind::Operation {
                contract_operation_id: operation.clone(),
            },
            function: entry.function,
            signature: entry.signature.clone(),
        })
    }

    pub fn http_gateway_entry(
        self: &Arc<Self>,
        ingress: &IngressSelector,
        identity: &GatewayEntryIdentity,
    ) -> Result<DeploymentExecutionEntry, CodeEntryLookupError> {
        let mut bindings = self
            .ingress_bindings
            .iter()
            .filter(|binding| &binding.selector == ingress);
        let binding = bindings
            .next()
            .ok_or_else(|| CodeEntryLookupError::HttpIngressNotFound {
                ingress: ingress.clone(),
            })?;
        if bindings.next().is_some() {
            return Err(CodeEntryLookupError::DuplicateHttpIngress {
                ingress: ingress.clone(),
            });
        }
        let key = &binding.gateway_entry_key;
        let entry = self.http_gateway_entries.get(key).ok_or_else(|| {
            CodeEntryLookupError::HttpGatewayNotFound {
                gateway_entry_key: key.clone(),
            }
        })?;
        if &entry.identity != identity {
            return Err(CodeEntryLookupError::HttpGatewayIdentityMismatch {
                gateway_entry_key: key.clone(),
                expected: entry.identity.clone(),
                actual: identity.clone(),
            });
        }
        Ok(DeploymentExecutionEntry {
            image: Arc::clone(self),
            _kind: DeploymentExecutionEntryKind::HttpGateway {
                gateway_entry_key: key.clone(),
                gateway_entry_identity: entry.identity.clone(),
            },
            function: entry.function,
            signature: entry.signature.clone(),
        })
    }
}

impl DeploymentCacheValue for DeploymentExecutionImage {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeploymentExecutionEntryKind {
    Operation {
        contract_operation_id: ContractOperationId,
    },
    HttpGateway {
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
    },
}

#[derive(Debug)]
pub struct DeploymentExecutionEntry {
    image: Arc<DeploymentExecutionImage>,
    _kind: DeploymentExecutionEntryKind,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl DeploymentExecutionEntry {
    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeEntryLookupError {
    OperationNotFound {
        contract_operation_id: ContractOperationId,
    },
    HttpGatewayNotFound {
        gateway_entry_key: GatewayEntryKey,
    },
    HttpIngressNotFound {
        ingress: IngressSelector,
    },
    DuplicateHttpIngress {
        ingress: IngressSelector,
    },
    HttpGatewayIdentityMismatch {
        gateway_entry_key: GatewayEntryKey,
        expected: GatewayEntryIdentity,
        actual: GatewayEntryIdentity,
    },
}

impl fmt::Display for CodeEntryLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperationNotFound {
                contract_operation_id,
            } => write!(
                formatter,
                "deployment operation entry {contract_operation_id} does not exist"
            ),
            Self::HttpGatewayNotFound { gateway_entry_key } => write!(
                formatter,
                "deployment HTTP gateway entry {gateway_entry_key} does not exist"
            ),
            Self::HttpIngressNotFound { ingress } => {
                write!(formatter, "deployment HTTP ingress {ingress:?} does not exist")
            }
            Self::DuplicateHttpIngress { ingress } => write!(
                formatter,
                "deployment HTTP ingress {ingress:?} has duplicate canonical bindings"
            ),
            Self::HttpGatewayIdentityMismatch {
                gateway_entry_key,
                expected,
                actual,
            } => write!(
                formatter,
                "deployment HTTP gateway entry {gateway_entry_key} identity mismatch: expected {expected}, got {actual}"
            ),
        }
    }
}

impl std::error::Error for CodeEntryLookupError {}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentExecutionImageError {
    #[error(transparent)]
    Link(#[from] BytecodeLinkError),
    #[error(transparent)]
    Verification(#[from] VerificationError),
    #[error("deployment image dependency slot is invalid: {0}")]
    Dependency(#[from] ServiceDependencySlotError),
    #[error("deployment image has duplicate dependency key {0:?}")]
    DuplicateDependency(ServiceRequirementKey),
}

pub fn link_deployment_execution_image(
    hydrated: HydratedDeploymentBytecode,
    limits: &DeploymentExecutionLimits,
) -> Result<DeploymentExecutionImage, DeploymentExecutionImageError> {
    let linked = link_deployment(&hydrated, limits.link())?;
    let facts: ExecutableFacts =
        verify_executable_facts(&hydrated, &linked, limits.verification())?;
    let owner = DeploymentOwnerIdentity::new(hydrated.reference().clone());
    let service_protocol_identity = hydrated
        .deployment()
        .contract
        .service_protocol_identity
        .clone();
    let ingress_bindings = hydrated.deployment().ingress.clone().into_boxed_slice();
    let dependency_slots = dependency_slots(&hydrated)?;
    let operation_entries = linked
        .operation_entries()
        .iter()
        .map(|entry| {
            (
                entry.contract_operation_id().clone(),
                CallableEntryFacts {
                    function: entry.function(),
                    signature: entry.signature().clone(),
                },
            )
        })
        .collect();
    let http_gateway_entries = linked
        .gateway_entries()
        .iter()
        .filter_map(|entry| {
            entry.handler().map(|handler| {
                (
                    entry.gateway_entry_key().clone(),
                    HttpGatewayEntryFacts {
                        identity: entry.gateway_entry_identity().clone(),
                        function: handler.function(),
                        signature: handler.signature().clone(),
                        adapter_plan: entry.adapter_plan().clone(),
                    },
                )
            })
        })
        .collect();
    let (constant_heap, statement_schedule, callable_effects, resume_sites) = facts.into_parts();
    Ok(DeploymentExecutionImage {
        linked,
        owner,
        service_protocol_identity,
        ingress_bindings,
        dependency_slots,
        operation_entries,
        http_gateway_entries,
        constant_heap,
        statement_schedule,
        callable_effects,
        resume_sites,
    })
}

fn dependency_slots(
    hydrated: &HydratedDeploymentBytecode,
) -> Result<BTreeMap<ServiceRequirementKey, ServiceDependencySlot>, DeploymentExecutionImageError> {
    let mut slots = BTreeMap::new();
    for dependency in hydrated.service_dependencies().values() {
        let slot = ServiceDependencySlot::try_new(
            dependency.key().clone(),
            dependency.contract().clone(),
            dependency.used_operations().iter().cloned(),
        )?;
        let key = slot.key().clone();
        if slots.insert(key.clone(), slot).is_some() {
            return Err(DeploymentExecutionImageError::DuplicateDependency(key));
        }
    }
    Ok(slots)
}
