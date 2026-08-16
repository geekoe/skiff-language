//! Public vocabulary for a concrete linked-bytecode candidate.
//!
//! A [`LinkedBytecodeCandidate`] is the bounded linker's atomic construction
//! substrate for a deployment execution image. Its constructors enforce local
//! container shape, canonical table keys and image-local reference bounds;
//! source-semantic authority remains exclusively with the compiler.

mod authority;
mod candidate;
mod frame;
mod function;
mod index;
mod instruction;
mod loan;
mod plan;
mod signature;
mod specialization;
mod stack_map;
mod tables;
mod targets;

pub use authority::{
    LinkedBytecodeAuthority, LinkedBytecodeAuthorityField, LinkedBytecodeAuthorityPins,
    LinkedBytecodeAuthorityPinsError,
};
pub use candidate::{
    CandidateLocation, CandidateReferenceKind, CandidateTable, CandidateTypeStructureLimit,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts,
    LinkedBytecodeHeaderField, LinkedPackageBytecodeProvenance,
    LinkedPackageBytecodeProvenanceError,
};
pub use frame::{LinkedFrameLayout, LinkedFrameLayoutError, LinkedParameterSlot};
pub use function::{LinkedCallableEffectDeclaration, LinkedFunction, LinkedFunctionTables};
pub use index::{
    ActiveRegionIndex, ActorCreateIndex, ActorMethodIndex, ArtifactCallbackCaptureIndex,
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ArtifactShapeIndex, ArtifactTypeIndex,
    ArtifactWritablePathIndex, BytecodePackageIndex, CallLoanLayoutIndex,
    CallbackCaptureLayoutIndex, ConstantIndex, ExceptionRegionIndex, FrameSlotIndex,
    FrozenConstantNodeIndex, FunctionIndex, HostEffectAdapterIndex, InstructionBoundaryIndex,
    InstructionIndex, InterfaceTableIndex, IntrinsicIndex, ResumeSiteIndex, ServiceOperationIndex,
    ShapeIndex, SwitchTableIndex, SyntheticCallbackIndex, TaskTargetIndex, TypeIndex,
    WritablePathIndex,
};
pub use instruction::{
    LinkedInstruction, LinkedInstructionError, LinkedInstructionTarget, LinkedResolvedOperand,
};
pub use loan::{LinkedCallLoanBinding, LinkedCallLoanLayout, LinkedCallLoanLayoutError};
pub use plan::{LinkedResourceDropPlan, LinkedValueDropPlan, LinkedValueTransferPlan};
pub use signature::{
    LinkedCallableSignature, LinkedCallableSignatureError, LinkedNativeCallableSignature,
};
pub use specialization::{ArtifactFunctionKey, ArtifactFunctionKeyParseError, SpecializationKey};
pub use stack_map::{
    LinkedProgramPointState, LinkedSlotState, LinkedStackMapCandidate,
    LinkedStackMapCandidateError, LinkedStackValue, LinkedWritableLoanState,
};
pub use tables::{
    LinkedActiveRegion, LinkedActiveRegionKind, LinkedCatchMatcher, LinkedExceptionRegion,
    LinkedResumeResultMaterialization, LinkedResumeSite, LinkedResumeSiteError,
    LinkedSourceMapEntry, LinkedStatementEntry, LinkedSwitchCase, LinkedSwitchTable,
    LinkedSwitchTableError, LinkedWritablePathEntry, LinkedWritablePathError,
    LinkedWritablePathSegment,
};
pub use targets::{
    LinkedActorCreateTarget, LinkedActorImplementationRef, LinkedActorMethodTarget,
    LinkedActorStateField, LinkedArtifactPoolOrigin, LinkedArtifactPoolOriginError,
    LinkedCallbackCapture, LinkedCallbackCaptureLayout, LinkedCallbackCaptureLayoutError,
    LinkedCallbackInterfaceMethod, LinkedConstantEntry, LinkedConstantReference,
    LinkedConstantRoot, LinkedConstantSymbolPath, LinkedConstantSymbolPathParseError,
    LinkedContainerLayout, LinkedContainerLayoutKind, LinkedContainerPosition,
    LinkedContainerPositionKind, LinkedDbObjectTargetId, LinkedDbOperation, LinkedExactLocalTarget,
    LinkedFrozenBehaviorBinding, LinkedFrozenConstantNode, LinkedFrozenConstantValue,
    LinkedGatewayCallable, LinkedGatewayCallableRole, LinkedGatewayEntry, LinkedGatewayEntryError,
    LinkedHostBindingKey, LinkedHostEffectAdapterTarget, LinkedHostTargetError,
    LinkedInterfaceInstantiation, LinkedInterfaceMethodAbiId, LinkedInterfaceRequirementMethod,
    LinkedInterfaceRequirementTable, LinkedInterfaceTable, LinkedInterfaceTableError,
    LinkedInterfaceTableKind, LinkedInterfaceTextError, LinkedInterfaceTextKind,
    LinkedIntrinsicCanonicalKey, LinkedIntrinsicKind, LinkedIntrinsicTarget,
    LinkedIntrinsicTargetError, LinkedLocalInterfaceMethod, LinkedLocalInterfaceTable,
    LinkedOperationEntry, LinkedOperationReceiver, LinkedPublicInstanceKey,
    LinkedRemoteInterfaceMethod, LinkedRemoteInterfaceTable, LinkedRepresentationCarrier,
    LinkedServiceBoundaryErrorPlan, LinkedServiceBoundaryPlan, LinkedServiceBoundaryValue,
    LinkedServiceCallbackPlan, LinkedServiceOperationTarget, LinkedShapeEntry, LinkedShapeError,
    LinkedShapeField, LinkedStaticIntrinsicTarget, LinkedSyntheticCallbackTarget,
    LinkedTaskPayloadParameter, LinkedTaskPayloadPlan, LinkedTaskTarget, LinkedTaskTargetError,
    LinkedTaskTiming, LinkedTypeEntry,
};

#[cfg(test)]
mod tests;
