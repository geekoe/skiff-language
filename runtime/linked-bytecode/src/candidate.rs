mod validation;

use std::fmt;

use skiff_artifact_model::{
    BytecodeArtifactRef, ContractOperationId, GatewayEntryKey,
    NativeValueLifecycleRegistryIdentity, PackageBuildId, ServiceRequirementKey,
};

use crate::{
    LinkedActorCreateTarget, LinkedActorMethodTarget, LinkedCallbackCaptureLayout,
    LinkedConstantEntry, LinkedConstantRoot, LinkedExactLocalTarget, LinkedFrozenConstantNode,
    LinkedGatewayEntry, LinkedHostEffectAdapterTarget, LinkedInterfaceTable, LinkedIntrinsicTarget,
    LinkedOperationEntry, LinkedResumeSite, LinkedServiceOperationTarget, LinkedShapeEntry,
    LinkedSyntheticCallbackTarget, LinkedTypeEntry, LinkedWritablePathEntry, SpecializationKey,
};

use crate::BytecodePackageIndex;

/// Exact package bytecode/header provenance retained beside all linked rows.
/// The schema string is intentionally data: current production input is the
/// lifecycle-pinned `skiff-bytecode-v4`, and unknown values fail hydration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPackageBytecodeProvenance {
    index: BytecodePackageIndex,
    package_build_id: PackageBuildId,
    artifact_ref: BytecodeArtifactRef,
    declared_bytecode_identity: Box<str>,
    magic: Box<str>,
    schema_version: Box<str>,
    isa_version: Box<str>,
    opcode_table_fingerprint: Box<str>,
    lifecycle_registry: NativeValueLifecycleRegistryIdentity,
}

