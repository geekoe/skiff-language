//! Source-free deployment projection and runtime assembly construction.
//!
//! Phase 03 consumers share the DTO validation and fixtures from this crate.
//! The `projection` and `assembly` modules are intentionally independent shells
//! until their respective checkpoint tasks implement them.

pub mod assembly;
pub mod error;
pub mod fixtures;
pub mod projection;
pub mod validation;

pub use error::{DeploymentError, Result};

#[cfg(test)]
mod tests;
