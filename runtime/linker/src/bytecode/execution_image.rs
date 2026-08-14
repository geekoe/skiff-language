use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::{
    ContractOperationId, DeploymentIngressBinding, GatewayAdapterPlan, GatewayEntryIdentity,
    GatewayEntryKey, HostEffectExecutorIdentity, IngressSelector, Opcode, ServiceProtocolIdentity,
    ServiceRequirementKey, StatementAttributionClass,
};
use skiff_runtime_deployment_image::{
    DeploymentCacheValue, DeploymentOwnerIdentity, ServiceDependencySlot,
    ServiceDependencySlotError,
};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, FrozenConstantNodeIndex, FunctionIndex, HostEffectAdapterIndex,
    InstructionIndex, LinkedActorMethodTarget, LinkedBytecodeCandidate, LinkedCallableSignature,
    LinkedCallbackCaptureLayout, LinkedConstantEntry, LinkedConstantRoot, LinkedExactLocalTarget,
    LinkedFrozenConstantNode, LinkedInterfaceTable, LinkedIntrinsicTarget,
    LinkedPackageBytecodeProvenance, LinkedServiceOperationTarget, LinkedShapeEntry,
    LinkedSyntheticCallbackTarget, LinkedTypeEntry, LinkedWritablePathEntry, ResumeSiteIndex,
    ShapeIndex, TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;
use skiff_runtime_model::vm_value::CompactTypeTag;

use super::{entry::link_deployment, BytecodeLinkError, LinkLimits};

mod constants;
mod resume;
mod statements;

pub use constants::ExecutionConstantHeap;
pub use resume::{ExecutionResumeKind, ExecutionResumeSite, ExecutionResumeSites};
pub use statements::{ExecutionStatementEvent, ExecutionStatementSchedule};

pub(in crate::bytecode) use constants::build_constant_heap;
pub(in crate::bytecode) use resume::build_resume_sites;
pub(in crate::bytecode) use statements::build_statement_schedule;

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
    constant_heap: ExecutionConstantHeap,
    statement_schedule: ExecutionStatementSchedule,
    resume_sites: ExecutionResumeSites,
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
    parameter_dense_record_shape: Option<ShapeIndex>,
}

