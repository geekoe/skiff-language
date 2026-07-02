use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use skiff_runtime_transport::protocol::RouterControlServiceConfig;

use crate::{
    error::{Result, RuntimeError},
    loader::ArtifactLoadOptions,
};

use super::{ServiceOperationContext, ServiceRuntimeContext};

#[derive(Clone)]
pub(crate) struct ArtifactLoadState {
    pub(super) artifact_roots: Vec<PathBuf>,
    pub(super) load_options: ArtifactLoadOptions,
    pub(super) service_config: Vec<RouterControlServiceConfig>,
    pub(super) epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BuildOperationAbiRouteKey {
    pub(super) build_id: String,
    pub(super) operation_abi_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BuildSelectorRouteKey {
    pub(super) build_id: String,
    pub(super) selector: String,
}

#[derive(Clone, Default)]
pub(crate) struct ServiceRouteState {
    pub(super) services: Arc<Vec<Arc<ServiceRuntimeContext>>>,
    pub(super) route_by_build_and_operation_abi_id:
        Arc<HashMap<BuildOperationAbiRouteKey, Vec<ServiceOperationContext>>>,
    pub(super) operation_abi_id_by_build_and_selector: Arc<HashMap<BuildSelectorRouteKey, String>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ReleaseIdleBuildsReport {
    pub(super) released_builds: Vec<String>,
    pub(super) skipped_active_builds: Vec<String>,
    pub(super) stopped_spawn_workers: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RuntimeMemoryMaintenanceReport {
    pub(super) idle_builds: ReleaseIdleBuildsReport,
    pub(super) artifact_cache_evicted_entries: usize,
    pub(super) artifact_cache_evicted_bytes: usize,
    pub(super) artifact_cache_remaining_bytes: usize,
}

#[derive(Debug, Clone)]
struct LoadedBuildLifecycle {
    last_used: Instant,
    active_executions: usize,
    releasing: bool,
}

#[derive(Debug, Default)]
pub(crate) struct LoadedBuildRegistry {
    builds: StdMutex<HashMap<String, LoadedBuildLifecycle>>,
}

pub(crate) struct BuildExecutionGuard {
    registry: Option<Arc<LoadedBuildRegistry>>,
    build_id: String,
}

impl ServiceRouteState {
    pub(super) fn build_ids(&self) -> HashSet<String> {
        self.services
            .iter()
            .map(|service| service.build_id.clone())
            .collect()
    }

    pub(super) fn remove_builds(&mut self, build_ids: &HashSet<String>) -> usize {
        if build_ids.is_empty() {
            return 0;
        }
        let mut services = self.services.iter().cloned().collect::<Vec<_>>();
        let before = services.len();
        services.retain(|service| !build_ids.contains(&service.build_id));

        let mut operation_routes = (*self.route_by_build_and_operation_abi_id).clone();
        operation_routes.retain(|route_key, _| !build_ids.contains(&route_key.build_id));
        let mut selector_routes = (*self.operation_abi_id_by_build_and_selector).clone();
        selector_routes.retain(|route_key, _| !build_ids.contains(&route_key.build_id));

        self.services = Arc::new(services);
        self.route_by_build_and_operation_abi_id = Arc::new(operation_routes);
        self.operation_abi_id_by_build_and_selector = Arc::new(selector_routes);
        before.saturating_sub(self.services.len())
    }
}

impl LoadedBuildRegistry {
    pub(super) fn from_build_ids(build_ids: impl IntoIterator<Item = String>) -> Self {
        let registry = Self::default();
        registry.replace_builds(build_ids);
        registry
    }

    pub(super) fn replace_builds(&self, build_ids: impl IntoIterator<Item = String>) {
        let now = Instant::now();
        let mut builds = self
            .builds
            .lock()
            .expect("loaded build registry lock poisoned");
        builds.clear();
        for build_id in build_ids {
            builds.insert(
                build_id,
                LoadedBuildLifecycle {
                    last_used: now,
                    active_executions: 0,
                    releasing: false,
                },
            );
        }
    }

    pub(super) fn upsert_builds(&self, build_ids: impl IntoIterator<Item = String>) {
        let now = Instant::now();
        let mut builds = self
            .builds
            .lock()
            .expect("loaded build registry lock poisoned");
        for build_id in build_ids {
            builds
                .entry(build_id)
                .and_modify(|state| {
                    state.last_used = now;
                    state.releasing = false;
                })
                .or_insert(LoadedBuildLifecycle {
                    last_used: now,
                    active_executions: 0,
                    releasing: false,
                });
        }
    }

    pub(super) fn touch(&self, build_id: &str) {
        let now = Instant::now();
        if let Ok(mut builds) = self.builds.lock() {
            if let Some(state) = builds.get_mut(build_id) {
                state.last_used = now;
            }
        }
    }

    pub(super) fn begin_execution(self: &Arc<Self>, build_id: &str) -> Result<BuildExecutionGuard> {
        let mut builds = self.builds.lock().map_err(|_| {
            RuntimeError::Decode("loaded build registry lock is poisoned".to_string())
        })?;
        let Some(state) = builds.get_mut(build_id) else {
            return Ok(BuildExecutionGuard {
                registry: None,
                build_id: build_id.to_string(),
            });
        };
        state.last_used = Instant::now();
        state.active_executions += 1;
        Ok(BuildExecutionGuard {
            registry: Some(self.clone()),
            build_id: build_id.to_string(),
        })
    }

    pub(super) fn mark_idle_releasing(&self, idle_for: Duration) -> (Vec<String>, Vec<String>) {
        let now = Instant::now();
        let mut releasable = Vec::new();
        let mut skipped_active = Vec::new();
        let mut builds = self
            .builds
            .lock()
            .expect("loaded build registry lock poisoned");

        for (build_id, state) in builds.iter_mut() {
            if now.duration_since(state.last_used) < idle_for || state.releasing {
                continue;
            }
            if state.active_executions == 0 {
                state.releasing = true;
                releasable.push(build_id.clone());
            } else {
                skipped_active.push(build_id.clone());
            }
        }
        releasable.sort();
        skipped_active.sort();
        (releasable, skipped_active)
    }

    pub(super) fn remove_releasing_builds(&self, build_ids: &HashSet<String>) -> Vec<String> {
        let mut removed = Vec::new();
        let mut builds = self
            .builds
            .lock()
            .expect("loaded build registry lock poisoned");
        for build_id in build_ids {
            let should_remove = builds
                .get(build_id)
                .is_some_and(|state| state.releasing && state.active_executions == 0);
            if should_remove {
                builds.remove(build_id);
                removed.push(build_id.clone());
            } else if let Some(state) = builds.get_mut(build_id) {
                state.releasing = false;
            }
        }
        removed.sort();
        removed
    }

    fn finish_execution(&self, build_id: &str) {
        if let Ok(mut builds) = self.builds.lock() {
            if let Some(state) = builds.get_mut(build_id) {
                state.active_executions = state.active_executions.saturating_sub(1);
                state.last_used = Instant::now();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn active_count(&self, build_id: &str) -> usize {
        self.builds
            .lock()
            .expect("loaded build registry lock poisoned")
            .get(build_id)
            .map(|state| state.active_executions)
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(super) fn force_last_used_for_test(&self, build_id: &str, last_used: Instant) {
        let mut builds = self
            .builds
            .lock()
            .expect("loaded build registry lock poisoned");
        if let Some(state) = builds.get_mut(build_id) {
            state.last_used = last_used;
        }
    }

    #[cfg(test)]
    pub(super) fn contains(&self, build_id: &str) -> bool {
        self.builds
            .lock()
            .expect("loaded build registry lock poisoned")
            .contains_key(build_id)
    }
}

impl Drop for BuildExecutionGuard {
    fn drop(&mut self) {
        if let Some(registry) = &self.registry {
            registry.finish_execution(&self.build_id);
        }
    }
}
