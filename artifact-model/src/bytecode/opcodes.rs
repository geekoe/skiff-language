//! Canonical bytecode opcode contracts.
//!
//! [`OpcodeContract`] is the single semantic owner shared by the encoder,
//! linker, verifier and VM. The 63-row declaration in `table` generates the
//! semantic [`Opcode`] enum, [`Opcode::ALL`], the contract table and the
//! temporary [`OpcodeDescriptor`] compatibility view consumed by the Phase 1
//! structural validator. No consumer should maintain a second opcode match
//! that restates contract facts.

mod fingerprint;
mod model;
mod statement;
mod table;
mod typed;

pub use fingerprint::*;
pub use model::*;
pub use statement::*;
pub use table::*;
pub use typed::*;

#[cfg(test)]
mod tests;
