use std::fmt;

use serde::Serialize;

use crate::PlatformErrorProjectionKey;

/// Operand word kind. Operand order is word order in the encoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperandKind {
    Immediate,
    Branch,
    Slot,
    Pool,
    Table,
    Reloc,
}

impl OperandKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Branch => "branch",
            Self::Slot => "slot",
            Self::Pool => "pool",
            Self::Table => "table",
            Self::Reloc => "reloc",
        }
    }
}

/// Semantic role of one operand word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OperandRole {
    SourceSlot,
    DestinationSlot,
    Slot,
    BranchTarget,
    SwitchTable,
    /// Retained only for the Phase 1 descriptor API. No ISA v4 opcode has an
    /// exception-region operand; exception regions are interval tables.
    Region,
    ActiveRegion,
    LocalTarget,
    ServiceTarget,
    ActorTarget,
    InterfaceTarget,
    CallbackTarget,
    HostTarget,
    IntrinsicTarget,
    ArgCount,
    InputCount,
    ResultCount,
    SelectorCount,
    FailureKind,
    CaptureCount,
    FieldCount,
    MethodOrdinal,
    FieldOrdinal,
    ConstantRef,
    TypeRef,
    ElementTypeRef,
    KeyTypeRef,
    ValueTypeRef,
    ShapeRef,
    WritablePathRef,
    CaptureLayoutRef,
    ResumeRef,
    CallLoanLayout,
}

impl OperandRole {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SourceSlot => "sourceSlot",
            Self::DestinationSlot => "destinationSlot",
            Self::Slot => "slot",
            Self::BranchTarget => "branchTarget",
            Self::SwitchTable => "switchTable",
            Self::Region => "region",
            Self::ActiveRegion => "activeRegion",
            Self::LocalTarget => "localTarget",
            Self::ServiceTarget => "serviceTarget",
            Self::ActorTarget => "actorTarget",
            Self::InterfaceTarget => "interfaceTarget",
            Self::CallbackTarget => "callbackTarget",
            Self::HostTarget => "hostTarget",
            Self::IntrinsicTarget => "intrinsicTarget",
            Self::ArgCount => "argCount",
            Self::InputCount => "inputCount",
            Self::ResultCount => "resultCount",
            Self::SelectorCount => "selectorCount",
            Self::FailureKind => "failureKind",
            Self::CaptureCount => "captureCount",
            Self::FieldCount => "fieldCount",
            Self::MethodOrdinal => "methodOrdinal",
            Self::FieldOrdinal => "fieldOrdinal",
            Self::ConstantRef => "constantRef",
            Self::TypeRef => "typeRef",
            Self::ElementTypeRef => "elementTypeRef",
            Self::KeyTypeRef => "keyTypeRef",
            Self::ValueTypeRef => "valueTypeRef",
            Self::ShapeRef => "shapeRef",
            Self::WritablePathRef => "writablePathRef",
            Self::CaptureLayoutRef => "captureLayoutRef",
            Self::ResumeRef => "resumeRef",
            Self::CallLoanLayout => "callLoanLayout",
        }
    }

    /// Compatibility projection for the pre-contract descriptor API. New
    /// consumers must use [`OperandSpec::kind`] instead of inferring kind from
    /// a role.
    pub const fn operand_kind(self) -> OperandKind {
        match self {
            Self::SourceSlot | Self::DestinationSlot | Self::Slot => OperandKind::Slot,
            Self::BranchTarget => OperandKind::Branch,
            Self::SwitchTable | Self::Region | Self::ActiveRegion | Self::CallLoanLayout => {
                OperandKind::Table
            }
            Self::LocalTarget
            | Self::ServiceTarget
            | Self::ActorTarget
            | Self::InterfaceTarget
            | Self::CallbackTarget
            | Self::HostTarget
            | Self::IntrinsicTarget => OperandKind::Reloc,
            Self::ArgCount
            | Self::InputCount
            | Self::ResultCount
            | Self::SelectorCount
            | Self::FailureKind
            | Self::CaptureCount
            | Self::FieldCount
            | Self::MethodOrdinal
            | Self::FieldOrdinal => OperandKind::Immediate,
            Self::ConstantRef
            | Self::TypeRef
            | Self::ElementTypeRef
            | Self::KeyTypeRef
            | Self::ValueTypeRef
            | Self::ShapeRef
            | Self::WritablePathRef
            | Self::CaptureLayoutRef
            | Self::ResumeRef => OperandKind::Pool,
        }
    }

    pub const fn pool_category(self) -> Option<PoolCategory> {
        match self {
            Self::ConstantRef => Some(PoolCategory::Constants),
            Self::TypeRef | Self::ElementTypeRef | Self::KeyTypeRef | Self::ValueTypeRef => {
                Some(PoolCategory::Types)
            }
            Self::ShapeRef => Some(PoolCategory::Shapes),
            Self::WritablePathRef => Some(PoolCategory::WritablePaths),
            Self::CaptureLayoutRef => Some(PoolCategory::CallbackCapture),
            Self::ResumeRef => Some(PoolCategory::Resume),
            _ => None,
        }
    }

    pub const fn table_category(self) -> Option<TableCategory> {
        match self {
            Self::SwitchTable => Some(TableCategory::SwitchTables),
            Self::Region => Some(TableCategory::ExceptionRegions),
            Self::ActiveRegion => Some(TableCategory::ActiveRegions),
            Self::CallLoanLayout => Some(TableCategory::CallLoanLayouts),
            _ => None,
        }
    }
}

