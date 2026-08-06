use super::*;

impl AssemblyAdmissionController {
    /// Applies one exact router -> runtime assembly transition.
    ///
    /// Prepare only constructs staged immutable state. Commit is the sole
    /// active-pointer publication point, while abort drops the complete staged
    /// candidate under the same serialized transition permit.
    pub(crate) async fn apply_activation_control<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
        bootstrap_service_db: Option<&AssemblyActivationServiceDb>,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        self.apply_activation_control_inner(
            control,
            resolver,
            config_snapshot_resolver,
            bootstrap_service_db,
            None,
        )
        .await
    }

    pub(crate) async fn apply_cancellable_activation_control<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
        bootstrap_service_db: Option<&AssemblyActivationServiceDb>,
        cancellation: &CancellationToken,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        self.apply_activation_control_inner(
            control,
            resolver,
            config_snapshot_resolver,
            bootstrap_service_db,
            Some(cancellation),
        )
        .await
    }

    async fn apply_activation_control_inner<R, C>(
        &self,
        control: AssemblyActivationControl,
        resolver: &R,
        config_snapshot_resolver: &C,
        bootstrap_service_db: Option<&AssemblyActivationServiceDb>,
        cancellation: Option<&CancellationToken>,
    ) -> anyhow::Result<Option<AssemblyActivationControl>>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let _ = (resolver, config_snapshot_resolver, bootstrap_service_db, cancellation);
        control.validate().map_err(anyhow::Error::msg)?;
        match control {
            AssemblyActivationControl::Prepare {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            } => {
                if service_db.is_some() {
                    anyhow::bail!(
                        "assembly activation serviceDb is not supported; use connection bootstrap"
                    );
                }
                self.ensure_replica(&replica_id)?;
                let transition = AssemblyTransition {
                    profile,
                    activation_id,
                    expected_generation,
                    candidate_generation,
                    assembly,
                    config_snapshot,
                };
                // M2 lazy-load deployment: the activation coordination layer is
                // preserved as wire compatibility only. Prepare acknowledges
                // without any loading or materialization; the per-buildId lazy
                // load path is the only runtime materialization mechanism.
                Ok(Some(transition.prepared_control(replica_id)))
            }
            AssemblyActivationControl::Commit {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            } => {
                if service_db.is_some() {
                    anyhow::bail!(
                        "assembly activation serviceDb is not supported; use connection bootstrap"
                    );
                }
                self.ensure_replica(&replica_id)?;
                let transition = AssemblyTransition {
                    profile,
                    activation_id,
                    expected_generation,
                    candidate_generation,
                    assembly,
                    config_snapshot,
                };
                // M2: Commit records the committed tuple metadata (feeding the
                // Register frame and health) without loading anything. Requests
                // for these build ids materialize through the lazy-load path.
                self.record_committed_transition(&transition)?;
                Ok(Some(transition.register_control(replica_id)))
            }
            AssemblyActivationControl::Abort {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            } => {
                self.ensure_replica(&replica_id)?;
                self.abort_transition(&AssemblyTransition {
                    profile,
                    activation_id,
                    expected_generation,
                    candidate_generation,
                    assembly,
                    config_snapshot,
                })
                .await?;
                Ok(None)
            }
            AssemblyActivationControl::Prepared { .. }
            | AssemblyActivationControl::Reject { .. }
            | AssemblyActivationControl::Register { .. } => {
                anyhow::bail!("runtime received a router-invalid assembly activation reply")
            }
        }
    }

    /// Records the router-coordinated committed tuple metadata without any
    /// loading or materialization. The loaded registry and the lazy-load path
    /// remain the only runtime materialization mechanism.
    fn record_committed_transition(&self, transition: &AssemblyTransition) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        if let Some(committed) = &state.committed {
            if committed.profile != transition.profile {
                anyhow::bail!(
                    "durable committed profile changed without bootstrap recovery"
                );
            }
        }
        state.next_generation = state.next_generation.max(transition.candidate_generation);
        state.committed = Some(CommittedAssembly {
            profile: transition.profile.clone(),
            generation: transition.candidate_generation,
            assembly: transition.assembly.clone(),
            config_snapshot: transition.config_snapshot.clone(),
        });
        state.staged = None;
        state.preparing = None;
        state.candidate = None;
        state.last_outcome = None;
        Ok(())
    }

    pub(crate) fn registration(&self) -> anyhow::Result<Option<AssemblyActivationControl>> {
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        Ok(state
            .committed
            .as_ref()
            .map(|committed| AssemblyActivationControl::Register {
                profile: committed.profile.clone(),
                generation: committed.generation,
                assembly: committed.assembly.clone(),
                config_snapshot: committed.config_snapshot.clone(),
                replica_id: self.runtime_replica_id.clone(),
            }))
    }

    async fn abort_transition(&self, transition: &AssemblyTransition) -> anyhow::Result<()> {
        let _reload = self.reload.lock().await;
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        match state.staged.as_ref() {
            Some(staged) if staged.transition.same_tuple(transition) => {
                state.staged = None;
                state.last_outcome = None;
                info!(
                    event = "runtime.assembly_staging_aborted",
                    assembly_identity = %transition.assembly.assembly_identity,
                    generation = transition.candidate_generation
                );
                Ok(())
            }
            Some(_) => anyhow::bail!("abort tuple does not match the staged assembly activation"),
            None => match state.preparing.as_ref() {
                Some(preparing) if preparing.same_tuple(transition) => {
                    state.preparing = None;
                    state.candidate = None;
                    state.last_outcome = None;
                    info!(
                        event = "runtime.assembly_preparing_aborted",
                        assembly_identity = %transition.assembly.assembly_identity,
                        generation = transition.candidate_generation
                    );
                    Ok(())
                }
                Some(_) => {
                    anyhow::bail!("abort tuple does not match the preparing assembly activation")
                }
                None => Ok(()),
            },
        }
    }

    fn ensure_replica(&self, replica_id: &str) -> anyhow::Result<()> {
        if replica_id != self.runtime_replica_id {
            anyhow::bail!(
                "assembly activation replicaId {replica_id} does not match runtime replica {}",
                self.runtime_replica_id
            );
        }
        Ok(())
    }
}

impl AssemblyTransition {
    fn same_tuple(&self, other: &Self) -> bool {
        self.profile == other.profile
            && self.activation_id == other.activation_id
            && self.expected_generation == other.expected_generation
            && self.candidate_generation == other.candidate_generation
            && self.assembly == other.assembly
            && self.config_snapshot == other.config_snapshot
    }

    fn prepared_control(&self, replica_id: String) -> AssemblyActivationControl {
        AssemblyActivationControl::Prepared {
            profile: self.profile.clone(),
            activation_id: self.activation_id.clone(),
            expected_generation: self.expected_generation,
            candidate_generation: self.candidate_generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
            replica_id,
        }
    }

    fn reject_control(
        &self,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    ) -> AssemblyActivationControl {
        AssemblyActivationControl::Reject {
            profile: self.profile.clone(),
            activation_id: self.activation_id.clone(),
            expected_generation: self.expected_generation,
            candidate_generation: self.candidate_generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
            replica_id,
            reason,
        }
    }

    fn register_control(&self, replica_id: String) -> AssemblyActivationControl {
        AssemblyActivationControl::Register {
            profile: self.profile.clone(),
            generation: self.candidate_generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
            replica_id,
        }
    }
}

