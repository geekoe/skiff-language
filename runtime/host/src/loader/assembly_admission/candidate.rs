use super::*;

impl AssemblyAdmissionController {
    pub(super) fn begin_online_candidate(
        &self,
        generation: u64,
        identity: AssemblyIdentity,
    ) -> anyhow::Result<()> {
        if generation == 0 {
            anyhow::bail!("assembly candidate generation must be greater than zero");
        }
        self.begin_exact_candidate(generation, identity)
    }

    pub(super) fn begin_recovery_candidate(
        &self,
        generation: u64,
        identity: AssemblyIdentity,
    ) -> anyhow::Result<()> {
        skiff_artifact_model::validate_activation_generation(generation, "committed.generation")
            .map_err(anyhow::Error::msg)?;
        self.begin_exact_candidate(generation, identity)
    }

    fn begin_exact_candidate(
        &self,
        generation: u64,
        identity: AssemblyIdentity,
    ) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        if let Some(candidate) = &state.candidate {
            anyhow::bail!(
                "assembly admission generation {} is already building",
                candidate.generation
            );
        }
        state.next_generation = state.next_generation.max(generation);
        state.candidate = Some(AssemblyCandidateHealth {
            generation,
            identity,
            stage: AssemblyCandidateStage::Load,
            started_at: OffsetDateTime::now_utc(),
        });
        Ok(())
    }

    pub(super) async fn resolve_started_exact_candidate<R, C>(
        &self,
        generation: u64,
        reference: &RuntimeAssemblyRef,
        config_snapshot_reference: &RuntimeConfigSnapshotRef,
        resolver: &R,
        config_snapshot_resolver: &C,
        resolution_context: &'static str,
        service_db: Option<&AssemblyActivationServiceDb>,
        environment: &str,
    ) -> Result<PreparedAssembly, (AssemblyActivationRejectReason, anyhow::Error)>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + ?Sized,
    {
        let identity = reference.assembly_identity.clone();
        let assembly = resolver
            .resolve_runtime_assembly(reference)
            .map_err(|error| {
                let _ = self.fail_candidate(generation, &identity, AssemblyCandidateStage::Load);
                (
                    AssemblyActivationRejectReason::Resolve,
                    error.context(resolution_context),
                )
            })?;
        match skiff_artifact_identity::runtime_assembly_ref(&assembly) {
            Ok(resolved) if &resolved == reference => {}
            Ok(_) => {
                let _ = self.fail_candidate(generation, &identity, AssemblyCandidateStage::Load);
                return Err((
                    AssemblyActivationRejectReason::Resolve,
                    anyhow::anyhow!("resolved RuntimeAssembly content mismatches exact ref"),
                ));
            }
            Err(error) => {
                let _ = self.fail_candidate(generation, &identity, AssemblyCandidateStage::Load);
                return Err((AssemblyActivationRejectReason::Resolve, error.into()));
            }
        }
        let config_snapshot = config_snapshot_resolver
            .resolve(config_snapshot_reference)
            .map_err(|_| {
                let _ = self.fail_candidate(generation, &identity, AssemblyCandidateStage::Load);
                (
                    AssemblyActivationRejectReason::Resolve,
                    anyhow::anyhow!(
                        "RuntimeConfigSnapshot {} resolution failed",
                        config_snapshot_reference.snapshot_id
                    ),
                )
            })?;
        if config_snapshot.snapshot_ref() != config_snapshot_reference {
            let _ = self.fail_candidate(generation, &identity, AssemblyCandidateStage::Load);
            return Err((
                AssemblyActivationRejectReason::Resolve,
                anyhow::anyhow!(
                    "RuntimeConfigSnapshot {} rejected: resolved content does not match the requested opaque id",
                    config_snapshot_reference.snapshot_id
                ),
            ));
        }
        if let Err(error) = super::super::config_snapshot::validate_snapshot_environment(
            &config_snapshot,
            environment,
        ) {
            let _ = self.fail_candidate_config_snapshot_environment(
                generation,
                &identity,
                config_snapshot_reference,
            );
            return Err((AssemblyActivationRejectReason::Admission, error));
        }
        self.build_started_candidate(
            generation,
            &identity,
            assembly,
            resolver,
            service_db,
            Some(environment),
            Some(config_snapshot_reference),
            Some(&config_snapshot),
        )
        .await
        .map_err(|error| {
            let reason = self
                .health()
                .ok()
                .and_then(|health| health.last_outcome)
                .map(|outcome| reject_reason_for_stage(outcome.stage))
                .unwrap_or(AssemblyActivationRejectReason::Admission);
            (reason, error)
        })
    }
}
