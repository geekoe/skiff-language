//! C1 temporary adapter for legacy runtime value graph imports.
//!
//! Owner: C1 runtime-boundary promotion.
//! Deletion point: after runtime callers import
//! `skiff_runtime_model::runtime_value_graph` directly.

#[allow(unused_imports)]
pub use skiff_runtime_model::runtime_value_graph::*;