/// Opaque, atomic-image-backed execution authority for one indexed host target.
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

    pub const fn constant_heap(&self) -> &ExecutionConstantHeap {
        &self.constant_heap
    }

    pub const fn statement_schedule(&self) -> &ExecutionStatementSchedule {
        &self.statement_schedule
    }

    pub const fn resume_sites(&self) -> &ExecutionResumeSites {
        &self.resume_sites
    }

    pub fn types(&self) -> &[LinkedTypeEntry] {
        self.linked.types()
    }

    /// Returns the compiler-owned plan for one exact linked type row.
    ///
    /// This is a checked direct-index lookup. It deliberately performs no
    /// nominal, registry, or equivalent-type search.
    pub fn type_plan(
        &self,
        index: TypeIndex,
    ) -> Option<&skiff_runtime_linked_bytecode::LinkedValueTransferPlan> {
        let position = usize::try_from(index.get()).ok()?;
        self.linked
            .types()
            .get(position)
            .filter(|entry| entry.index() == index)
            .map(LinkedTypeEntry::plan)
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
            parameter_dense_record_shape: None,
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
            parameter_dense_record_shape: entry.parameter_dense_record_shape,
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
    parameter_dense_record_shape: Option<ShapeIndex>,
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

    /// Exact compiler-emitted dense layout for the gateway's parameter, when
    /// the entry surface admits such materialization.
    pub const fn parameter_dense_record_shape(&self) -> Option<ShapeIndex> {
        self.parameter_dense_record_shape
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

/// Fail-closed structural error while assembling runtime-only image views.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionImageConstructionError {
    #[error("linked type {type_index:?} cannot be represented by a VM value type tag")]
    CompactTypeTagOutOfRange { type_index: TypeIndex },
    #[error("constant {constant:?} has a reference kind unsupported by the execution image")]
    UnsupportedConstantReference { constant: ConstantIndex },
    #[error("constant {constant:?} references missing frozen node {node:?}")]
    ConstantNodeMissing {
        constant: ConstantIndex,
        node: FrozenConstantNodeIndex,
    },
    #[error("constant {constant:?} references unsupported frozen node {node:?}")]
    UnsupportedConstantNode {
        constant: ConstantIndex,
        node: FrozenConstantNodeIndex,
    },
    #[error("constant {constant:?} number cannot be represented by the VM scalar carrier")]
    ConstantNumberNotRepresentable { constant: ConstantIndex },
    #[error("statement schedule for function {function:?} exceeds addressable memory")]
    StatementScheduleOverflow { function: FunctionIndex },
    #[error(
        "statement row for function {function:?} references missing instruction {instruction:?}"
    )]
    StatementInstructionOutOfBounds {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    #[error(
        "statement contract for {function:?}/{instruction:?} ({opcode:?}) requires exactly one {expected_attribution:?} event; found {matching}"
    )]
    StatementContractMismatch {
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
        expected_attribution: StatementAttributionClass,
        matching: usize,
    },
    #[error("resume site {resume_site:?} references missing function {function:?}")]
    ResumeFunctionMissing {
        resume_site: ResumeSiteIndex,
        function: FunctionIndex,
    },
    #[error(
        "resume site {resume_site:?} references missing instruction {function:?}/{instruction:?}"
    )]
    ResumeInstructionMissing {
        resume_site: ResumeSiteIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    #[error(
        "resume site {resume_site:?} points at non-pending opcode {opcode:?} at {function:?}/{instruction:?}"
    )]
    ResumeOpcodeNotPending {
        resume_site: ResumeSiteIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    },
    #[error(
        "resume site {resume_site:?} has {matching_targets} matching targets at {function:?}/{instruction:?}; expected one"
    )]
    ResumeTargetMismatch {
        resume_site: ResumeSiteIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
        matching_targets: usize,
    },
    #[error("StreamNext resume site {resume_site:?} has an invalid mechanical descriptor shape")]
    StreamResumeShape { resume_site: ResumeSiteIndex },
    #[error("pending instruction ordinal overflow in function {function:?}")]
    ResumeInstructionOverflow { function: FunctionIndex },
    #[error(
        "pending instruction {function:?}/{instruction:?} has {actual} resume targets; expected one"
    )]
    PendingInstructionResumeCardinality {
        function: FunctionIndex,
        instruction: InstructionIndex,
        actual: usize,
    },
    #[error(
        "pending instruction {function:?}/{instruction:?} does not match resume site {resume_site:?}"
    )]
    PendingInstructionResumeMismatch {
        function: FunctionIndex,
        instruction: InstructionIndex,
        resume_site: ResumeSiteIndex,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentExecutionImageError {
    #[error(transparent)]
    Link(#[from] BytecodeLinkError),
    #[error(transparent)]
    Construction(#[from] ExecutionImageConstructionError),
    #[error("deployment image dependency slot is invalid: {0}")]
    Dependency(#[from] ServiceDependencySlotError),
    #[error("deployment image has duplicate dependency key {0:?}")]
    DuplicateDependency(ServiceRequirementKey),
}

pub fn link_deployment_execution_image(
    hydrated: HydratedDeploymentBytecode,
    limits: &LinkLimits,
) -> Result<DeploymentExecutionImage, DeploymentExecutionImageError> {
    let linked = link_deployment(&hydrated, limits)?;
    validate_compact_type_tags(&linked)?;
    let constant_heap = build_constant_heap(&linked)?;
    let statement_schedule = build_statement_schedule(&linked)?;
    let resume_sites = build_resume_sites(&linked)?;
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
                        parameter_dense_record_shape: linked
                            .functions()
                            .get(handler.function().get() as usize)
                            .and_then(|function| function.frame().parameters().first())
                            .and_then(|parameter| parameter.dense_record_shape()),
                    },
                )
            })
        })
        .collect();
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
        resume_sites,
    })
}

pub(super) fn compact_type_tag(
    type_index: TypeIndex,
) -> Result<CompactTypeTag, ExecutionImageConstructionError> {
    CompactTypeTag::try_from_type_index(type_index.get())
        .ok_or(ExecutionImageConstructionError::CompactTypeTagOutOfRange { type_index })
}

fn validate_compact_type_tags(
    linked: &LinkedBytecodeCandidate,
) -> Result<(), ExecutionImageConstructionError> {
    for linked_type in linked.types() {
        compact_type_tag(linked_type.index())?;
    }
    Ok(())
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

#[cfg(test)]
mod type_tag_tests {
    use skiff_runtime_linked_bytecode::TypeIndex;

    use super::{compact_type_tag, ExecutionImageConstructionError};

    #[test]
    fn compact_type_tag_boundary_preserves_row_zero_and_rejects_u32_max() {
        let row_zero = compact_type_tag(TypeIndex::new(0)).expect("row zero must be representable");
        assert_eq!(row_zero.type_index(), 0);
        assert_eq!(
            compact_type_tag(TypeIndex::new(u32::MAX)),
            Err(ExecutionImageConstructionError::CompactTypeTagOutOfRange {
                type_index: TypeIndex::new(u32::MAX),
            })
        );
    }
}
