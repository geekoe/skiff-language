//! Adapter-independent durable activation state reducer.
//!
//! The transition functions mirror the CAS semantics of the frozen file
//! adapter (`deployment/src/storage/activation.rs`) exactly: stale generation,
//! occupied pending slot, mismatched pending tuple, and imprecise ACK sets are
//! `CasMismatch`; lexical/schema corruption is `InvalidRecord`; a completely
//! identical mutation tuple replays to the current state without an error.
//! The Router-owned Mongo adapter and the file adapter must produce the same
//! outcomes for the same operation sequence (conformance corpus in
//! `deployment/tests/activation_reducer_contract.rs`).

use std::collections::BTreeSet;

use skiff_artifact_model::{
    validate_activation_token, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
};

use crate::storage::{EnvironmentActivationState, PendingActivation};

use super::error::{cas_error, invalid_error, ActivationStateError, ActivationStateResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareInput {
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
    pub participant_replica_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInput {
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
    pub connected_replica_ids: Vec<String>,
    pub prepared_replica_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortInput {
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
}

pub fn prepare(
    current: &EnvironmentActivationState,
    input: &PrepareInput,
) -> ActivationStateResult<EnvironmentActivationState> {
    let participants = canonical_replica_ids(&input.environment, &input.participant_replica_ids)?;
    validate_current(current, &input.environment)?;
    if current.committed.generation != input.expected_generation {
        return Err(cas_error(
            &input.environment,
            format!(
                "committed generation {} does not equal expected {}",
                current.committed.generation, input.expected_generation
            ),
        ));
    }
    let pending = PendingActivation {
        activation_id: input.activation_id.clone(),
        expected_generation: input.expected_generation,
        candidate_generation: input.candidate_generation,
        assembly: input.assembly.clone(),
        config_snapshot: input.config_snapshot.clone(),
        participant_replica_ids: participants,
    };
    match &current.pending {
        Some(existing) if existing == &pending => return Ok(current.clone()),
        Some(_) => {
            return Err(cas_error(
                &input.environment,
                "a different activation is already pending",
            ))
        }
        None => {}
    }
    let mut next = current.clone();
    next.pending = Some(pending);
    validate_next(&next)?;
    Ok(next)
}

pub fn abort(
    current: &EnvironmentActivationState,
    input: &AbortInput,
) -> ActivationStateResult<EnvironmentActivationState> {
    validate_token(&input.environment, &input.activation_id, "activationId")?;
    validate_current(current, &input.environment)?;
    if current.committed.generation != input.expected_generation {
        return Err(cas_error(
            &input.environment,
            "abort expected generation is stale",
        ));
    }
    let Some(pending) = &current.pending else {
        return Ok(current.clone());
    };
    if pending.activation_id != input.activation_id {
        return Err(cas_error(
            &input.environment,
            "abort activationId does not match pending",
        ));
    }
    let mut next = current.clone();
    next.pending = None;
    validate_next(&next)?;
    Ok(next)
}

pub fn commit(
    current: &EnvironmentActivationState,
    input: &CommitInput,
) -> ActivationStateResult<EnvironmentActivationState> {
    validate_token(&input.environment, &input.activation_id, "activationId")?;
    let expected_candidate = input.expected_generation.checked_add(1).ok_or_else(|| {
        ActivationStateError::CasMismatch {
            environment: input.environment.clone(),
            message: "commit generation overflow".to_string(),
        }
    })?;
    if input.candidate_generation != expected_candidate {
        return Err(cas_error(
            &input.environment,
            "candidateGeneration must be expectedGeneration + 1",
        ));
    }
    let connected = canonical_ack_set(&input.environment, &input.connected_replica_ids)?;
    let prepared = canonical_ack_set(&input.environment, &input.prepared_replica_ids)?;
    validate_current(current, &input.environment)?;
    if current.pending.is_none()
        && current.committed.generation == input.candidate_generation
        && current.committed.assembly == input.assembly
        && current.committed.config_snapshot == input.config_snapshot
    {
        return Ok(current.clone());
    }
    if current.committed.generation != input.expected_generation {
        return Err(cas_error(
            &input.environment,
            "commit expected generation is stale",
        ));
    }
    let pending = current
        .pending
        .as_ref()
        .ok_or_else(|| ActivationStateError::CasMismatch {
            environment: input.environment.clone(),
            message: "commit has no pending activation".to_string(),
        })?;
    if pending.activation_id != input.activation_id
        || pending.expected_generation != input.expected_generation
        || pending.candidate_generation != input.candidate_generation
        || pending.assembly != input.assembly
        || pending.config_snapshot != input.config_snapshot
    {
        return Err(cas_error(
            &input.environment,
            "commit tuple does not match pending activation",
        ));
    }
    let participants = pending
        .participant_replica_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if !participants.is_subset(&connected) || participants != prepared {
        return Err(cas_error(
            &input.environment,
            "commit requires the exact connected and prepared ACK sets for all participants",
        ));
    }
    let mut next = current.clone();
    next.committed = crate::storage::CommittedActivation {
        generation: input.candidate_generation,
        assembly: input.assembly.clone(),
        config_snapshot: input.config_snapshot.clone(),
    };
    next.pending = None;
    validate_next(&next)?;
    Ok(next)
}

fn validate_current(
    current: &EnvironmentActivationState,
    environment: &str,
) -> ActivationStateResult<()> {
    if current.environment != environment {
        return Err(invalid_error(environment, "unknown activation environment"));
    }
    current
        .validate()
        .map_err(|error| invalid_error(environment, error.to_string()))
}

fn validate_next(next: &EnvironmentActivationState) -> ActivationStateResult<()> {
    next.validate()
        .map_err(|error| invalid_error(&next.environment, error.to_string()))
}

fn validate_token(environment: &str, value: &str, label: &str) -> ActivationStateResult<()> {
    validate_activation_token(value, label).map_err(|message| invalid_error(environment, message))
}

fn canonical_replica_ids(
    environment: &str,
    values: &[String],
) -> ActivationStateResult<Vec<String>> {
    if values.is_empty() {
        return Err(invalid_error(
            environment,
            "participant/ACK replica set must not be empty",
        ));
    }
    let set = canonical_ack_set(environment, values)?;
    if set.len() != values.len() {
        return Err(invalid_error(environment, "replica ids must be unique"));
    }
    Ok(set.into_iter().collect())
}

fn canonical_ack_set(
    environment: &str,
    values: &[String],
) -> ActivationStateResult<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for value in values {
        validate_token(environment, value, "replica id")?;
        if !result.insert(value.clone()) {
            return Err(invalid_error(
                environment,
                format!("replica ids must be unique: duplicate {value}"),
            ));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION;
    use skiff_artifact_model::AssemblyIdentity;

    fn refs() -> (
        RuntimeAssemblyRef,
        RuntimeConfigSnapshotRef,
        RuntimeConfigSnapshotRef,
    ) {
        use skiff_artifact_model::RuntimeConfigSnapshotId;
        let assembly = RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        };
        let config = RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("config snapshot id"),
        };
        let other_config = RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("config snapshot id"),
        };
        (assembly, config, other_config)
    }

    fn initial() -> EnvironmentActivationState {
        let (assembly, config, _) = refs();
        EnvironmentActivationState {
            schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
            environment: "test".to_string(),
            committed: crate::storage::CommittedActivation {
                generation: 7,
                assembly,
                config_snapshot: config,
            },
            pending: None,
        }
    }

    fn prepare_input(activation_id: &str, expected: u64) -> PrepareInput {
        let (assembly, _, config) = refs();
        PrepareInput {
            environment: "test".to_string(),
            activation_id: activation_id.to_string(),
            expected_generation: expected,
            candidate_generation: expected + 1,
            assembly,
            config_snapshot: config,
            participant_replica_ids: vec!["runtime-b".to_string(), "runtime-a".to_string()],
        }
    }

    #[test]
    fn prepare_canonicalizes_participants_and_replays_identically() {
        let state = initial();
        let input = prepare_input("activation-8", 7);
        let prepared = prepare(&state, &input).expect("prepare");
        assert_eq!(
            prepared.pending.as_ref().unwrap().participant_replica_ids,
            vec!["runtime-a".to_string(), "runtime-b".to_string()]
        );
        let replay = prepare(&prepared, &input).expect("identical replay");
        assert_eq!(replay, prepared);
    }

    #[test]
    fn prepare_conflict_and_stale_generation_are_cas_mismatch() {
        let state = initial();
        let first = prepare_input("activation-8", 7);
        let prepared = prepare(&state, &first).expect("prepare");
        let other = prepare_input("activation-8x", 7);
        assert!(matches!(
            prepare(&prepared, &other),
            Err(ActivationStateError::CasMismatch { .. })
        ));
        let stale = prepare_input("activation-9", 6);
        assert!(matches!(
            prepare(&state, &stale),
            Err(ActivationStateError::CasMismatch { .. })
        ));
    }

    #[test]
    fn commit_requires_exact_ack_sets_and_replays_after_success() {
        let state = initial();
        let input = prepare_input("activation-8", 7);
        let prepared = prepare(&state, &input).expect("prepare");
        let partial = CommitInput {
            environment: "test".to_string(),
            activation_id: "activation-8".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: input.assembly.clone(),
            config_snapshot: input.config_snapshot.clone(),
            connected_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
            prepared_replica_ids: vec!["runtime-a".to_string()],
        };
        assert!(matches!(
            commit(&prepared, &partial),
            Err(ActivationStateError::CasMismatch { .. })
        ));
        let full = CommitInput {
            prepared_replica_ids: vec!["runtime-a".to_string(), "runtime-b".to_string()],
            ..partial
        };
        let committed = commit(&prepared, &full).expect("commit");
        assert_eq!(committed.committed.generation, 8);
        assert!(committed.pending.is_none());
        let replay = commit(&committed, &full).expect("identical replay");
        assert_eq!(replay, committed);
    }

    #[test]
    fn abort_clears_pending_and_is_idempotent_without_pending() {
        let state = initial();
        let input = prepare_input("activation-8", 7);
        let prepared = prepare(&state, &input).expect("prepare");
        let aborted = abort(
            &prepared,
            &AbortInput {
                environment: "test".to_string(),
                activation_id: "activation-8".to_string(),
                expected_generation: 7,
            },
        )
        .expect("abort");
        assert!(aborted.pending.is_none());
        let replay = abort(
            &aborted,
            &AbortInput {
                environment: "test".to_string(),
                activation_id: "activation-8".to_string(),
                expected_generation: 7,
            },
        )
        .expect("abort without pending");
        assert_eq!(replay, aborted);
    }

    #[test]
    fn invalid_inputs_are_invalid_record() {
        let state = initial();
        let mut empty = prepare_input("activation-8", 7);
        empty.participant_replica_ids.clear();
        assert!(matches!(
            prepare(&state, &empty),
            Err(ActivationStateError::InvalidRecord { .. })
        ));
        let mut bad_generation = prepare_input("activation-8", 7);
        bad_generation.candidate_generation = 9;
        assert!(matches!(
            prepare(&state, &bad_generation),
            Err(ActivationStateError::InvalidRecord { .. })
        ));
        let mut duplicate = prepare_input("activation-8", 7);
        duplicate.participant_replica_ids = vec!["runtime-a".to_string(); 2];
        assert!(matches!(
            prepare(&state, &duplicate),
            Err(ActivationStateError::InvalidRecord { .. })
        ));
    }

    #[test]
    fn overflow_commit_is_cas_mismatch() {
        let state = initial();
        let input = CommitInput {
            environment: "test".to_string(),
            activation_id: "activation-8".to_string(),
            expected_generation: u64::MAX,
            candidate_generation: 0,
            assembly: refs().0,
            config_snapshot: refs().1,
            connected_replica_ids: Vec::new(),
            prepared_replica_ids: Vec::new(),
        };
        assert!(matches!(
            commit(&state, &input),
            Err(ActivationStateError::CasMismatch { .. })
        ));
    }
}