/// Relocation kinds stored by an artifact function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelocationKind {
    LocalExecutableRef,
    PackageCallableRef,
    ServiceOperationRef,
    ActorMethodRef,
    InterfaceRequirementRef,
    LocalInterfaceRef,
    RemoteInterfaceRef,
    SyntheticCallbackRef,
    TaskSubmitRef,
    HostEffectRef,
    IntrinsicRef,
    TypeRef,
    ShapeRef,
    FrozenConstantRef,
}

impl RelocationKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::LocalExecutableRef => "localExecutableRef",
            Self::PackageCallableRef => "packageCallableRef",
            Self::ServiceOperationRef => "serviceOperationRef",
            Self::ActorMethodRef => "actorMethodRef",
            Self::InterfaceRequirementRef => "interfaceRequirementRef",
            Self::LocalInterfaceRef => "localInterfaceRef",
            Self::RemoteInterfaceRef => "remoteInterfaceRef",
            Self::SyntheticCallbackRef => "syntheticCallbackRef",
            Self::TaskSubmitRef => "taskSubmitRef",
            Self::HostEffectRef => "hostEffectRef",
            Self::IntrinsicRef => "intrinsicRef",
            Self::TypeRef => "typeRef",
            Self::ShapeRef => "shapeRef",
            Self::FrozenConstantRef => "frozenConstantRef",
        }
    }
}

/// Image-local target kind produced by the deployment linker for one operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkedOperandKind {
    Immediate,
    Instruction,
    FrameSlot,
    SwitchTable,
    ActiveRegion,
    CallLoanLayout,
    Function,
    ServiceOperation,
    ActorMethod,
    InterfaceTable,
    SyntheticCallback,
    HostEffectAdapter,
    Intrinsic,
    Constant,
    Type,
    Shape,
    WritablePath,
    CallbackCaptureLayout,
    ResumeSite,
}

