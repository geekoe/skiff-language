//! Port-level sequence contract for `ActivationStateRepository` (frozen fake
//! seam, C-router-activation-state §10): idempotent replay, CAS conflicts,
//! single audit per effective mutation, audit-failure rollback, transient
//! retry without duplicate audit, initialize semantics, and missing-state
//! reads. The same sequence contract is exercised against the real Mongo
//! adapter in `activation_mongo_probe.rs`.

#[cfg(test)]
mod tests {

    use skiff_artifact_model::{
        AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    };
    use skiff_deployment::activation_state::{
        activation_audit_event_id, ActivationAuditEvent, ActivationAuditOperation,
        ActivationAuditOutcome, EnvironmentActivationState,
    };
    use skiff_router::activation::{
        memory::MemoryActivationStateRepository, repository::AbortInput, repository::CommitInput,
        repository::PrepareInput, ActivationStateRepository, RepositoryError,
    };

    fn assembly(byte: u8) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                char::from(b'a' + byte).to_string().repeat(64)
            )),
        }
    }

    fn config(byte: u8) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                "skiff-runtime-config-snapshot-v1:{}",
                char::from(b'a' + byte).to_string().repeat(32)
            ))
            .expect("config snapshot id"),
        }
    }

    fn initial_state() -> EnvironmentActivationState {
        EnvironmentActivationState::initial("test", 7, assembly(0), config(0))
    }

    fn prepare_input(activation_id: &str) -> PrepareInput {
        PrepareInput {
            environment: "test".to_string(),
            activation_id: activation_id.to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: assembly(1),
            config_snapshot: config(1),
            participant_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
        }
    }

    fn commit_input(prepared: &EnvironmentActivationState) -> CommitInput {
        let pending = prepared.pending.as_ref().expect("prepared pending");
        CommitInput {
            environment: "test".to_string(),
            activation_id: pending.activation_id.clone(),
            expected_generation: pending.expected_generation,
            candidate_generation: pending.candidate_generation,
            assembly: pending.assembly.clone(),
            config_snapshot: pending.config_snapshot.clone(),
            connected_replica_ids: pending.participant_replica_ids.clone(),
            prepared_replica_ids: pending.participant_replica_ids.clone(),
        }
    }

    fn abort_input(prepared: &EnvironmentActivationState) -> AbortInput {
        let pending = prepared.pending.as_ref().expect("prepared pending");
        AbortInput {
            environment: "test".to_string(),
            activation_id: pending.activation_id.clone(),
            expected_generation: pending.expected_generation,
        }
    }

    #[tokio::test]
    async fn full_lifecycle_sequence_writes_one_audit_per_effective_mutation() {
        let repository = MemoryActivationStateRepository::new();
        repository
            .initialize(&initial_state())
            .await
            .expect("initialize");

        let input = prepare_input("activation-8");
        let prepared = repository.prepare(input.clone()).await.expect("prepare");
        assert_eq!(
            prepared.pending.as_ref().unwrap().activation_id,
            "activation-8"
        );

        let replay = repository
            .prepare(input.clone())
            .await
            .expect("identical replay");
        assert_eq!(
            replay, prepared,
            "identical replay must return current state"
        );

        let mut conflicting = prepare_input("activation-8x");
        conflicting.participant_replica_ids = vec!["runtime-a".to_string()];
        assert!(
            matches!(
                repository.prepare(conflicting).await,
                Err(RepositoryError::CasMismatch { .. })
            ),
            "different pending tuple must fail CAS"
        );

        let aborted = repository
            .abort(abort_input(&prepared))
            .await
            .expect("abort");
        assert!(aborted.pending.is_none());
        let replay_abort = repository
            .abort(abort_input(&prepared))
            .await
            .expect("abort replay");
        assert_eq!(replay_abort, aborted);

        let prepared_again = repository
            .prepare(input.clone())
            .await
            .expect("prepare again");
        let committed = repository
            .commit(commit_input(&prepared_again))
            .await
            .expect("commit");
        assert_eq!(committed.committed.generation, 8);
        let commit_replay = repository
            .commit(commit_input(&prepared_again))
            .await
            .expect("commit replay");
        assert_eq!(commit_replay, committed);

        let events = repository.audit_events().await;
        assert_eq!(
            events.len(),
            4,
            "one audit per effective mutation: prepare, abort, prepare, commit"
        );
        assert_eq!(events[0].operation, ActivationAuditOperation::Prepare);
        assert_eq!(events[1].operation, ActivationAuditOperation::Abort);
        assert_eq!(events[2].operation, ActivationAuditOperation::Prepare);
        assert_eq!(events[3].operation, ActivationAuditOperation::Commit);

        let read = repository.read("test").await.expect("read");
        assert_eq!(read, committed);
    }

    #[tokio::test]
    async fn audit_failure_rolls_back_the_whole_mutation() {
        let repository = MemoryActivationStateRepository::new();
        let initial = initial_state();
        repository.initialize(&initial).await.expect("initialize");
        repository.fail_next_audit_inserts(5).await;

        let error = repository
            .prepare(prepare_input("activation-failure"))
            .await
            .expect_err("audit append must fail");
        assert!(
            matches!(error, RepositoryError::Transient { .. }),
            "audit failure surfaces as transient: {error:?}"
        );
        let read = repository.read("test").await.expect("read");
        assert_eq!(
            read, initial,
            "state must be rolled back to the pre-mutation tuple"
        );
        assert!(
            repository.audit_events().await.is_empty(),
            "failed mutation must not leave an audit event"
        );
    }

    #[tokio::test]
    async fn transient_failures_retry_without_duplicate_audit() {
        let repository = MemoryActivationStateRepository::new();
        repository
            .initialize(&initial_state())
            .await
            .expect("initialize");
        repository.fail_next_transient_operations(2).await;

        let prepared = repository
            .prepare(prepare_input("activation-retry"))
            .await
            .expect("prepare");
        assert_eq!(prepared.pending.unwrap().activation_id, "activation-retry");
        assert_eq!(
            repository.audit_events().await.len(),
            1,
            "transient retries must not duplicate the audit event"
        );
    }

    #[tokio::test]
    async fn missing_state_read_and_initialize_conflict_are_cas_mismatch() {
        let repository = MemoryActivationStateRepository::new();
        assert!(matches!(
            repository.read("test").await,
            Err(RepositoryError::CasMismatch { .. })
        ));
        let initial = initial_state();
        repository.initialize(&initial).await.expect("initialize");
        let mut different = initial.clone();
        different.committed.assembly = assembly(2);
        assert!(matches!(
            repository.initialize(&different).await,
            Err(RepositoryError::CasMismatch { .. })
        ));
        let replay = repository
            .initialize(&initial)
            .await
            .expect("identical initialize");
        assert_eq!(replay, initial);
    }

    #[tokio::test]
    async fn append_audit_is_idempotent_by_event_id() {
        let repository = MemoryActivationStateRepository::new();
        let event = ActivationAuditEvent::new(
            "test",
            "activation-8",
            ActivationAuditOperation::Prepare,
            7,
            8,
            ActivationAuditOutcome::Ok,
            Some(vec!["runtime-a".to_string()]),
            1_752_531_600_000,
        );
        repository.append_audit(&event).await.expect("append");
        repository
            .append_audit(&event)
            .await
            .expect("idempotent append");
        repository
            .append_audit(&event)
            .await
            .expect("idempotent append");
        assert_eq!(repository.audit_events().await.len(), 1);
        assert_eq!(
            event.event_id,
            activation_audit_event_id(
                "test",
                "activation-8",
                ActivationAuditOperation::Prepare,
                7,
                8
            )
        );
    }
}
