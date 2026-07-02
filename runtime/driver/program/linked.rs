//! LP2 temporary adapter for promoted linked IR DTOs.
//!
//! Owner: skiff-runtime-linked-program DTO contract.
//! Deletion/narrowing point: after runtime eval/request callers import
//! `skiff_runtime_linked_program::linked` or narrower DTO modules directly.

pub use skiff_runtime_linked_program::linked::*;