impl LinkedOperandKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Instruction => "instruction",
            Self::FrameSlot => "frameSlot",
            Self::SwitchTable => "switchTable",
            Self::ActiveRegion => "activeRegion",
            Self::CallLoanLayout => "callLoanLayout",
            Self::Function => "function",
            Self::ServiceOperation => "serviceOperation",
            Self::ActorMethod => "actorMethod",
            Self::InterfaceTable => "interfaceTable",
            Self::SyntheticCallback => "syntheticCallback",
            Self::HostEffectAdapter => "hostEffectAdapter",
            Self::Intrinsic => "intrinsic",
            Self::Constant => "constant",
            Self::Type => "type",
            Self::Shape => "shape",
            Self::WritablePath => "writablePath",
            Self::CallbackCaptureLayout => "callbackCaptureLayout",
            Self::ResumeSite => "resumeSite",
        }
    }

    pub const fn operand_kind(self) -> OperandKind {
        match self {
            Self::Immediate => OperandKind::Immediate,
            Self::Instruction => OperandKind::Branch,
            Self::FrameSlot => OperandKind::Slot,
            Self::SwitchTable | Self::ActiveRegion | Self::CallLoanLayout => OperandKind::Table,
            Self::Function
            | Self::ServiceOperation
            | Self::ActorMethod
            | Self::InterfaceTable
            | Self::SyntheticCallback
            | Self::HostEffectAdapter
            | Self::Intrinsic => OperandKind::Reloc,
            Self::Constant
            | Self::Type
            | Self::Shape
            | Self::WritablePath
            | Self::CallbackCaptureLayout
            | Self::ResumeSite => OperandKind::Pool,
        }
    }

    pub const fn pool_category(self) -> Option<PoolCategory> {
        match self {
            Self::Constant => Some(PoolCategory::Constants),
            Self::Type => Some(PoolCategory::Types),
            Self::Shape => Some(PoolCategory::Shapes),
            Self::WritablePath => Some(PoolCategory::WritablePaths),
            Self::CallbackCaptureLayout => Some(PoolCategory::CallbackCapture),
            Self::ResumeSite => Some(PoolCategory::Resume),
            _ => None,
        }
    }

    pub const fn table_category(self) -> Option<TableCategory> {
        match self {
            Self::SwitchTable => Some(TableCategory::SwitchTables),
            Self::ActiveRegion => Some(TableCategory::ActiveRegions),
            Self::CallLoanLayout => Some(TableCategory::CallLoanLayouts),
            _ => None,
        }
    }
}

/// One complete operand contract. Relocation compatibility belongs to the
/// operand itself rather than to a row-wide opcode allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperandSpec {
    pub kind: OperandKind,
    pub role: OperandRole,
    pub linked_kind: LinkedOperandKind,
    pub allowed_relocations: &'static [RelocationKind],
}

impl OperandSpec {
    pub const fn new(
        kind: OperandKind,
        role: OperandRole,
        linked_kind: LinkedOperandKind,
        allowed_relocations: &'static [RelocationKind],
    ) -> Self {
        assert!(kind as u8 == linked_kind.operand_kind() as u8);
        assert!(kind as u8 == role.operand_kind() as u8);
        assert!(
            kind as u8 == OperandKind::Reloc as u8 || allowed_relocations.is_empty(),
            "only relocation operands may declare relocation kinds"
        );
        assert!(
            kind as u8 != OperandKind::Reloc as u8 || !allowed_relocations.is_empty(),
            "every relocation operand needs an exact non-empty allowlist"
        );
        Self {
            kind,
            role,
            linked_kind,
            allowed_relocations,
        }
    }
}

/// Stack-effect arity source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arity {
    Fixed(u16),
    Declared(OperandRole),
    FunctionResultCount,
}

/// Compatibility stack effect retained for the Phase 1 validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackEffect {
    pub arity: Arity,
}

impl StackEffect {
    pub const fn new(arity: Arity) -> Self {
        Self { arity }
    }
}

/// Artifact-level pool categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoolCategory {
    Constants,
    Types,
    Shapes,
    Effects,
    Resume,
    CallbackCapture,
    WritablePaths,
}

