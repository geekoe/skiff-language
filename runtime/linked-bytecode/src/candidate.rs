mod validation;

use std::fmt;

use skiff_artifact_model::{
    AbiInterfaceId, ActorAbiIdentity, ActorMethodIdentity, ContractOperationId, GatewayEntryKey,
    ServiceRequirementKey,
};

use crate::{
    LinkedActorMethodTarget, LinkedConstantEntry, LinkedExactLocalTarget, LinkedGatewayEntry,
    LinkedHostEffectAdapterTarget, LinkedInterfaceTable, LinkedOperationEntry, LinkedResumeSite,
    LinkedServiceOperationTarget, LinkedShapeEntry, LinkedSyntheticCallbackTarget, LinkedTypeEntry,
    SpecializationKey,
};

/// The linker's sole public raw aggregate builder. Every component row keeps
/// private fields and must be constructed through its local constructor.
#[derive(Debug)]
pub struct LinkedBytecodeCandidateParts {
    pub functions: Vec<crate::LinkedFunction>,
    pub operation_entries: Vec<LinkedOperationEntry>,
    pub gateway_entries: Vec<LinkedGatewayEntry>,
    pub exact_local_targets: Vec<LinkedExactLocalTarget>,
    pub service_operations: Vec<LinkedServiceOperationTarget>,
    pub actor_methods: Vec<LinkedActorMethodTarget>,
    pub interface_tables: Vec<LinkedInterfaceTable>,
    pub synthetic_callbacks: Vec<LinkedSyntheticCallbackTarget>,
    pub host_effect_adapters: Vec<LinkedHostEffectAdapterTarget>,
    pub types: Vec<LinkedTypeEntry>,
    pub shapes: Vec<LinkedShapeEntry>,
    pub constants: Vec<LinkedConstantEntry>,
    pub resume_sites: Vec<LinkedResumeSite>,
}

/// Concrete linker output that has passed only local candidate-shape checks.
/// It is not a verified execution image.
#[derive(Debug)]
pub struct LinkedBytecodeCandidate {
    functions: Box<[crate::LinkedFunction]>,
    operation_entries: Box<[LinkedOperationEntry]>,
    gateway_entries: Box<[LinkedGatewayEntry]>,
    exact_local_targets: Box<[LinkedExactLocalTarget]>,
    service_operations: Box<[LinkedServiceOperationTarget]>,
    actor_methods: Box<[LinkedActorMethodTarget]>,
    interface_tables: Box<[LinkedInterfaceTable]>,
    synthetic_callbacks: Box<[LinkedSyntheticCallbackTarget]>,
    host_effect_adapters: Box<[LinkedHostEffectAdapterTarget]>,
    types: Box<[LinkedTypeEntry]>,
    shapes: Box<[LinkedShapeEntry]>,
    constants: Box<[LinkedConstantEntry]>,
    resume_sites: Box<[LinkedResumeSite]>,
}

impl LinkedBytecodeCandidate {
    pub fn try_from_parts(
        parts: LinkedBytecodeCandidateParts,
    ) -> Result<Self, LinkedBytecodeCandidateError> {
        validation::validate_parts(&parts)?;
        Ok(Self {
            functions: parts.functions.into_boxed_slice(),
            operation_entries: parts.operation_entries.into_boxed_slice(),
            gateway_entries: parts.gateway_entries.into_boxed_slice(),
            exact_local_targets: parts.exact_local_targets.into_boxed_slice(),
            service_operations: parts.service_operations.into_boxed_slice(),
            actor_methods: parts.actor_methods.into_boxed_slice(),
            interface_tables: parts.interface_tables.into_boxed_slice(),
            synthetic_callbacks: parts.synthetic_callbacks.into_boxed_slice(),
            host_effect_adapters: parts.host_effect_adapters.into_boxed_slice(),
            types: parts.types.into_boxed_slice(),
            shapes: parts.shapes.into_boxed_slice(),
            constants: parts.constants.into_boxed_slice(),
            resume_sites: parts.resume_sites.into_boxed_slice(),
        })
    }

    pub fn functions(&self) -> &[crate::LinkedFunction] {
        &self.functions
    }

    pub fn operation_entries(&self) -> &[LinkedOperationEntry] {
        &self.operation_entries
    }

    pub fn gateway_entries(&self) -> &[LinkedGatewayEntry] {
        &self.gateway_entries
    }

    pub fn exact_local_targets(&self) -> &[LinkedExactLocalTarget] {
        &self.exact_local_targets
    }

    pub fn service_operations(&self) -> &[LinkedServiceOperationTarget] {
        &self.service_operations
    }

    pub fn actor_methods(&self) -> &[LinkedActorMethodTarget] {
        &self.actor_methods
    }

    pub fn interface_tables(&self) -> &[LinkedInterfaceTable] {
        &self.interface_tables
    }

    pub fn synthetic_callbacks(&self) -> &[LinkedSyntheticCallbackTarget] {
        &self.synthetic_callbacks
    }

    pub fn host_effect_adapters(&self) -> &[LinkedHostEffectAdapterTarget] {
        &self.host_effect_adapters
    }

