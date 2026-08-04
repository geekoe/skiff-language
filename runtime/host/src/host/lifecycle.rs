use std::time::Duration;

use tokio::time::sleep;
use tracing::warn;

use crate::error::{Result, RuntimeError};

use super::{router_session, RuntimeHost};

impl RuntimeHost {
    pub async fn run_forever(self) -> Result<()> {
        self.run_reconnect_loop().await
    }

    async fn run_reconnect_loop(self) -> Result<()> {
        let mut backoff = Duration::from_millis(250);
        loop {
            match self.run_router_session_once().await {
                Ok(()) => {
                    backoff = Duration::from_millis(250);
                    warn!(
                        event = "runtime.router_disconnected",
                        reconnect_in_ms = backoff.as_millis() as u64
                    );
                }
                Err(error) => {
                    warn!(
                        event = "runtime.router_connection_error",
                        error = %format_args!("{error:#}"),
                        reconnect_in_ms = backoff.as_millis() as u64
                    );
                }
            }
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }

    async fn run_router_session_once(&self) -> Result<()> {
        router_session::run_once(self.clone()).await
    }

    pub(super) async fn recover_durable_committed(
        &self,
        profile: &str,
        generation: u64,
        assembly: &skiff_artifact_model::RuntimeAssemblyRef,
        config_snapshot: &skiff_artifact_model::RuntimeConfigSnapshotRef,
        resolver: &skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver,
        config_snapshot_resolver: &skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore,
        service_db: &skiff_artifact_model::AssemblyActivationServiceDb,
    ) -> Result<()> {
        self.freeze_bootstrap_profile(profile).map_err(|error| {
            RuntimeError::invalid_artifact(format!(
                "router bootstrap activation profile check failed: {error:#}"
            ))
        })?;
        self.assembly_admission
            .discard_transient_for_reconnect()
            .map_err(|error| {
                RuntimeError::invalid_artifact(format!(
                    "committed recovery staging reset failed: {error}"
                ))
            })?;
        self.assembly_admission
            .recover_committed(
                profile,
                generation,
                assembly,
                config_snapshot,
                resolver,
                config_snapshot_resolver,
                Some(service_db),
            )
            .await
            .map_err(|error| {
                RuntimeError::invalid_artifact(format!(
                    "committed assembly admission failed: {error:#}"
                ))
            })?;
        Ok(())
    }
}
