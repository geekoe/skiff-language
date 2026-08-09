use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::Digest;

use crate::NativeValueLifecycleResolution;

use super::contract::{
    ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueLifecyclePolicyError,
};

pub(super) struct ClassificationContext<'a, R: ValueLifecycleFactResolver> {
    pub(super) resolver: &'a mut R,
    pub(super) budget: &'a mut ValueLifecyclePolicyBudget,
    pub(super) state: TraversalState,
}

impl<'a, R: ValueLifecycleFactResolver> ClassificationContext<'a, R> {
    pub(super) fn new(resolver: &'a mut R, budget: &'a mut ValueLifecyclePolicyBudget) -> Self {
        Self {
            resolver,
            budget,
            state: TraversalState::default(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TraversalState {
    visiting: BTreeSet<String>,
    memo: BTreeMap<String, NativeValueLifecycleResolution>,
}

impl TraversalState {
    pub(super) fn exact_key<T: Serialize>(
        domain: &str,
        value: &T,
    ) -> Result<String, ValueLifecyclePolicyError> {
        let bytes = skiff_canonical_json::canonical_json_bytes(value).map_err(|error| {
            ValueLifecyclePolicyError::CanonicalProjection {
                message: error.to_string(),
            }
        })?;
        Ok(format!(
            "{domain}:{}",
            hex::encode(sha2::Sha256::digest(bytes))
        ))
    }

    pub(super) fn begin(
        &mut self,
        key: &str,
    ) -> Result<Option<NativeValueLifecycleResolution>, ValueLifecyclePolicyError> {
        if let Some(cached) = self.memo.get(key) {
            return Ok(Some(cached.clone()));
        }
        if !self.visiting.insert(key.to_string()) {
            return Err(ValueLifecyclePolicyError::DescriptorCycle {
                key: key.to_string(),
            });
        }
        Ok(None)
    }

    pub(super) fn finish(
        &mut self,
        key: String,
        result: &Result<NativeValueLifecycleResolution, ValueLifecyclePolicyError>,
    ) {
        self.visiting.remove(&key);
        if let Ok(resolution) = result {
            self.memo.insert(key, resolution.clone());
        }
    }
}