    pub fn types(&self) -> &[LinkedTypeEntry] {
        &self.types
    }

    pub fn shapes(&self) -> &[LinkedShapeEntry] {
        &self.shapes
    }

    pub fn constants(&self) -> &[LinkedConstantEntry] {
        &self.constants
    }

    pub fn resume_sites(&self) -> &[LinkedResumeSite] {
        &self.resume_sites
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateTable {
    Functions,
    OperationEntries,
    GatewayEntries,
    ExactLocalTargets,
    ServiceOperations,
    ActorMethods,
    InterfaceTables,
    SyntheticCallbacks,
    HostEffectAdapters,
    Types,
    Shapes,
    Constants,
    ResumeSites,
}

impl CandidateTable {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Functions => "functions",
            Self::OperationEntries => "operation entries",
            Self::GatewayEntries => "gateway entries",
            Self::ExactLocalTargets => "exact local targets",
            Self::ServiceOperations => "service operations",
            Self::ActorMethods => "actor methods",
            Self::InterfaceTables => "interface tables",
            Self::SyntheticCallbacks => "synthetic callbacks",
            Self::HostEffectAdapters => "host effect adapters",
            Self::Types => "types",
            Self::Shapes => "shapes",
            Self::Constants => "constants",
            Self::ResumeSites => "resume sites",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedBytecodeCandidateError {
    TableTooLarge {
        table: CandidateTable,
        len: usize,
    },
    DuplicateIndex {
        table: CandidateTable,
        index: u32,
    },
    NonDenseIndex {
        table: CandidateTable,
        position: usize,
        expected: u32,
        actual: u32,
    },
    DuplicateFunctionKey {
        key: SpecializationKey,
    },
    DuplicateExactLocalTarget {
        key: SpecializationKey,
    },
    DuplicateOperationEntry {
        contract_operation_id: ContractOperationId,
    },
    NonCanonicalOperationEntryOrder {
        previous: ContractOperationId,
        current: ContractOperationId,
    },
    DuplicateGatewayEntry {
        gateway_entry_key: GatewayEntryKey,
    },
    NonCanonicalGatewayEntryOrder {
        previous: GatewayEntryKey,
        current: GatewayEntryKey,
    },
    DuplicateServiceOperation {
        service_requirement_key: ServiceRequirementKey,
        contract_operation_id: ContractOperationId,
    },
    DuplicateActorMethod {
        actor_abi_identity: ActorAbiIdentity,
        method_identity: ActorMethodIdentity,
    },
    DuplicateInterfaceTable {
        interface_identity: AbiInterfaceId,
    },
    RootFunctionOutOfBounds {
        source_table: CandidateTable,
        source_index: u32,
        function_index: u32,
        function_len: usize,
    },
}

impl fmt::Display for LinkedBytecodeCandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TableTooLarge { table, len } => {
                write!(formatter, "{} table has {len} rows, exceeding u32", table.name())
            }
            Self::DuplicateIndex { table, index } => {
                write!(formatter, "{} table repeats index {index}", table.name())
            }
            Self::NonDenseIndex {
                table,
                position,
                expected,
                actual,
            } => write!(
                formatter,
                "{} row {position} has index {actual}; expected dense index {expected}",
                table.name()
            ),
            Self::DuplicateFunctionKey { key } => write!(
                formatter,
                "function specialization {key:?} appears more than once"
            ),
            Self::DuplicateExactLocalTarget { key } => write!(
                formatter,
                "exact local specialization {key:?} appears more than once"
            ),
            Self::DuplicateOperationEntry {
                contract_operation_id,
            } => write!(
                formatter,
                "operation entry {contract_operation_id} appears more than once"
            ),
            Self::NonCanonicalOperationEntryOrder { previous, current } => write!(
                formatter,
                "operation entry {current} must sort after {previous}"
            ),
            Self::DuplicateGatewayEntry { gateway_entry_key } => write!(
                formatter,
                "gateway entry {gateway_entry_key} appears more than once"
            ),
            Self::NonCanonicalGatewayEntryOrder { previous, current } => write!(
                formatter,
                "gateway entry {current} must sort after {previous}"
            ),
            Self::DuplicateServiceOperation {
                service_requirement_key,
                contract_operation_id,
            } => write!(
                formatter,
                "service operation ({service_requirement_key:?}, {contract_operation_id}) appears more than once"
            ),
            Self::DuplicateActorMethod {
                actor_abi_identity,
                method_identity,
            } => write!(
                formatter,
                "actor method ({}, {}) appears more than once",
                actor_abi_identity.as_str(),
                method_identity.as_str()
            ),
            Self::DuplicateInterfaceTable { interface_identity } => write!(
                formatter,
                "interface table {interface_identity:?} appears more than once"
            ),
            Self::RootFunctionOutOfBounds {
                source_table,
                source_index,
                function_index,
                function_len,
            } => write!(
                formatter,
                "{} row {source_index} references function index {function_index}, but the function table has {function_len} rows",
                source_table.name()
            ),
        }
    }
}

impl std::error::Error for LinkedBytecodeCandidateError {}
