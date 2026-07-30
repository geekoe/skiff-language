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
                        error = %error,
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
        resolver: &skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver,
        service_db: &skiff_artifact_model::AssemblyActivationServiceDb,
    ) -> Result<()> {
        self.assembly_admission
            .discard_transient_for_reconnect()
            .map_err(|error| {
                RuntimeError::invalid_artifact(format!(
                    "committed recovery staging reset failed: {error}"
                ))
            })?;
        let state = resolver
            .store()
            .read_environment_activation(&self.environment)
            .map_err(|error| {
                RuntimeError::invalid_artifact(format!(
                    "committed activation recovery failed: {error}"
                ))
            })?;
        self.assembly_admission
            .recover_committed(
                &state.environment,
                state.committed.generation,
                &state.committed.assembly,
                resolver,
                Some(service_db),
            )
            .await
            .map_err(|error| {
                RuntimeError::invalid_artifact(format!(
                    "committed assembly admission failed: {error}"
                ))
            })?;
        Ok(())
    }
}