impl LinkedPackageBytecodeProvenance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: BytecodePackageIndex,
        package_build_id: PackageBuildId,
        artifact_ref: BytecodeArtifactRef,
        declared_bytecode_identity: impl Into<String>,
        magic: impl Into<String>,
        schema_version: impl Into<String>,
        isa_version: impl Into<String>,
        opcode_table_fingerprint: impl Into<String>,
        lifecycle_registry: NativeValueLifecycleRegistryIdentity,
    ) -> Result<Self, LinkedPackageBytecodeProvenanceError> {
        validate_lifecycle_registry_identity(&lifecycle_registry)?;
        let declared_bytecode_identity = validate_header_text(
            declared_bytecode_identity.into(),
            LinkedBytecodeHeaderField::BytecodeIdentity,
        )?;
        let magic = validate_header_text(magic.into(), LinkedBytecodeHeaderField::Magic)?;
        let schema_version = validate_header_text(
            schema_version.into(),
            LinkedBytecodeHeaderField::SchemaVersion,
        )?;
        let isa_version =
            validate_header_text(isa_version.into(), LinkedBytecodeHeaderField::IsaVersion)?;
        let opcode_table_fingerprint = validate_header_text(
            opcode_table_fingerprint.into(),
            LinkedBytecodeHeaderField::OpcodeTableFingerprint,
        )?;
        if artifact_ref.bytecode_identity.is_empty() {
            return Err(LinkedPackageBytecodeProvenanceError::EmptyArtifactReferenceIdentity);
        }
        if artifact_ref.artifact_path.is_some() {
            return Err(LinkedPackageBytecodeProvenanceError::ArtifactReferencePathNotAllowed);
        }
        if artifact_ref.bytecode_identity != declared_bytecode_identity.as_ref() {
            return Err(
                LinkedPackageBytecodeProvenanceError::ArtifactIdentityMismatch {
                    referenced: artifact_ref.bytecode_identity.clone(),
                    declared: declared_bytecode_identity.into_string(),
                },
            );
        }
        Ok(Self {
            index,
            package_build_id,
            artifact_ref,
            declared_bytecode_identity,
            magic,
            schema_version,
            isa_version,
            opcode_table_fingerprint,
            lifecycle_registry,
        })
    }

    pub const fn index(&self) -> BytecodePackageIndex {
        self.index
    }

    pub const fn package_build_id(&self) -> &PackageBuildId {
        &self.package_build_id
    }

    pub const fn artifact_ref(&self) -> &BytecodeArtifactRef {
        &self.artifact_ref
    }

    pub fn declared_bytecode_identity(&self) -> &str {
        &self.declared_bytecode_identity
    }

    pub fn magic(&self) -> &str {
        &self.magic
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn isa_version(&self) -> &str {
        &self.isa_version
    }

    pub fn opcode_table_fingerprint(&self) -> &str {
        &self.opcode_table_fingerprint
    }

    pub const fn lifecycle_registry(&self) -> &NativeValueLifecycleRegistryIdentity {
        &self.lifecycle_registry
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedLifecycleRegistryField {
    RegistryId,
    Version,
    Fingerprint,
}

impl LinkedLifecycleRegistryField {
    const fn name(self) -> &'static str {
        match self {
            Self::RegistryId => "lifecycle registry id",
            Self::Version => "lifecycle registry version",
            Self::Fingerprint => "lifecycle registry fingerprint",
        }
    }
}

fn validate_lifecycle_registry_identity(
    identity: &NativeValueLifecycleRegistryIdentity,
) -> Result<(), LinkedPackageBytecodeProvenanceError> {
    for (field, value) in [
        (
            LinkedLifecycleRegistryField::RegistryId,
            &identity.registry_id,
        ),
        (LinkedLifecycleRegistryField::Version, &identity.version),
        (
            LinkedLifecycleRegistryField::Fingerprint,
            &identity.fingerprint,
        ),
    ] {
        if value.is_empty() {
            return Err(
                LinkedPackageBytecodeProvenanceError::EmptyLifecycleRegistryField { field },
            );
        }
        if let Some((character_index, _)) = value
            .chars()
            .enumerate()
            .find(|(_, character)| character.is_whitespace() || character.is_control())
        {
            return Err(
                LinkedPackageBytecodeProvenanceError::InvalidLifecycleRegistryField {
                    field,
                    value: value.clone(),
                    character_index,
                },
            );
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedBytecodeHeaderField {
    BytecodeIdentity,
    Magic,
    SchemaVersion,
    IsaVersion,
    OpcodeTableFingerprint,
}

impl LinkedBytecodeHeaderField {
    pub const fn name(self) -> &'static str {
        match self {
            Self::BytecodeIdentity => "bytecode identity",
            Self::Magic => "magic",
            Self::SchemaVersion => "schema version",
            Self::IsaVersion => "ISA version",
            Self::OpcodeTableFingerprint => "opcode table fingerprint",
        }
    }
}

fn validate_header_text(
    value: String,
    field: LinkedBytecodeHeaderField,
) -> Result<Box<str>, LinkedPackageBytecodeProvenanceError> {
    if value.is_empty() {
        return Err(LinkedPackageBytecodeProvenanceError::EmptyHeaderField { field });
    }
    if let Some((character_index, _)) = value
        .chars()
        .enumerate()
        .find(|(_, character)| character.is_whitespace() || character.is_control())
    {
        return Err(LinkedPackageBytecodeProvenanceError::InvalidHeaderField {
            field,
            value,
            character_index,
        });
    }
    Ok(value.into_boxed_str())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedPackageBytecodeProvenanceError {
    EmptyArtifactReferenceIdentity,
    ArtifactReferencePathNotAllowed,
    ArtifactIdentityMismatch {
        referenced: String,
        declared: String,
    },
    EmptyHeaderField {
        field: LinkedBytecodeHeaderField,
    },
    InvalidHeaderField {
        field: LinkedBytecodeHeaderField,
        value: String,
        character_index: usize,
    },
    EmptyLifecycleRegistryField {
        field: LinkedLifecycleRegistryField,
    },
    InvalidLifecycleRegistryField {
        field: LinkedLifecycleRegistryField,
        value: String,
        character_index: usize,
    },
}

impl fmt::Display for LinkedPackageBytecodeProvenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyArtifactReferenceIdentity => {
                formatter.write_str("bytecode artifact reference identity must not be empty")
            }
            Self::ArtifactReferencePathNotAllowed => formatter.write_str(
                "linked bytecode provenance must not retain an artifact locator path",
            ),
            Self::ArtifactIdentityMismatch {
                referenced,
                declared,
            } => {
                write!(
                    formatter,
                    "bytecode artifact reference identity {referenced:?} does not match declared header identity {declared:?}"
                )
            }
            Self::EmptyHeaderField { field } => {
                write!(formatter, "bytecode {} must not be empty", field.name())
            }
            Self::InvalidHeaderField {
                field,
                value,
                character_index,
            } => write!(
                formatter,
                "bytecode {} {value:?} contains whitespace or a control character at character index {character_index}",
                field.name()
            ),
            Self::EmptyLifecycleRegistryField { field } => {
                write!(formatter, "{} must not be empty", field.name())
            }
            Self::InvalidLifecycleRegistryField {
                field,
                value,
                character_index,
            } => write!(
                formatter,
                "{} {value:?} contains whitespace or a control character at character index {character_index}",
                field.name()
            ),
        }
    }
}

impl std::error::Error for LinkedPackageBytecodeProvenanceError {}

/// The linker's sole public raw aggregate builder. Every component row keeps
/// private fields and must be constructed through its local constructor.
#[derive(Debug)]
pub struct LinkedBytecodeCandidateParts {
    pub packages: Vec<LinkedPackageBytecodeProvenance>,
    pub functions: Vec<crate::LinkedFunction>,
    pub operation_entries: Vec<LinkedOperationEntry>,
    pub gateway_entries: Vec<LinkedGatewayEntry>,
    pub exact_local_targets: Vec<LinkedExactLocalTarget>,
    pub service_operations: Vec<LinkedServiceOperationTarget>,
    pub actor_creates: Vec<LinkedActorCreateTarget>,
    pub actor_methods: Vec<LinkedActorMethodTarget>,
    pub interface_tables: Vec<LinkedInterfaceTable>,
    pub synthetic_callbacks: Vec<LinkedSyntheticCallbackTarget>,
    pub callback_capture_layouts: Vec<LinkedCallbackCaptureLayout>,
    pub host_effect_adapters: Vec<LinkedHostEffectAdapterTarget>,
    pub intrinsics: Vec<LinkedIntrinsicTarget>,
    pub types: Vec<LinkedTypeEntry>,
    pub shapes: Vec<LinkedShapeEntry>,
    pub constants: Vec<LinkedConstantEntry>,
    pub constant_roots: Vec<LinkedConstantRoot>,
    pub frozen_constant_nodes: Vec<LinkedFrozenConstantNode>,
    pub resume_sites: Vec<LinkedResumeSite>,
    pub writable_paths: Vec<LinkedWritablePathEntry>,
}

/// Concrete linker output that has passed only local candidate-shape checks.
/// It is not a verified execution image and is never a valid VM input.
#[derive(Debug)]
pub struct LinkedBytecodeCandidate {
    packages: Box<[LinkedPackageBytecodeProvenance]>,
    functions: Box<[crate::LinkedFunction]>,
    operation_entries: Box<[LinkedOperationEntry]>,
    gateway_entries: Box<[LinkedGatewayEntry]>,
    exact_local_targets: Box<[LinkedExactLocalTarget]>,
    service_operations: Box<[LinkedServiceOperationTarget]>,
    actor_creates: Box<[LinkedActorCreateTarget]>,
    actor_methods: Box<[LinkedActorMethodTarget]>,
    interface_tables: Box<[LinkedInterfaceTable]>,
    synthetic_callbacks: Box<[LinkedSyntheticCallbackTarget]>,
    callback_capture_layouts: Box<[LinkedCallbackCaptureLayout]>,
    host_effect_adapters: Box<[LinkedHostEffectAdapterTarget]>,
    intrinsics: Box<[LinkedIntrinsicTarget]>,
    types: Box<[LinkedTypeEntry]>,
    shapes: Box<[LinkedShapeEntry]>,
    constants: Box<[LinkedConstantEntry]>,
    constant_roots: Box<[LinkedConstantRoot]>,
    frozen_constant_nodes: Box<[LinkedFrozenConstantNode]>,
    resume_sites: Box<[LinkedResumeSite]>,
    writable_paths: Box<[LinkedWritablePathEntry]>,
}

impl LinkedBytecodeCandidate {
    pub fn try_from_parts(
        parts: LinkedBytecodeCandidateParts,
    ) -> Result<Self, LinkedBytecodeCandidateError> {
        validation::validate_parts(&parts)?;
        Ok(Self {
            packages: parts.packages.into_boxed_slice(),
            functions: parts.functions.into_boxed_slice(),
            operation_entries: parts.operation_entries.into_boxed_slice(),
            gateway_entries: parts.gateway_entries.into_boxed_slice(),
            exact_local_targets: parts.exact_local_targets.into_boxed_slice(),
            service_operations: parts.service_operations.into_boxed_slice(),
            actor_creates: parts.actor_creates.into_boxed_slice(),
            actor_methods: parts.actor_methods.into_boxed_slice(),
            interface_tables: parts.interface_tables.into_boxed_slice(),
            synthetic_callbacks: parts.synthetic_callbacks.into_boxed_slice(),
            callback_capture_layouts: parts.callback_capture_layouts.into_boxed_slice(),
            host_effect_adapters: parts.host_effect_adapters.into_boxed_slice(),
            intrinsics: parts.intrinsics.into_boxed_slice(),
            types: parts.types.into_boxed_slice(),
            shapes: parts.shapes.into_boxed_slice(),
            constants: parts.constants.into_boxed_slice(),
            constant_roots: parts.constant_roots.into_boxed_slice(),
            frozen_constant_nodes: parts.frozen_constant_nodes.into_boxed_slice(),
            resume_sites: parts.resume_sites.into_boxed_slice(),
            writable_paths: parts.writable_paths.into_boxed_slice(),
        })
    }

    pub fn packages(&self) -> &[LinkedPackageBytecodeProvenance] {
        &self.packages
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

    pub fn actor_creates(&self) -> &[LinkedActorCreateTarget] {
        &self.actor_creates
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

    pub fn callback_capture_layouts(&self) -> &[LinkedCallbackCaptureLayout] {
        &self.callback_capture_layouts
    }

    pub fn host_effect_adapters(&self) -> &[LinkedHostEffectAdapterTarget] {
        &self.host_effect_adapters
    }

    pub fn intrinsics(&self) -> &[LinkedIntrinsicTarget] {
        &self.intrinsics
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

    pub fn constant_roots(&self) -> &[LinkedConstantRoot] {
        &self.constant_roots
    }

    pub fn frozen_constant_nodes(&self) -> &[LinkedFrozenConstantNode] {
        &self.frozen_constant_nodes
    }

    pub fn resume_sites(&self) -> &[LinkedResumeSite] {
        &self.resume_sites
    }

    pub fn writable_paths(&self) -> &[LinkedWritablePathEntry] {
        &self.writable_paths
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateTable {
    Packages,
    Functions,
    OperationEntries,
    GatewayEntries,
    ExactLocalTargets,
    ServiceOperations,
    ActorCreates,
    ActorMethods,
    InterfaceTables,
    SyntheticCallbacks,
    CallbackCaptureLayouts,
    HostEffectAdapters,
    Intrinsics,
    Types,
    Shapes,
    Constants,
    ConstantRoots,
    FrozenConstantNodes,
    ResumeSites,
    WritablePaths,
}

impl CandidateTable {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Functions => "functions",
            Self::OperationEntries => "operation entries",
            Self::GatewayEntries => "gateway entries",
            Self::ExactLocalTargets => "exact local targets",
            Self::ServiceOperations => "service operations",
            Self::ActorCreates => "actor creates",
            Self::ActorMethods => "actor methods",
            Self::InterfaceTables => "interface tables",
            Self::SyntheticCallbacks => "synthetic callbacks",
            Self::CallbackCaptureLayouts => "callback capture layouts",
            Self::HostEffectAdapters => "host effect adapters",
            Self::Intrinsics => "intrinsics",
            Self::Types => "types",
            Self::Shapes => "shapes",
            Self::Constants => "constants",
            Self::ConstantRoots => "constant roots",
            Self::FrozenConstantNodes => "frozen constant nodes",
            Self::ResumeSites => "resume sites",
            Self::WritablePaths => "writable paths",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateLocation {
    TableRow {
        table: CandidateTable,
        row: u32,
    },
    Function {
        function: crate::FunctionIndex,
    },
    Instruction {
        function: crate::FunctionIndex,
        instruction: crate::InstructionIndex,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateReferenceKind {
    Function,
    Instruction,
    InstructionBoundary,
    FrameSlot,
    ExceptionRegion,
    ActiveRegion,
    SwitchTable,
    CallLoanLayout,
    ServiceOperation,
    ActorMethod,
    InterfaceTable,
    SyntheticCallback,
    CallbackCaptureLayout,
    HostEffectAdapter,
    Intrinsic,
    Type,
    Shape,
    Constant,
    FrozenConstantNode,
    ResumeSite,
    WritablePath,
}

impl CandidateReferenceKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Instruction => "instruction",
            Self::InstructionBoundary => "instruction boundary",
            Self::FrameSlot => "frame slot",
            Self::ExceptionRegion => "exception region",
            Self::ActiveRegion => "active region",
            Self::SwitchTable => "switch table",
            Self::CallLoanLayout => "call loan layout",
            Self::ServiceOperation => "service operation",
            Self::ActorMethod => "actor method",
            Self::InterfaceTable => "interface table",
            Self::SyntheticCallback => "synthetic callback",
            Self::CallbackCaptureLayout => "callback capture layout",
            Self::HostEffectAdapter => "host effect adapter",
            Self::Intrinsic => "intrinsic",
            Self::Type => "type",
            Self::Shape => "shape",
            Self::Constant => "constant",
            Self::FrozenConstantNode => "frozen constant node",
            Self::ResumeSite => "resume site",
            Self::WritablePath => "writable path",
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
    NonCanonicalFunctionOrder {
        previous: SpecializationKey,
        current: SpecializationKey,
    },
    DuplicatePackage {
        package_build_id: PackageBuildId,
    },
    NonCanonicalPackageOrder {
        previous: PackageBuildId,
        current: PackageBuildId,
    },
    MissingPackageProvenance {
        location: CandidateLocation,
        package_build_id: PackageBuildId,
    },
    MissingOriginSpecialization {
        location: CandidateLocation,
        key: SpecializationKey,
    },
    MissingContainerLayout {
        type_index: crate::TypeIndex,
        expected: crate::LinkedContainerLayoutKind,
    },
    UnexpectedContainerLayout {
        type_index: crate::TypeIndex,
        actual: crate::LinkedContainerLayoutKind,
    },
    ContainerLayoutKindMismatch {
        type_index: crate::TypeIndex,
        expected: crate::LinkedContainerLayoutKind,
        actual: crate::LinkedContainerLayoutKind,
    },
    ContainerBuiltinArityMismatch {
        type_index: crate::TypeIndex,
        kind: crate::LinkedContainerLayoutKind,
        expected: usize,
        actual: usize,
    },
    ContainerPositionTypeMismatch {
        type_index: crate::TypeIndex,
        position: crate::LinkedContainerPositionKind,
        actual: crate::TypeIndex,
    },
    ContainerPositionPlanMismatch {
        type_index: crate::TypeIndex,
        position: crate::LinkedContainerPositionKind,
    },
    DuplicateExactLocalTarget {
        key: SpecializationKey,
    },
    NonCanonicalExactLocalTargetOrder {
        previous: SpecializationKey,
        current: SpecializationKey,
    },
    ExactLocalTargetFunctionMismatch {
        row: u32,
        function: crate::FunctionIndex,
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
    GatewayCallableFunctionMismatch {
        gateway_entry_key: GatewayEntryKey,
        role: crate::LinkedGatewayCallableRole,
        function: crate::FunctionIndex,
    },
    DuplicateServiceOperation {
        service_requirement_key: ServiceRequirementKey,
        contract_operation_id: ContractOperationId,
    },
    DuplicateActorMethod {
        first_index: u32,
        duplicate_index: u32,
    },
    NonCanonicalActorMethodOrder {
        previous_index: u32,
        current_index: u32,
    },
    DuplicateActorCreate {
        first_index: u32,
        duplicate_index: u32,
    },
    NonCanonicalActorCreateOrder {
        previous_index: u32,
        current_index: u32,
    },
    DuplicateInterfaceTable {
        first_index: u32,
        duplicate_index: u32,
    },
    DuplicateResumeSite {
        first_index: u32,
        duplicate_index: u32,
        function: crate::FunctionIndex,
        site: crate::InstructionIndex,
    },
    DuplicateArtifactOrigin {
        table: CandidateTable,
        first_index: u32,
        duplicate_index: u32,
    },
    DuplicateConstantRoot {
        owner_package_build_id: PackageBuildId,
        symbol_path: String,
    },
    NonCanonicalConstantRootOrder {
        previous_owner: PackageBuildId,
        previous_symbol_path: String,
        current_owner: PackageBuildId,
        current_symbol_path: String,
    },
    ReferenceOutOfBounds {
        location: CandidateLocation,
        reference: CandidateReferenceKind,
        index: u32,
        len: usize,
    },
    InvalidRange {
        location: CandidateLocation,
        start: u32,
        end: u32,
    },
    NonTopologicalFrozenConstantEdge {
        node: u32,
        child: u32,
    },
    ConstantResolutionMismatch {
        constant: u32,
        node: u32,
    },
    FrozenRecordFieldCountMismatch {
        node: u32,
        shape: u32,
        child_count: usize,
        field_count: usize,
    },
    FrozenImplementationRecordMismatch {
        node: u32,
        record: u32,
    },
    FrozenBehaviorTargetMismatch {
        node: u32,
        function: u32,
    },
    EmptyFrozenImplementation {
        node: u32,
    },
    NonCanonicalFrozenBehaviorOrder {
        node: u32,
        previous: String,
        current: String,
    },
    ProgramPointCountMismatch {
        function: crate::FunctionIndex,
        instruction_count: usize,
        state_count: usize,
    },
    ProgramPointSlotCountMismatch {
        function: crate::FunctionIndex,
        instruction: crate::InstructionIndex,
        slot_count: usize,
        state_count: usize,
    },
    ProgramPointOperandDepthExceeded {
        function: crate::FunctionIndex,
        instruction: crate::InstructionIndex,
        declared_max: u32,
        actual: usize,
    },
    LoanRootNotWritable {
        function: crate::FunctionIndex,
        layout: crate::CallLoanLayoutIndex,
        root_slot: crate::FrameSlotIndex,
    },
    LoanPathSpecializationMismatch {
        function: crate::FunctionIndex,
        layout: crate::CallLoanLayoutIndex,
        path: crate::WritablePathIndex,
    },
    LoanRootTypeMismatch {
        function: crate::FunctionIndex,
        layout: crate::CallLoanLayoutIndex,
        root_slot: crate::FrameSlotIndex,
        path: crate::WritablePathIndex,
    },
    ActorTargetFunctionOwnerMismatch {
        actor_method: crate::ActorMethodIndex,
        function: crate::FunctionIndex,
    },
    ActorCreateTargetFunctionOwnerMismatch {
        actor_create: crate::ActorCreateIndex,
        function: crate::FunctionIndex,
    },
    SyntheticCallbackFunctionMismatch {
        callback: crate::SyntheticCallbackIndex,
        function: crate::FunctionIndex,
    },
    CallbackCaptureFunctionMismatch {
        layout: crate::CallbackCaptureLayoutIndex,
        function: crate::FunctionIndex,
    },
    CallbackCaptureOriginMismatch {
        layout: crate::CallbackCaptureLayoutIndex,
        function: crate::FunctionIndex,
    },
    CallbackCaptureTypeMismatch {
        layout: crate::CallbackCaptureLayoutIndex,
        slot: crate::FrameSlotIndex,
    },
    CallbackCapturePlanMismatch {
        layout: crate::CallbackCaptureLayoutIndex,
        slot: crate::FrameSlotIndex,
    },
    LifecycleAdapterRoleMismatch {
        location: CandidateLocation,
        expected: skiff_artifact_model::NativeValueAdapterRole,
        actual: skiff_artifact_model::NativeValueAdapterRole,
    },
    EmptyLifecycleAdapterBindingKey {
        location: CandidateLocation,
    },
    ZeroLifecycleAdapterAbiVersion {
        location: CandidateLocation,
        binding_key: String,
    },
}

impl fmt::Display for LinkedBytecodeCandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid linked bytecode candidate: {self:?}")
    }
}

impl std::error::Error for LinkedBytecodeCandidateError {}
