//! Deterministic MIR-to-bytecode emission.
//!
//! This module is the only compiler owner allowed to construct canonical
//! `BytecodeArtifact` program bodies. Its public source-program input is an
//! opaque Phase 1 admission proof over self-contained
//! [`skiff_compiler_lowering::mir`]. It never reads File IR, AST, source text
//! or projection JSON to recover an expression, type, target, liveness, effect
//! or source fact.
//!
//! Frozen constants and [`BytecodeValueTransferPlans`] are separate explicit
//! inputs. Missing, extra, ambiguous or incomplete facts are compilation
//! errors; the emitter never invents `SnapshotShare`, omits unsupported code,
//! or returns a partial artifact.

mod admission;
mod constants;
mod emitter;
mod error;
mod functions;
mod inputs;
mod plans;

pub use admission::{admit_phase_1_bytecode_mir, AdmittedPhase1BytecodeMir};
pub use emitter::emit_bytecode_artifact;
pub use error::{
    BytecodeEmissionError, Phase1MirFactMismatch, Phase1UnsupportedCapability,
};
pub use skiff_compiler_lowering::mir::MirSourceEventUnavailableReason;
pub use plans::{
    derive_bytecode_value_transfer_plans, BytecodeValueTransferPlans, FunctionValueTransferPlans,
};

#[cfg(test)]
#[path = "tests/bytecode_emitter_constants.rs"]
mod bytecode_emitter_constants_tests;
#[cfg(test)]
#[path = "tests/bytecode_emitter_core.rs"]
mod bytecode_emitter_core_tests;
#[cfg(test)]
#[path = "tests/phase_1_admission.rs"]
mod phase_1_admission_tests;
