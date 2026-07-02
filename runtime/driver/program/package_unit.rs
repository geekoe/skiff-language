//! LP2 temporary adapter for promoted package linked model types.
//!
//! Owner: skiff-runtime-linked-program DTO contract.
//! Deletion/narrowing point: after runtime host/request callers import
//! `skiff_runtime_linked_program::package_unit` directly or the surface is
//! narrowed to activation/host-only helpers.

pub use skiff_runtime_linked_program::package_unit::*;
