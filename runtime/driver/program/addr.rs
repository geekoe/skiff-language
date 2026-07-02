//! F1 temporary adapter for legacy program address imports.
//!
//! Owner: F1.
//! Deletion/narrowing point: after F1 downstream users import
//! `skiff_runtime_model::addr` directly or this adapter is narrowed to
//! test-only fixtures.

pub use skiff_runtime_model::addr::*;
