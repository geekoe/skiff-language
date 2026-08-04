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
                let reply = match self
                    .prepare_transition(
                        &transition,
                        resolver,
                        config_snapshot_resolver,
                        bootstrap_service_db,
                        cancellation,
                    )
                    .await
                {
                    Ok(PrepareTransitionOutcome::Prepared) => {
                        Some(transition.prepared_control(replica_id))
                    }
                    Ok(PrepareTransitionOutcome::Cancelled) => None,
                    Err((reason, error)) => {
                        warn!(
                            event = "runtime.assembly_prepare_rejected",
                            assembly_identity = %transition.assembly.assembly_identity,
                            generation = transition.candidate_generation,
                            reason = ?reason,
                            error = %format_args!("{error:#}")
                        );
                        Some(transition.reject_control(replica_id, reason))
                    }
                };
                Ok(reply)
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
                let reply = match self
                    .commit_transition(
                        &transition,
                        resolver,
                        config_snapshot_resolver,
                        bootstrap_service_db,
                    )
                    .await
                {
                    Ok(()) => transition.register_control(replica_id),
                    Err((reason, error)) => {
                        warn!(
                            event = "runtime.assembly_commit_rejected",
                            assembly_identity = %transition.assembly.assembly_identity,
                            generation = transition.candidate_generation,
                            reason = ?reason,
                            error = %format_args!("{error:#}")
                        );
                        transition.reject_control(replica_id, reason)
                    }
                };
                Ok(Some(reply))
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

    async fn prepare_transition<R, C>(
        &self,
        transition: &AssemblyTransition,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PrepareTransitionOutcome, (AssemblyActivationRejectReason, anyhow::Error)>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let _reload = self.reload.lock().await;
        {
            let state = self
                .state
                .read()
                .map_err(|_| admission_reject("assembly admission state lock is poisoned"))?;
            if state
                .staged
                .as_ref()
                .is_some_and(|staged| staged.transition == *transition)
                || committed_matches_transition(state.committed.as_ref(), transition)
            {
                return Ok(PrepareTransitionOutcome::Prepared);
            }
            if state.staged.is_some() {
                return Err(admission_reject(
                    "a different assembly activation is already staged",
                ));
            }
            if let Some(committed) = &state.committed {
                if committed.profile != transition.profile
                    || committed.generation != transition.expected_generation
                {
                    return Err(admission_reject(
                        "prepare expected generation does not match active committed assembly",
                    ));
                }
            } else if transition.expected_generation != 0 {
                return Err(admission_reject(
                    "runtime must recover the committed assembly before preparing its successor",
                ));
            }
        }

        let identity = transition.assembly.assembly_identity.clone();
        self.begin_online_transition(transition)
            .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
        info!(
            event = "runtime.assembly_candidate_started",
            assembly_identity = %identity,
            generation = transition.candidate_generation
        );
        let prepare = self.resolve_started_exact_candidate(
            transition.candidate_generation,
            &transition.assembly,
            &transition.config_snapshot,
            resolver,
            config_snapshot_resolver,
            "exact RuntimeAssembly record resolution failed",
            service_db,
            &transition.profile,
        );
        let prepared = if let Some(cancellation) = cancellation {
            tokio::select! {
                () = cancellation.wait_cancelled() => {
                    self.cancel_preparing_transition(transition)
                        .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
                    return Ok(PrepareTransitionOutcome::Cancelled);
                }
                result = prepare => result,
            }
        } else {
            prepare.await
        };
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            self.cancel_preparing_transition(transition)
                .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
            return Ok(PrepareTransitionOutcome::Cancelled);
        }
        let prepared = prepared?;
        self.stage_prepared(transition.clone(), prepared)
            .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
        info!(
            event = "runtime.assembly_prepared",
            assembly_identity = %identity,
            generation = transition.candidate_generation
        );
        Ok(PrepareTransitionOutcome::Prepared)
    }

    async fn commit_transition<R, C>(
        &self,
        transition: &AssemblyTransition,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
    ) -> Result<(), (AssemblyActivationRejectReason, anyhow::Error)>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        let _reload = self.reload.lock().await;
        {
            let state = self
                .state
                .read()
                .map_err(|_| admission_reject("assembly admission state lock is poisoned"))?;
            if committed_matches_transition(state.committed.as_ref(), transition) {
                return Ok(());
            }
            if let Some(staged) = &state.staged {
                if staged.transition != *transition {
                    return Err(admission_reject(
                        "commit tuple does not match the staged assembly activation",
                    ));
                }
                drop(state);
                self.commit_staged(transition)
                    .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
                return Ok(());
            }
            if let Some(committed) = &state.committed {
                if committed.profile != transition.profile
                    || committed.generation != transition.expected_generation
                {
                    return Err(admission_reject(
                        "commit expected generation does not match active committed assembly",
                    ));
                }
            }
        }

        // A fresh process has no staged heap state. An exact commit replay is
        // the durable recovery signal, so rebuild it before registering.
        self.prepare_recovery_transition(
            transition,
            resolver,
            config_snapshot_resolver,
            service_db,
        )
        .await?;
        self.commit_staged(transition)
            .map(|_| ())
            .map_err(|error| (AssemblyActivationRejectReason::Admission, error))
    }

    async fn prepare_recovery_transition<R, C>(
        &self,
        transition: &AssemblyTransition,
        resolver: &R,
        config_snapshot_resolver: &C,
        service_db: Option<&AssemblyActivationServiceDb>,
    ) -> Result<(), (AssemblyActivationRejectReason, anyhow::Error)>
    where
        R: RuntimeAssemblyRecordResolver + Sync + ?Sized,
        C: skiff_runtime_config_snapshot::RuntimeConfigSnapshotResolver + Sync + ?Sized,
    {
        self.begin_online_transition(transition)
            .map_err(|error| (AssemblyActivationRejectReason::Admission, error))?;
        let prepared = self
            .resolve_started_exact_candidate(
                transition.candidate_generation,
                &transition.assembly,
                &transition.config_snapshot,
                resolver,
                config_snapshot_resolver,
                "committed RuntimeAssembly recovery resolution failed",
                service_db,
                &transition.profile,
            )
            .await?;
        self.stage_prepared(transition.clone(), prepared)
            .map_err(|error| (AssemblyActivationRejectReason::Admission, error))
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

    fn cancel_preparing_transition(&self, transition: &AssemblyTransition) -> anyhow::Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        match state.preparing.as_ref() {
            Some(preparing) if preparing.same_tuple(transition) => {}
            Some(_) => {
                anyhow::bail!("cancel tuple does not match the preparing assembly activation")
            }
            None if state.candidate.is_none() && state.staged.is_none() => {
                // The candidate may have failed at the same instant cancellation won the
                // session race. The serialized transition still owns this state, so erase the
                // superseded diagnostic outcome and converge to the same clean Abort result.
                state.last_outcome = None;
                return Ok(());
            }
            None => {
                anyhow::bail!("cancelled assembly activation has no exact preparing transition")
            }
        }
        state.preparing = None;
        state.candidate = None;
        state.last_outcome = None;
        info!(
            event = "runtime.assembly_prepare_cancelled",
            assembly_identity = %transition.assembly.assembly_identity,
            generation = transition.candidate_generation
        );
        Ok(())
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

    fn stage_prepared(
        &self,
        transition: AssemblyTransition,
        prepared: PreparedAssembly,
    ) -> anyhow::Result<()> {
        if prepared.generation != transition.candidate_generation {
            anyhow::bail!("prepared assembly generation does not match activation transition");
        }
        let identity = prepared.candidate.assembly().assembly_identity.clone();
        if identity != transition.assembly.assembly_identity {
            anyhow::bail!("prepared assembly identity does not match activation transition");
        }
        if prepared.config_snapshot != transition.config_snapshot {
            anyhow::bail!("prepared config snapshot does not match activation transition");
        }
        let observed_at = OffsetDateTime::now_utc();
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        ensure_current_candidate(&state, prepared.generation, &identity)?;
        if state.preparing.as_ref() != Some(&transition) {
            anyhow::bail!("prepared assembly does not match the preparing activation tuple");
        }
        if state.staged.is_some() {
            anyhow::bail!("an assembly activation is already staged");
        }
        state.candidate = None;
        state.preparing = None;
        state.last_outcome = Some(AssemblyAdmissionOutcome {
            generation: prepared.generation,
            identity,
            succeeded: true,
            stage: AssemblyCandidateStage::Admit,
            observed_at,
            error: None,
        });
        state.staged = Some(StagedAssembly {
            transition,
            prepared,
        });
        Ok(())
    }

    fn commit_staged(
        &self,
        transition: &AssemblyTransition,
    ) -> anyhow::Result<Arc<ActiveAssembly>> {
        let mut state = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("assembly admission state lock is poisoned"))?;
        let staged = state
            .staged
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("commit has no staged assembly activation"))?;
        if staged.transition != *transition {
            anyhow::bail!("commit tuple does not match staged assembly activation");
        }
        let staged = state
            .staged
            .take()
            .expect("staged assembly was checked above");
        let committed = CommittedAssembly {
            profile: transition.profile.clone(),
            generation: transition.candidate_generation,
            assembly: transition.assembly.clone(),
            config_snapshot: transition.config_snapshot.clone(),
        };
        let active = publish_committed_locked(&mut state, staged.prepared, committed)?;
        info!(
            event = "runtime.assembly_committed",
            assembly_identity = %transition.assembly.assembly_identity,
            generation = transition.candidate_generation
        );
        Ok(active)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepareTransitionOutcome {
    Prepared,
    Cancelled,
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

fn committed_matches_transition(
    committed: Option<&CommittedAssembly>,
    transition: &AssemblyTransition,
) -> bool {
    committed.is_some_and(|committed| {
        committed.profile == transition.profile
            && committed.generation == transition.candidate_generation
            && committed.assembly == transition.assembly
            && committed.config_snapshot == transition.config_snapshot
    })
}

fn admission_reject(message: impl Into<String>) -> (AssemblyActivationRejectReason, anyhow::Error) {
    (
        AssemblyActivationRejectReason::Admission,
        anyhow::anyhow!(message.into()),
    )
}
