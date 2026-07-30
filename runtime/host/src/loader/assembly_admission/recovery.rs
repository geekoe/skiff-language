use super::*;

impl AssemblyAdmissionController {
    /// Rebuilds exactly the durable committed tuple after connection bootstrap.
    ///
    /// Pending activation data is validated by the canonical store reader but is not
    /// activated here. Any staged heap state from the previous session is discarded.
    pub(crate) async fn recover_committed<R, C>(
        &self,
        environment: &str,
        generation: u64,
        assembly: &RuntimeAssemblyRef,
        config_snapshot: &RuntimeConfigSnapshotRef,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
    ) -> anyhow::Result<Arc<ActiveAssembly>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let _reload = self.reload.lock().await;
        self.discard_transient_for_reconnect()?;
        skiff_artifact_model::validate_activation_environment(environment)
            .map_err(anyhow::Error::msg)?;
        skiff_artifact_model::validate_activation_generation(generation, "committed.generation")
            .map_err(anyhow::Error::msg)?;
        skiff_artifact_model::validate_runtime_assembly_ref(assembly)
            .map_err(anyhow::Error::msg)?;
        skiff_artifact_model::validate_runtime_config_snapshot_ref(config_snapshot)
            .map_err(anyhow::Error::msg)?;

        let committed = CommittedAssembly {
            environment: environment.to_string(),
            generation,
            assembly: assembly.clone(),
            config_snapshot: config_snapshot.clone(),
        };
        self.validate_recovery_transition(&committed)?;

        self.begin_recovery_candidate(generation, assembly.assembly_identity.clone())?;
        let prepared = self
            .resolve_started_exact_candidate(
                generation,
                assembly,
                config_snapshot,
                resolver,
                config_snapshot_resolver,
                "committed RuntimeAssembly recovery resolution failed",
                service_db,
                environment,
            )
            .await
            .map_err(|(_, error)| error)?;
        self.publish_recovered_committed(prepared, committed)
    }

    pub(crate) fn discard_transient_for_reconnect(&self) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        state.staged = None;
        state.candidate = None;
        Ok(())
    }

    fn validate_recovery_transition(&self, durable: &CommittedAssembly) -> anyhow::Result<()> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        if let Some(current) = &state.committed {
            if current.environment != durable.environment {
                anyhow::bail!("durable committed environment changed across reconnect");
            }
            if durable.generation < current.generation {
                anyhow::bail!("durable committed generation rolled back across reconnect");
            }
            if durable.generation == current.generation && current.assembly != durable.assembly {
                anyhow::bail!("durable committed assembly changed without generation advance");
            }
            if durable.generation == current.generation
                && current.config_snapshot != durable.config_snapshot
            {
                anyhow::bail!(
                    "durable committed config snapshot changed without generation advance"
                );
            }
        }
        Ok(())
    }

    fn publish_recovered_committed(
        &self,
        prepared: PreparedAssembly,
        committed: CommittedAssembly,
    ) -> anyhow::Result<Arc<ActiveAssembly>> {
        let identity = committed.assembly.assembly_identity.clone();
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        ensure_current_candidate(&state, committed.generation, &identity)?;
        publish_committed_locked(&mut state, prepared, committed)
    }
}
