//! Public vocabulary for an unverified, concrete linked-bytecode candidate.
//!
//! A [`LinkedBytecodeCandidate`] is linker output and verifier input. Its
//! constructors enforce only local container shape, dense table indices and
//! root-to-function bounds;
//! possession of a candidate never implies semantic verification or VM
//! executability.

mod candidate;
mod frame;
mod function;
mod index;
mod instruction;
mod signature;
mod specialization;
mod tables;
mod targets;

pub use candidate::{
    CandidateTable, LinkedBytecodeCandidate, LinkedBytecodeCandidateError,
    LinkedBytecodeCandidateParts,
};
pub use frame::{LinkedFrameLayout, LinkedFrameLayoutError};
pub use function::{LinkedCallableEffectDeclaration, LinkedFunction, LinkedFunctionTables};
pub use index::{
    ActorMethodIndex, ConstantIndex, FrameSlotIndex, FunctionIndex, HostEffectAdapterIndex,
    InstructionIndex, InterfaceTableIndex, ResumeSiteIndex, ServiceOperationIndex, ShapeIndex,
    SyntheticCallbackIndex, TypeIndex,
};
pub use instruction::LinkedInstruction;
pub use signature::{LinkedCallableSignature, LinkedCallableSignatureError};
pub use specialization::SpecializationKey;
pub use tables::{
    LinkedCatchMatcher, LinkedExceptionRegion, LinkedResumeSite, LinkedSourceMapEntry,
    LinkedStatementEntry, LinkedSwitchTable,
};
pub use targets::{
    LinkedActorMethodTarget, LinkedCallbackCapture, LinkedConstantEntry, LinkedConstantValue,
    LinkedExactLocalTarget, LinkedGatewayEntry, LinkedHostEffectAdapterTarget,
    LinkedInterfaceMethod, LinkedInterfaceTable, LinkedOperationEntry,
    LinkedServiceOperationTarget, LinkedShapeEntry, LinkedSyntheticCallbackTarget, LinkedTypeEntry,
};

#[cfg(test)]
mod tests;
