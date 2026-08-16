use std::fmt;

use skiff_artifact_model::{
    ContractOperationId, GatewayEntryKey, PackageBuildId, ServiceRequirementKey,
};

use crate::SpecializationKey;

use super::{CandidateLocation, CandidateTable};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateTypeStructureLimit {
    TypeNodes,
    TypePayloadBytes,
    TypeDepth,
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
    TypeStructureLimitExceeded {
        location: CandidateLocation,
        limit: CandidateTypeStructureLimit,
        max: u64,
        actual: u64,
    },
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
        previous: Box<SpecializationKey>,
        current: Box<SpecializationKey>,
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
    RepresentationCarrierMismatch {
        type_index: crate::TypeIndex,
        detail: &'static str,
    },
    SleepRepresentationCarrierMismatch {
        host_effect_adapter: crate::HostEffectAdapterIndex,
        detail: &'static str,
    },
    DuplicateExactLocalTarget {
        key: SpecializationKey,
    },
    NonCanonicalExactLocalTargetOrder {
        previous: Box<SpecializationKey>,
        current: Box<SpecializationKey>,
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
    MissingRemoteServiceOperation {
        location: CandidateLocation,
        method_slot: u32,
    },
    RemoteServiceOperationMismatch {
        location: CandidateLocation,
        method_slot: u32,
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
    TypePlanMismatch {
        location: CandidateLocation,
        type_index: crate::TypeIndex,
    },
    InvalidRange {
        location: CandidateLocation,
        start: u32,
        end: u32,
    },
    NonCanonicalStatementEntries {
        function: crate::FunctionIndex,
        source: skiff_artifact_model::StatementEntryValidationError,
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
    ProgramPointSlotValueMismatch {
        function: crate::FunctionIndex,
        instruction: crate::InstructionIndex,
        slot: crate::FrameSlotIndex,
        expected_type: crate::TypeIndex,
        actual_type: crate::TypeIndex,
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
    StreamProducerResultCountNotZero {
        function: crate::FunctionIndex,
        result_count: usize,
    },
    StreamProducerTypeMismatch {
        function: crate::FunctionIndex,
        stream_type: crate::TypeIndex,
    },
    EndResumeOnlyValidForStreamNext {
        resume_site: u32,
        function: crate::FunctionIndex,
        site: crate::InstructionIndex,
    },
    StreamNextMissingEndResume {
        resume_site: u32,
        function: crate::FunctionIndex,
        site: crate::InstructionIndex,
    },
    StreamNextResumeEndTargetsEqual {
        resume_site: u32,
        function: crate::FunctionIndex,
        site: crate::InstructionIndex,
        resume: crate::InstructionIndex,
        end_resume: crate::InstructionIndex,
    },
    ResumeResultMaterializationMismatch {
        resume_site: u32,
        result_index: usize,
        detail: &'static str,
    },
    EmitStreamItemShapeMismatch {
        resume_site: u32,
        detail: &'static str,
    },
    EmptyLifecycleAdapterBindingKey {
        location: CandidateLocation,
    },
    ZeroLifecycleAdapterAbiVersion {
        location: CandidateLocation,
        binding_key: String,
    },
    TaskPayloadPlanMissing {
        location: CandidateLocation,
    },
}

impl fmt::Display for LinkedBytecodeCandidateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid linked bytecode candidate: {self:?}")
    }
}

impl std::error::Error for LinkedBytecodeCandidateError {}
