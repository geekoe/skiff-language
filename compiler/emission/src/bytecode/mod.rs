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

mod constants;
mod emitter;
mod error;
mod inputs;
mod plans;

pub use emitter::emit_bytecode_artifact;
pub use error::BytecodeEmissionError;
pub use plans::{BytecodeValueTransferPlans, FunctionValueTransferPlans};
