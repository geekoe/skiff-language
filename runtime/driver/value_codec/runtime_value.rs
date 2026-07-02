//! C1 temporary adapter for legacy runtime value imports.
//!
//! Owner: C1 runtime-boundary promotion.
//! Deletion point: after runtime callers import
//! `skiff_runtime_model::runtime_value` or `skiff_runtime_model::value`
//! directly.

pub use skiff_runtime_model::runtime_value::*;