impl PoolCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Constants => "constants",
            Self::Types => "types",
            Self::Shapes => "shapes",
            Self::Effects => "effects",
            Self::Resume => "resume",
            Self::CallbackCapture => "callbackCapture",
            Self::WritablePaths => "writablePaths",
        }
    }

    pub const fn expected_entry_kind(self) -> PoolEntryKind {
        match self {
            Self::Constants => PoolEntryKind::ConstantRef,
            Self::Types => PoolEntryKind::TypeRef,
            Self::Shapes => PoolEntryKind::ShapeRef,
            Self::Effects => PoolEntryKind::HostEffectRef,
            Self::Resume => PoolEntryKind::ResumeDescriptor,
            Self::CallbackCapture => PoolEntryKind::CallbackCaptureLayout,
            Self::WritablePaths => PoolEntryKind::WritablePath,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoolEntryKind {
    ConstantRef,
    TypeRef,
    ShapeRef,
    HostEffectRef,
    ResumeDescriptor,
    CallbackCaptureLayout,
    WritablePath,
}

/// Function-local auxiliary table categories. `ExceptionRegions` is retained
/// for the legacy public enum, but no operand in the canonical table selects
/// it; static catch lookup is range-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableCategory {
    ExceptionRegions,
    ActiveRegions,
    SwitchTables,
    CallLoanLayouts,
}

impl TableCategory {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ExceptionRegions => "exceptionRegions",
            Self::ActiveRegions => "activeRegions",
            Self::SwitchTables => "switchTables",
            Self::CallLoanLayouts => "callLoanLayouts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchWhen {
    True,
    False,
}

impl BranchWhen {
    pub const fn name(self) -> &'static str {
        match self {
            Self::True => "true",
            Self::False => "false",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlContract {
    Fallthrough,
    Jump {
        target: OperandRole,
    },
    Branch {
        target: OperandRole,
        when: BranchWhen,
    },
    Switch {
        table: OperandRole,
    },
    Return,
    TailCall,
    Raise,
    Rethrow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingMode {
    ServiceBoundary,
    ActorBoundary,
    InterfaceBoundary,
    CallbackBoundary,
    HostEffect,
    StreamRead,
    StreamBackpressure,
}

impl PendingMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ServiceBoundary => "serviceBoundary",
            Self::ActorBoundary => "actorBoundary",
            Self::InterfaceBoundary => "interfaceBoundary",
            Self::CallbackBoundary => "callbackBoundary",
            Self::HostEffect => "hostEffect",
            Self::StreamRead => "streamRead",
            Self::StreamBackpressure => "streamBackpressure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingContract {
    Never,
    TransitiveTarget {
        target: OperandRole,
    },
    NoPendingTarget {
        target: OperandRole,
        loan_layout: OperandRole,
    },
    ActualWithResume {
        resume: OperandRole,
        mode: PendingMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeoutAttribution {
    ActiveRegionSite,
}

impl TimeoutAttribution {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ActiveRegionSite => "activeRegionSite",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointContract {
    None,
    Budget {
        budget_stop: FailureDisposition,
        timeout_attribution: TimeoutAttribution,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    Assertion,
    BudgetStop,
    ScalarNonFinite,
    DivideByZero,
    CollectionIndexOutOfBounds,
    CollectionMissingKey,
    WritablePathIntermediateMissingKey,
    MapEntryIndexOutOfBounds,
    WritablePathTypeInvariant,
    WritablePathCowInvariant,
}

impl FailureKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Assertion => "assertion",
            Self::BudgetStop => "budgetStop",
            Self::ScalarNonFinite => "scalarNonFinite",
            Self::DivideByZero => "divideByZero",
            Self::CollectionIndexOutOfBounds => "collectionIndexOutOfBounds",
            Self::CollectionMissingKey => "collectionMissingKey",
            Self::WritablePathIntermediateMissingKey => "writablePathIntermediateMissingKey",
            Self::MapEntryIndexOutOfBounds => "mapEntryIndexOutOfBounds",
            Self::WritablePathTypeInvariant => "writablePathTypeInvariant",
            Self::WritablePathCowInvariant => "writablePathCowInvariant",
        }
    }
}

/// Exact runtime predicate that selects a checked-failure edge. Keeping the
/// trigger separate from the error identity makes success polarity and
/// boundary cases part of the fingerprinted contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureTrigger {
    AssertionFalse,
    BudgetExceeded,
    NonFiniteResult,
    ZeroDivisorIncludingNegativeZero,
    IndexOutOfBounds,
    MissingKey,
    IntermediateMissingKey,
    InternalTypeInvariant,
    InternalCowInvariant,
}

impl FailureTrigger {
    pub const fn name(self) -> &'static str {
        match self {
            Self::AssertionFalse => "assertionFalse",
            Self::BudgetExceeded => "budgetExceeded",
            Self::NonFiniteResult => "nonFiniteResult",
            Self::ZeroDivisorIncludingNegativeZero => "zeroDivisorIncludingNegativeZero",
            Self::IndexOutOfBounds => "indexOutOfBounds",
            Self::MissingKey => "missingKey",
            Self::IntermediateMissingKey => "intermediateMissingKey",
            Self::InternalTypeInvariant => "internalTypeInvariant",
            Self::InternalCowInvariant => "internalCowInvariant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureDisposition {
    Catchable {
        projection_key: PlatformErrorProjectionKey,
    },
    UncatchableTerminal,
    InvariantTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureContract {
    pub kind: FailureKind,
    pub trigger: FailureTrigger,
    pub disposition: FailureDisposition,
}

impl FailureContract {
    pub const fn catchable(
        kind: FailureKind,
        trigger: FailureTrigger,
        projection_key: PlatformErrorProjectionKey,
    ) -> Self {
        Self {
            kind,
            trigger,
            disposition: FailureDisposition::Catchable { projection_key },
        }
    }

    pub const fn terminal(kind: FailureKind, trigger: FailureTrigger) -> Self {
        Self {
            kind,
            trigger,
            disposition: FailureDisposition::UncatchableTerminal,
        }
    }

    pub const fn invariant(kind: FailureKind, trigger: FailureTrigger) -> Self {
        Self {
            kind,
            trigger,
            disposition: FailureDisposition::InvariantTerminal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionBehavior {
    None,
    PropagateTarget { target: OperandRole },
    RaiseAtCurrentSite,
    ThrowValue { type_ref: OperandRole },
    PreserveOriginal { source_slot: OperandRole },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionContract {
    pub behavior: ExceptionBehavior,
    pub failures: &'static [FailureContract],
}

impl ExceptionContract {
    pub const fn new(behavior: ExceptionBehavior, failures: &'static [FailureContract]) -> Self {
        Self { behavior, failures }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceUse {
    Assertion,
    CallSite,
    EffectSite,
    StreamSite,
    ThrowOrigin,
    GeneratedFailure,
    InstructionFailure,
}

impl SourceUse {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Assertion => "assertion",
            Self::CallSite => "callSite",
            Self::EffectSite => "effectSite",
            Self::StreamSite => "streamSite",
            Self::ThrowOrigin => "throwOrigin",
            Self::GeneratedFailure => "generatedFailure",
            Self::InstructionFailure => "instructionFailure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceOriginConstraint {
    SourceOrSynthetic,
    SyntheticOnly,
}

impl SourceOriginConstraint {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SourceOrSynthetic => "sourceOrSynthetic",
            Self::SyntheticOnly => "syntheticOnly",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceContract {
    None,
    Required {
        use_kind: SourceUse,
        origin: SourceOriginConstraint,
    },
    PreserveOriginal,
    ActiveRegion {
        operand: OperandRole,
    },
}

impl SourceContract {
    pub const fn requires_source_map(self) -> bool {
        matches!(self, Self::Required { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEffect {
    NotApplicable,
    Preserve,
    Enter { operand: OperandRole },
    Leave { operand: OperandRole },
    ExitFunction,
    TailReplace,
    Unwind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionContract {
    pub normal: RegionEffect,
    pub raised: RegionEffect,
}

impl RegionContract {
    pub const fn new(normal: RegionEffect, raised: RegionEffect) -> Self {
        Self { normal, raised }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityRequirement {
    ShareableValue,
    MovableValue,
    DroppableValue,
    NominalTag,
    ExactLocalTarget,
    InOutLoan,
    NoPendingTarget,
    TailEligible,
    LocalInterfaceTable,
    RemoteInterfaceTable,
    CallbackCapture,
    CallbackInvocation,
    VerifiedShape,
    AffineFieldTake,
    VerifiedWritablePath,
    RepresentationType,
    ArrayBuilderToken,
    MapBuilderToken,
    CanonicalMapSnapshot,
    WritablePathFinalMapUpsert,
    InternalGenerated,
    StreamConsumer,
    StreamProducer,
    TrustedHostAdapter,
    TrustedIntrinsic,
}

impl CapabilityRequirement {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ShareableValue => "shareableValue",
            Self::MovableValue => "movableValue",
            Self::DroppableValue => "droppableValue",
            Self::NominalTag => "nominalTag",
            Self::ExactLocalTarget => "exactLocalTarget",
            Self::InOutLoan => "inOutLoan",
            Self::NoPendingTarget => "noPendingTarget",
            Self::TailEligible => "tailEligible",
            Self::LocalInterfaceTable => "localInterfaceTable",
            Self::RemoteInterfaceTable => "remoteInterfaceTable",
            Self::CallbackCapture => "callbackCapture",
            Self::CallbackInvocation => "callbackInvocation",
            Self::VerifiedShape => "verifiedShape",
            Self::AffineFieldTake => "affineFieldTake",
            Self::VerifiedWritablePath => "verifiedWritablePath",
            Self::RepresentationType => "representationType",
            Self::ArrayBuilderToken => "arrayBuilderToken",
            Self::MapBuilderToken => "mapBuilderToken",
            Self::CanonicalMapSnapshot => "canonicalMapSnapshot",
            Self::WritablePathFinalMapUpsert => "writablePathFinalMapUpsert",
            Self::InternalGenerated => "internalGenerated",
            Self::StreamConsumer => "streamConsumer",
            Self::StreamProducer => "streamProducer",
            Self::TrustedHostAdapter => "trustedHostAdapter",
            Self::TrustedIntrinsic => "trustedIntrinsic",
        }
    }
}

/// Closed failure kinds accepted by `trap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum TrapFailureKind {
    Assertion = 0,
}

impl TrapFailureKind {
    pub const fn from_encoded(encoded: u32) -> Option<Self> {
        match encoded {
            0 => Some(Self::Assertion),
            _ => None,
        }
    }
}

/// Phase 1 compatibility descriptor generated from the same row as the full
/// [`super::OpcodeContract`]. It contains no independently authored facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeDescriptor {
    pub kind: super::Opcode,
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub operand_layout: &'static [OperandKind],
    pub operand_roles: &'static [OperandRole],
    pub stack_in: &'static [StackEffect],
    pub stack_out: &'static [StackEffect],
    pub allowed_relocations: &'static [RelocationKind],
}

impl OpcodeDescriptor {
    pub const fn operand_word_count(&self) -> u32 {
        self.operand_layout.len() as u32
    }

    pub const fn instruction_word_count(&self) -> u32 {
        self.operand_word_count() + 1
    }

    pub fn operand_position(&self, role: OperandRole) -> Option<usize> {
        self.operand_roles
            .iter()
            .position(|candidate| *candidate == role)
    }

    pub fn operand_word(&self, role: OperandRole, operand_words: &[u32]) -> Option<u32> {
        self.operand_position(role)
            .and_then(|position| operand_words.get(position).copied())
    }
}

impl fmt::Display for OpcodeDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (0x{:02x})", self.mnemonic, self.opcode)
    }
}
