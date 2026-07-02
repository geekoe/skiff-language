//! C1 temporary adapter for legacy request heap imports.
//!
//! Owner: C1 runtime-boundary promotion.
//! Deletion point: after runtime callers import
//! `skiff_runtime_model::request_heap` directly.

pub use skiff_runtime_model::request_heap::*;
