use std::{collections::BTreeSet, path::Path};

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_identity::EnvironmentActivationStatePath;
use skiff_artifact_model::{
    validate_activation_environment, validate_activation_generation, validate_activation_token,
    validate_runtime_assembly_ref, validate_transition_generations, RuntimeAssemblyRef,
};

use super::{
    error::{EcosystemStorageError, StorageResult},
    io::{canonical_bytes, read_locked_bytes, CanonicalArtifactStore},
};

pub const ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION: &str =
    "skiff-environment-activation-state-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittedActivation {
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingActivation {
    pub activation_id: String,
    pub expected_generation: u64,
    pub candidate_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub participant_replica_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentActivationState {
    pub schema_version: String,
    pub environment: String,
    pub committed: CommittedActivation,
    pub pending: Option<PendingActivation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCommittedActivation {
    #[serde(deserialize_with = "skiff_artifact_model::deserialize_activation_generation")]
    generation: u64,
    assembly: RuntimeAssemblyRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPendingActivation {
    activation_id: String,
    #[serde(deserialize_with = "skiff_artifact_model::deserialize_activation_generation")]
    expected_generation: u64,
    #[serde(deserialize_with = "skiff_artifact_model::deserialize_activation_generation")]
    candidate_generation: u64,
    assembly: RuntimeAssemblyRef,
    participant_replica_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawEnvironmentActivationState {
    schema_version: String,
    environment: String,
    committed: RawCommittedActivation,
    pending: RequiredNullable<RawPendingActivation>,
}

#[derive(Debug)]
struct RequiredNullable<T>(Option<T>);

impl<'de, T> Deserialize<'de> for RequiredNullable<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Self)
    }
}

impl<'de> Deserialize<'de> for EnvironmentActivationState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawEnvironmentActivationState::deserialize(deserializer)?;
        let state = Self {
            schema_version: raw.schema_version,
            environment: raw.environment,
            committed: CommittedActivation {
                generation: raw.committed.generation,
                assembly: raw.committed.assembly,
            },
            pending: raw.pending.0.map(|pending| PendingActivation {
                activation_id: pending.activation_id,
                expected_generation: pending.expected_generation,
                candidate_generation: pending.candidate_generation,
                assembly: pending.assembly,
                participant_replica_ids: pending.participant_replica_ids,
            }),
        };
        state.validate().map_err(de::Error::custom)?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationRecoveryAction {
    StableCommitted,
    ReplayPrepare { replica_ids: Vec<String> },
    CommitPending,
    AbortPending { activation_id: String },
}

impl EnvironmentActivationState {
    pub fn initial(
        environment: impl Into<String>,
        generation: u64,
        assembly: RuntimeAssemblyRef,
    ) -> Self {
        Self {
            schema_version: ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
            environment: environment.into(),
            committed: CommittedActivation {
                generation,
                assembly,
            },
            pending: None,
        }
    }

    pub fn validate(&self) -> StorageResult<()> {
        map_activation_validation(
            &self.environment,
            validate_activation_environment(&self.environment),
        )?;
        let path = EnvironmentActivationStatePath::new(&self.environment)?;
        if self.schema_version != ENVIRONMENT_ACTIVATION_STATE_SCHEMA_VERSION {
            return invalid(path.as_str(), "activation state schemaVersion mismatch");
        }
        map_activation_validation(
            path.as_str(),
            validate_activation_generation(self.committed.generation, "committed.generation"),
        )?;
        map_activation_validation(
            path.as_str(),
            validate_runtime_assembly_ref(&self.committed.assembly),
        )?;
        if let Some(pending) = &self.pending {
            validate_token(&pending.activation_id, "activationId", path.as_str())?;
            if pending.expected_generation != self.committed.generation {
                return invalid(
                    path.as_str(),
                    "pending expectedGeneration must equal committed generation",
                );
            }
            map_activation_validation(
                path.as_str(),
                validate_transition_generations(
                    pending.expected_generation,
                    pending.candidate_generation,
                ),
            )?;
            map_activation_validation(
                path.as_str(),
                validate_runtime_assembly_ref(&pending.assembly),
            )?;
            let participants =
                normalized_replica_ids(&pending.participant_replica_ids, path.as_str())?;
            if participants != pending.participant_replica_ids {
                return invalid(
                    path.as_str(),
                    "participantReplicaIds must be non-empty, unique, and sorted",
                );
            }
        }
        Ok(())
    }

    pub fn recovery_action(
        &self,
        connected_replica_ids: &[String],
        prepared_replica_ids: &[String],
    ) -> StorageResult<ActivationRecoveryAction> {
        self.validate()?;
        let Some(pending) = &self.pending else {
            return Ok(ActivationRecoveryAction::StableCommitted);
        };
        let connected = normalized_set(connected_replica_ids, "connectedReplicaIds")?;
        let prepared = normalized_set(prepared_replica_ids, "preparedReplicaIds")?;
        let participants = pending
            .participant_replica_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if !participants.is_subset(&connected) || !prepared.is_subset(&participants) {
            return Ok(ActivationRecoveryAction::AbortPending {
                activation_id: pending.activation_id.clone(),
            });
        }
        if prepared == participants {
            return Ok(ActivationRecoveryAction::CommitPending);
        }
        Ok(ActivationRecoveryAction::ReplayPrepare {
            replica_ids: participants.difference(&prepared).cloned().collect(),
        })
    }
}

impl CanonicalArtifactStore {
    pub fn initialize_environment_activation(
        &self,
        state: &EnvironmentActivationState,
    ) -> StorageResult<()> {
        state.validate()?;
        if state.pending.is_some() {
            return invalid(
                &state.environment,
                "initial activation state cannot contain pending",
            );
        }
        self.read_runtime_assembly(&state.committed.assembly)?;
        self.cas_activation_state(&state.environment, None, state)
    }

    pub fn read_environment_activation(
        &self,
        environment: &str,
    ) -> StorageResult<EnvironmentActivationState> {
        let path = EnvironmentActivationStatePath::new(environment)?;
        let bytes = self.read_bytes(path.as_relative_path())?;
        let host_path = self.root().join(path.as_relative_path().as_path());
        let state = parse_state(&host_path, &bytes)?;
        if state.environment != environment {
            return invalid(&host_path, "activation state environment/path mismatch");
        }
        self.validate_activation_references(&state)?;
        Ok(state)
    }

    pub fn prepare_environment_activation(
        &self,
        environment: &str,
        activation_id: &str,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        participant_replica_ids: Vec<String>,
    ) -> StorageResult<EnvironmentActivationState> {
        self.read_runtime_assembly(&assembly)?;
        let participants = normalized_replica_ids(
            &participant_replica_ids,
            &EnvironmentActivationStatePath::new(environment)?.to_string(),
        )?;
        let pending = PendingActivation {
            activation_id: activation_id.to_string(),
            expected_generation,
            candidate_generation,
            assembly,
            participant_replica_ids: participants,
        };
        self.mutate_activation_state(environment, |current| {
            if current.committed.generation != expected_generation {
                return cas_error(
                    environment,
                    format!(
                        "committed generation {} does not equal expected {expected_generation}",
                        current.committed.generation
                    ),
                );
            }
            match &current.pending {
                Some(existing) if existing == &pending => return Ok(current.clone()),
                Some(_) => {
                    return cas_error(environment, "a different activation is already pending")
                }
                None => {}
            }
            let mut next = current.clone();
            next.pending = Some(pending);
            next.validate()?;
            Ok(next)
        })
    }

    pub fn abort_environment_activation(
        &self,
        environment: &str,
        activation_id: &str,
        expected_generation: u64,
    ) -> StorageResult<EnvironmentActivationState> {
        let path = EnvironmentActivationStatePath::new(environment)?;
        validate_token(activation_id, "activationId", path.as_str())?;
        self.mutate_activation_state(environment, |current| {
            if current.committed.generation != expected_generation {
                return cas_error(environment, "abort expected generation is stale");
            }
            let Some(pending) = &current.pending else {
                return Ok(current.clone());
            };
            if pending.activation_id != activation_id {
                return cas_error(environment, "abort activationId does not match pending");
            }
            let mut next = current.clone();
            next.pending = None;
            Ok(next)
        })
    }

    pub fn commit_environment_activation(
        &self,
        environment: &str,
        activation_id: &str,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: &RuntimeAssemblyRef,
        connected_replica_ids: &[String],
        prepared_replica_ids: &[String],
    ) -> StorageResult<EnvironmentActivationState> {
        let path = EnvironmentActivationStatePath::new(environment)?;
        validate_token(activation_id, "activationId", path.as_str())?;
        let expected_candidate = expected_generation.checked_add(1).ok_or_else(|| {
            EcosystemStorageError::CasMismatch {
                path: path.as_str().into(),
                message: "commit generation overflow".to_string(),
            }
        })?;
        if candidate_generation != expected_candidate {
            return cas_error(
                path.as_str(),
                "candidateGeneration must be expectedGeneration + 1",
            );
        }
        let connected = normalized_set(connected_replica_ids, "connectedReplicaIds")?;
        let prepared = normalized_set(prepared_replica_ids, "preparedReplicaIds")?;
        self.mutate_activation_state(environment, |current| {
            if current.pending.is_none()
                && current.committed.generation == candidate_generation
                && &current.committed.assembly == assembly
            {
                return Ok(current.clone());
            }
            if current.committed.generation != expected_generation {
                return cas_error(environment, "commit expected generation is stale");
            }
            let pending = current.pending.as_ref().ok_or_else(|| {
                EcosystemStorageError::CasMismatch {
                    path: environment.into(),
                    message: "commit has no pending activation".to_string(),
                }
            })?;
            if pending.activation_id != activation_id
                || pending.expected_generation != expected_generation
                || pending.candidate_generation != candidate_generation
                || &pending.assembly != assembly
            {
                return cas_error(environment, "commit tuple does not match pending activation");
            }
            let participants = pending
                .participant_replica_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if !participants.is_subset(&connected) || participants != prepared
            {
                return cas_error(
                    environment,
                    "commit requires the exact connected and prepared ACK sets for all participants",
                );
            }
            let mut next = current.clone();
            next.committed = CommittedActivation {
                generation: candidate_generation,
                assembly: assembly.clone(),
            };
            next.pending = None;
            next.validate()?;
            Ok(next)
        })
    }

    fn mutate_activation_state(
        &self,
        environment: &str,
        mutate: impl FnOnce(&EnvironmentActivationState) -> StorageResult<EnvironmentActivationState>,
    ) -> StorageResult<EnvironmentActivationState> {
        let path = EnvironmentActivationStatePath::new(environment)?;
        self.with_exclusive_pointer_lock(path.as_relative_path(), |destination| {
            let bytes = read_locked_bytes(destination)?.ok_or_else(|| {
                EcosystemStorageError::CasMismatch {
                    path: destination.to_path_buf(),
                    message: "environment activation state does not exist".to_string(),
                }
            })?;
            let current = parse_state(destination, &bytes)?;
            if current.environment != environment {
                return invalid(destination, "activation state environment/path mismatch");
            }
            self.validate_activation_references(&current)?;
            let next = mutate(&current)?;
            next.validate()?;
            self.validate_activation_references(&next)?;
            if next != current {
                self.replace_locked(destination, &canonical_bytes(&next)?)?;
            }
            Ok(next)
        })
    }

    fn cas_activation_state(
        &self,
        environment: &str,
        expected: Option<&EnvironmentActivationState>,
        candidate: &EnvironmentActivationState,
    ) -> StorageResult<()> {
        let path = EnvironmentActivationStatePath::new(environment)?;
        self.with_exclusive_pointer_lock(path.as_relative_path(), |destination| {
            let current = read_locked_bytes(destination)?
                .map(|bytes| parse_state(destination, &bytes))
                .transpose()?;
            if current.as_ref() != expected {
                return cas_error(
                    destination,
                    "activation state does not equal expected state",
                );
            }
            self.replace_locked(destination, &canonical_bytes(candidate)?)
        })
    }

    fn validate_activation_references(
        &self,
        state: &EnvironmentActivationState,
    ) -> StorageResult<()> {
        self.read_runtime_assembly(&state.committed.assembly)?;
        if let Some(pending) = &state.pending {
            self.read_runtime_assembly(&pending.assembly)?;
        }
        Ok(())
    }
}

fn parse_state(path: &Path, bytes: &[u8]) -> StorageResult<EnvironmentActivationState> {
    let state = serde_json::from_slice::<EnvironmentActivationState>(bytes).map_err(|source| {
        EcosystemStorageError::Json {
            path: path.to_path_buf(),
            source,
        }
    })?;
    state.validate()?;
    if canonical_bytes(&state)? != bytes {
        return invalid(path, "activation state bytes are not canonical JSON");
    }
    Ok(state)
}

fn normalized_replica_ids(values: &[String], path: &str) -> StorageResult<Vec<String>> {
    if values.is_empty() {
        return invalid(path, "participant/ACK replica set must not be empty");
    }
    let set = normalized_set(values, "replica ids")?;
    if set.len() != values.len() {
        return invalid(path, "replica ids must be unique");
    }
    Ok(set.into_iter().collect())
}

fn normalized_set(values: &[String], label: &str) -> StorageResult<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for value in values {
        validate_token(value, label, label)?;
        if !result.insert(value.clone()) {
            return invalid(label, format!("{label} contains duplicate {value}"));
        }
    }
    Ok(result)
}

fn validate_token(value: &str, label: &str, path: &str) -> StorageResult<()> {
    map_activation_validation(path, validate_activation_token(value, label))
}

fn map_activation_validation(
    path: impl AsRef<Path>,
    validation: Result<(), String>,
) -> StorageResult<()> {
    validation.map_err(|message| EcosystemStorageError::InvalidRecord {
        path: path.as_ref().to_path_buf(),
        message,
    })
}

fn cas_error<T>(path: impl AsRef<Path>, message: impl Into<String>) -> StorageResult<T> {
    Err(EcosystemStorageError::CasMismatch {
        path: path.as_ref().to_path_buf(),
        message: message.into(),
    })
}

fn invalid<T>(path: impl AsRef<Path>, message: impl Into<String>) -> StorageResult<T> {
    Err(EcosystemStorageError::InvalidRecord {
        path: path.as_ref().to_path_buf(),
        message: message.into(),
    })
}

#[cfg(test)]
#[path = "activation/tests.rs"]
mod tests;
