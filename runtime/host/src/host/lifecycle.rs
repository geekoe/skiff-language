use std::{collections::HashSet, env, time::Duration};

use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

use crate::error::{Result, RuntimeError};

use super::{router_session, ReleaseIdleBuildsReport, RuntimeHost, RuntimeMemoryMaintenanceReport};

const DEFAULT_RUNTIME_MEMORY_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_IDLE_BUILD_RELEASE_AFTER: Duration = Duration::from_secs(15 * 60);
const RUNTIME_MEMORY_MAINTENANCE_INTERVAL_ENV: &str =
    "SKIFF_RUNTIME_MEMORY_MAINTENANCE_INTERVAL_SECONDS";
const IDLE_BUILD_RELEASE_AFTER_ENV: &str = "SKIFF_RUNTIME_IDLE_BUILD_RELEASE_SECONDS";

impl RuntimeHost {
    pub async fn run_forever(self) -> Result<()> {
        let maintenance_host = self.clone();
        tokio::select! {
            result = self.run_reconnect_loop() => result,
            result = maintenance_host.run_memory_maintenance_loop() => result,
        }
    }

    async fn run_reconnect_loop(self) -> Result<()> {
        router_session::run_reconnect_loop(self).await
    }

    async fn run_memory_maintenance_loop(self) -> Result<()> {
        let idle_for = idle_build_release_after();
        let mut interval = tokio::time::interval(runtime_memory_maintenance_interval());
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = self.run_memory_maintenance_once(idle_for).await {
                warn!(
                    event = "runtime.memory_maintenance_error",
                    error = %error
                );
            }
        }
    }

    pub(crate) async fn release_idle_builds(
        &self,
        idle_for: Duration,
    ) -> Result<ReleaseIdleBuildsReport> {
        let (candidates, skipped_active_builds) = self.loaded_builds.mark_idle_releasing(idle_for);
        if candidates.is_empty() {
            return Ok(ReleaseIdleBuildsReport {
                released_builds: Vec::new(),
                skipped_active_builds,
                stopped_spawn_workers: 0,
            });
        }

        let stopped_spawn_workers = self.spawn_workers.stop_builds(&candidates).await;
        let candidate_set = candidates.into_iter().collect::<HashSet<_>>();
        let released_builds = self.loaded_builds.remove_releasing_builds(&candidate_set);
        let released_set = released_builds.iter().cloned().collect::<HashSet<_>>();
        if !released_set.is_empty() {
            let mut state = self.state.write().map_err(|_| {
                RuntimeError::Decode("runtime service route state lock is poisoned".to_string())
            })?;
            let removed_services = state.remove_builds(&released_set);
            info!(
                event = "runtime.idle_builds_released",
                build_count = released_set.len(),
                service_count = removed_services,
                stopped_spawn_workers
            );
        }

        Ok(ReleaseIdleBuildsReport {
            released_builds,
            skipped_active_builds,
            stopped_spawn_workers,
        })
    }

    pub(crate) async fn run_memory_maintenance_once(
        &self,
        idle_for: Duration,
    ) -> Result<RuntimeMemoryMaintenanceReport> {
        let artifact_eviction = self.artifact_caches.evict_lru_to_budget();
        if !artifact_eviction.entries.is_empty() {
            info!(
                event = "runtime.artifact_cache_evicted",
                entry_count = artifact_eviction.entries.len(),
                estimated_bytes = artifact_eviction.estimated_bytes,
                remaining_estimated_bytes = artifact_eviction.remaining_estimated_size_bytes
            );
        }
        let idle_builds = self.release_idle_builds(idle_for).await?;
        Ok(RuntimeMemoryMaintenanceReport {
            idle_builds,
            artifact_cache_evicted_entries: artifact_eviction.entries.len(),
            artifact_cache_evicted_bytes: artifact_eviction.estimated_bytes,
            artifact_cache_remaining_bytes: artifact_eviction.remaining_estimated_size_bytes,
        })
    }
}

fn runtime_memory_maintenance_interval() -> Duration {
    duration_from_env_seconds(
        RUNTIME_MEMORY_MAINTENANCE_INTERVAL_ENV,
        DEFAULT_RUNTIME_MEMORY_MAINTENANCE_INTERVAL,
    )
}

fn idle_build_release_after() -> Duration {
    duration_from_env_seconds(
        IDLE_BUILD_RELEASE_AFTER_ENV,
        DEFAULT_IDLE_BUILD_RELEASE_AFTER,
    )
}

fn duration_from_env_seconds(name: &str, default: Duration) -> Duration {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or(default)
}
