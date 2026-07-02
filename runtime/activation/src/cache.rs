use std::{
    collections::HashMap,
    mem,
    sync::{Arc, RwLock},
    time::Instant,
};

use serde::Serialize;
use skiff_runtime_linked_program::RuntimeProgramIdentity;

use crate::RuntimeActivation;

#[derive(Debug)]
pub struct RuntimeActivationCache {
    entries: RwLock<HashMap<String, CacheEntry<RuntimeActivationCacheEntry>>>,
}

impl Default for RuntimeActivationCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeActivationCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get_by_activation_identity(
        &self,
        activation_identity: impl AsRef<str>,
    ) -> Option<Arc<RuntimeActivationCacheEntry>> {
        self.entries
            .write()
            .expect("runtime activation cache lock poisoned")
            .get_mut(activation_identity.as_ref())
            .map(CacheEntry::touch)
    }

    pub fn get_by_dynamic_build_id(
        &self,
        dynamic_build_id: impl AsRef<str>,
    ) -> Option<Arc<RuntimeActivationCacheEntry>> {
        self.get_by_activation_identity(dynamic_build_id)
    }

    pub fn insert(
        &self,
        identity: RuntimeProgramIdentity,
        activation: RuntimeActivation,
    ) -> Arc<RuntimeActivationCacheEntry> {
        self.insert_arc(identity, Arc::new(activation))
    }

    pub fn insert_arc(
        &self,
        identity: RuntimeProgramIdentity,
        activation: Arc<RuntimeActivation>,
    ) -> Arc<RuntimeActivationCacheEntry> {
        let estimated_size_bytes =
            runtime_activation_cache_entry_estimated_size(&identity, activation.as_ref());
        self.insert_arc_with_estimate(identity, activation, estimated_size_bytes)
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .expect("runtime activation cache lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> RuntimeActivationCacheStats {
        let entries = self
            .entries
            .read()
            .expect("runtime activation cache lock poisoned");
        RuntimeActivationCacheStats {
            entries: entries.len(),
            estimated_size_bytes: entries
                .values()
                .map(|entry| entry.estimated_size_bytes)
                .sum(),
        }
    }

    pub fn remove(&self, identity: &str) -> Option<RemovedRuntimeActivationCacheEntry> {
        self.entries
            .write()
            .expect("runtime activation cache lock poisoned")
            .remove(identity)
            .map(|entry| RemovedRuntimeActivationCacheEntry {
                estimated_size_bytes: entry.estimated_size_bytes,
            })
    }

    pub fn oldest_candidate(&self) -> Option<RuntimeActivationCacheEvictionCandidate> {
        let entries = self
            .entries
            .read()
            .expect("runtime activation cache lock poisoned");
        entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(
                |(identity, entry)| RuntimeActivationCacheEvictionCandidate {
                    identity: identity.clone(),
                    last_used: entry.last_used,
                },
            )
    }

    fn insert_arc_with_estimate(
        &self,
        identity: RuntimeProgramIdentity,
        activation: Arc<RuntimeActivation>,
        estimated_size_bytes: usize,
    ) -> Arc<RuntimeActivationCacheEntry> {
        let dynamic_build_id = identity.dynamic_build_id.clone();
        let mut entries = self
            .entries
            .write()
            .expect("runtime activation cache lock poisoned");
        if let Some(existing) = entries.get_mut(&dynamic_build_id) {
            return existing.touch();
        }
        let entry = Arc::new(RuntimeActivationCacheEntry {
            identity,
            activation,
        });
        entries.insert(
            dynamic_build_id,
            CacheEntry::new(Arc::clone(&entry), estimated_size_bytes),
        );
        entry
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeActivationCacheEntry {
    identity: RuntimeProgramIdentity,
    activation: Arc<RuntimeActivation>,
}

impl RuntimeActivationCacheEntry {
    pub fn identity(&self) -> &RuntimeProgramIdentity {
        &self.identity
    }

    pub fn dynamic_build_id(&self) -> &str {
        &self.identity.dynamic_build_id
    }

    pub fn linked_image_identity(&self) -> &str {
        &self.identity.linked_image_identity
    }

    pub fn activation(&self) -> Arc<RuntimeActivation> {
        Arc::clone(&self.activation)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeActivationCacheStats {
    pub entries: usize,
    pub estimated_size_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct RuntimeActivationCacheEvictionCandidate {
    pub identity: String,
    pub last_used: Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct RemovedRuntimeActivationCacheEntry {
    pub estimated_size_bytes: usize,
}

#[derive(Debug)]
struct CacheEntry<T> {
    value: Arc<T>,
    last_used: Instant,
    estimated_size_bytes: usize,
}

impl<T> CacheEntry<T> {
    fn new(value: Arc<T>, estimated_size_bytes: usize) -> Self {
        Self {
            value,
            last_used: Instant::now(),
            estimated_size_bytes,
        }
    }

    fn touch(&mut self) -> Arc<T> {
        self.last_used = Instant::now();
        Arc::clone(&self.value)
    }
}

fn runtime_activation_cache_entry_estimated_size(
    identity: &RuntimeProgramIdentity,
    activation: &RuntimeActivation,
) -> usize {
    mem::size_of::<RuntimeActivationCacheEntry>()
        .saturating_add(identity.dynamic_build_id.len())
        .saturating_add(identity.linked_image_identity.len())
        .saturating_add(mem::size_of::<Arc<RuntimeActivation>>())
        .saturating_add(runtime_activation_estimated_size(activation))
}

fn runtime_activation_estimated_size(activation: &RuntimeActivation) -> usize {
    mem::size_of::<RuntimeActivation>()
        .saturating_add(activation.service.id.len())
        .saturating_add(
            activation
                .service
                .display_name
                .as_deref()
                .map(str::len)
                .unwrap_or(0),
        )
        .saturating_add(serialized_estimated_size(&activation.service.metadata))
        .saturating_add(activation.version.len())
        .saturating_add(serialized_estimated_size(&activation.package_configs))
        .saturating_add(serialized_estimated_size(&activation.service_dependencies))
        .saturating_add(serialized_estimated_size(&activation.db))
        .saturating_add(serialized_estimated_size(&activation.actors))
        .saturating_add(serialized_estimated_size(&activation.gateway))
}

fn serialized_estimated_size<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| mem::size_of_val(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_linked_program::{GatewayConfig, ServiceMeta, ServiceTimeoutConfig};

    #[test]
    fn activation_cache_reuses_same_activation_arc_for_same_dynamic_build_id() {
        let cache = RuntimeActivationCache::new();
        let first_activation = Arc::new(runtime_activation("v1"));
        let replacement_activation = Arc::new(runtime_activation("v2"));

        let first = cache.insert_arc(
            RuntimeProgramIdentity::new("build:shared", "image:first"),
            Arc::clone(&first_activation),
        );
        let second = cache.insert_arc(
            RuntimeProgramIdentity::new("build:shared", "image:replacement"),
            replacement_activation,
        );
        let fetched = cache
            .get_by_dynamic_build_id("build:shared")
            .expect("expected cached runtime activation entry");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &fetched));
        assert!(Arc::ptr_eq(&first.activation(), &first_activation));
        assert_eq!(first.dynamic_build_id(), "build:shared");
        assert_eq!(first.linked_image_identity(), "image:first");
        assert_eq!(cache.len(), 1);
    }

    fn runtime_activation(version: &str) -> RuntimeActivation {
        RuntimeActivation {
            service: ServiceMeta {
                id: "svc".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: version.to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: ServiceTimeoutConfig::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        }
    }
}
