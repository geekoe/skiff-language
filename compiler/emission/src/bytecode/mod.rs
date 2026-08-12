//! Deterministic MIR-to-bytecode emission.
//!
//! This module is the only compiler owner allowed to construct canonical
//! `BytecodeArtifact` program bodies. Its source-program input is public,
//! self-contained [`skiff_compiler_lowering::mir`] only. It never reads File
//! IR, AST, source text or projection JSON to recover an expression, type,
//! target, liveness, effect or source fact.
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

pub use admission::admit_phase_1_bytecode_mir;
pub use emitter::emit_bytecode_artifact;
pub use error::{BytecodeEmissionError, Phase1UnsupportedCapability};
pub use plans::{
    derive_bytecode_value_transfer_plans, BytecodeValueTransferPlans, FunctionValueTransferPlans,
};
