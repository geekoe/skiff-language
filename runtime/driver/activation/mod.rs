//! C1 temporary runtime-local adapter for promoted activation types.
//!
//! Owner: C1 skiff-runtime-activation promotion.
//! Deletion/narrowing point: after runtime callers import
//! `skiff_runtime_activation` directly, remove this adapter.

#![allow(unused_imports)]

pub mod cache {
    pub use skiff_runtime_activation::RuntimeActivationCache;
}

pub use skiff_runtime_activation::{
    build_runtime_activation_for_image, RemovedRuntimeActivationCacheEntry, RuntimeActivation,
    RuntimeActivationCache, RuntimeActivationCacheEntry, RuntimeActivationCacheEvictionCandidate,
    RuntimeActivationCacheStats,
};
