//! Bytecode artifact schema, bounded decoder and structural validator.
//!
//! This is the Phase 1 delivery of the bytecode VM architecture contract
//! (`doc/architecture/bytecode-vm.md`, `doc/implementation/bytecode-vm/design/phase-1-artifact-schema.md`).
//!
//! Module dependency direction (single owner of the opcode table):
//!
//! ```text
//! opcodes ← dto ← encode/decode ← validate
//! ```
//!
//! - `opcodes`: the unique `OPCODE_TABLE` descriptor table (63 instructions),
//!   operand/stack/relocation vocabulary and the table fingerprint.
//! - `dto`: the artifact wire schema (`BytecodeArtifact` and friends) plus the
//!   trusted compile-time `limits` constants.
//! - `encode`: instruction/function/artifact canonical encoders.
//! - `decode`: the bounded wordcode decoder (iterative, checked arithmetic).
//! - `validate`: C1–C8 structural validation producing the opaque
//!   `StructurallyValidatedView` (C9 identity consistency is reserved for the
//!   artifact-identity task).

pub mod decode;
pub mod dto;
pub mod encode;
pub mod opcodes;
pub mod validate;

pub use decode::*;
pub use dto::*;
pub use encode::*;
pub use opcodes::*;
pub use validate::*;

#[cfg(test)]
mod tests;
