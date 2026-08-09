use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_identity::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, DeploymentArtifactIdentity, ServiceDeploymentRef,
};

use super::{
    error::ActorLifecycleContractError,
    validation::{
        validate_non_empty, validate_positive_sequence, validate_sha256_identity, validate_token,
        SHA256_PREFIX,
    },
};
use crate::actor_owner::{validate_logical_key, ActorOwnerLogicalKeyFrameHeader};

/// Exact, path-free deployment owner. Its build id is the deployment artifact
/// identity; the full coordinate lets consumers reject one build id rebound
/// to a different service/revision tuple.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ExactDeploymentOwnerFrameMetadata {
    deployment: ServiceDeploymentRef,
}

impl ExactDeploymentOwnerFrameMetadata {
    pub fn new(deployment: ServiceDeploymentRef) -> Result<Self, ActorLifecycleContractError> {
        let owner = Self { deployment };
        owner.validate()?;
        Ok(owner)
    }

    pub fn deployment(&self) -> &ServiceDeploymentRef {
        &self.deployment
    }

    pub fn build_id(&self) -> &DeploymentArtifactIdentity {
        &self.deployment.deployment_artifact_identity
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        validate_non_empty(&self.deployment.service_id, "deploymentOwner.serviceId")?;
        validate_non_empty(
            &self.deployment.contract_version,
            "deploymentOwner.contractVersion",
        )?;
        validate_non_empty(
            self.deployment.deployment_revision.as_str(),
            "deploymentOwner.deploymentRevision",
        )?;
        validate_sha256_identity(
            self.deployment.deployment_artifact_identity.as_str(),
            DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
            "deploymentOwner.deploymentArtifactIdentity",
        )
    }
}

impl<'de> Deserialize<'de> for ExactDeploymentOwnerFrameMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let deployment = ServiceDeploymentRef::deserialize(deserializer)?;
        Self::new(deployment).map_err(de::Error::custom)
    }
}

/// Exact code identity pinned by an Actor incarnation or a durable activation
/// snapshot. It contains identities only, never executable indices/addresses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactActorExecutionIdentityFrameMetadata {
    deployment_owner: ExactDeploymentOwnerFrameMetadata,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawExactActorExecutionIdentityFrameMetadata {
    deployment_owner: ExactDeploymentOwnerFrameMetadata,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
}

impl ExactActorExecutionIdentityFrameMetadata {
    pub fn new(
        deployment_owner: ExactDeploymentOwnerFrameMetadata,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
    ) -> Result<Self, ActorLifecycleContractError> {
        let identity = Self {
            deployment_owner,
            actor_abi_identity,
            actor_implementation_identity,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn deployment_owner(&self) -> &ExactDeploymentOwnerFrameMetadata {
        &self.deployment_owner
    }

    pub fn build_id(&self) -> &DeploymentArtifactIdentity {
        self.deployment_owner.build_id()
    }

    pub fn actor_abi_identity(&self) -> &ActorAbiIdentity {
        &self.actor_abi_identity
    }

    pub fn actor_implementation_identity(&self) -> &ActorImplementationIdentity {
        &self.actor_implementation_identity
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        self.deployment_owner.validate()?;
        validate_sha256_identity(
            self.actor_abi_identity.as_str(),
            ACTOR_ABI_IDENTITY_PREFIX,
            "actorAbiIdentity",
        )?;
        validate_sha256_identity(
            self.actor_implementation_identity.as_str(),
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            "actorImplementationIdentity",
        )
    }
}

impl<'de> Deserialize<'de> for ExactActorExecutionIdentityFrameMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawExactActorExecutionIdentityFrameMetadata::deserialize(deserializer)?;
        Self::new(
            raw.deployment_owner,
            raw.actor_abi_identity,
            raw.actor_implementation_identity,
        )
        .map_err(de::Error::custom)
    }
}

/// Monotonic identity of one live incarnation for one logical Actor key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ActorIncarnation(u64);

impl ActorIncarnation {
    pub fn new(value: u64) -> Result<Self, ActorLifecycleContractError> {
        validate_positive_sequence(value, "incarnation")?;
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ActorIncarnation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Epoch of the current physical Actor arena. Compaction advances this value
/// without changing the logical Actor identity or deployment build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ActorArenaEpoch(u64);

impl ActorArenaEpoch {
    pub fn new(value: u64) -> Result<Self, ActorLifecycleContractError> {
        validate_positive_sequence(value, "arenaEpoch")?;
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ActorArenaEpoch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Complete fence for one materialized Actor arena.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactActorOwnerFenceFrameMetadata {
    actor: ActorOwnerLogicalKeyFrameHeader,
    execution: ExactActorExecutionIdentityFrameMetadata,
    incarnation: ActorIncarnation,
    arena_epoch: ActorArenaEpoch,
    owner_runtime_id: String,
    owner_lease_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawExactActorOwnerFenceFrameMetadata {
    actor: ActorOwnerLogicalKeyFrameHeader,
    execution: ExactActorExecutionIdentityFrameMetadata,
    incarnation: ActorIncarnation,
    arena_epoch: ActorArenaEpoch,
    owner_runtime_id: String,
    owner_lease_id: String,
}

impl ExactActorOwnerFenceFrameMetadata {
    pub fn new(
        actor: ActorOwnerLogicalKeyFrameHeader,
        execution: ExactActorExecutionIdentityFrameMetadata,
        incarnation: ActorIncarnation,
        arena_epoch: ActorArenaEpoch,
        owner_runtime_id: impl Into<String>,
        owner_lease_id: impl Into<String>,
    ) -> Result<Self, ActorLifecycleContractError> {
        let fence = Self {
            actor,
            execution,
            incarnation,
            arena_epoch,
            owner_runtime_id: owner_runtime_id.into(),
            owner_lease_id: owner_lease_id.into(),
        };
        fence.validate()?;
        Ok(fence)
    }

    pub fn actor(&self) -> &ActorOwnerLogicalKeyFrameHeader {
        &self.actor
    }

    pub fn execution(&self) -> &ExactActorExecutionIdentityFrameMetadata {
        &self.execution
    }

    pub const fn incarnation(&self) -> ActorIncarnation {
        self.incarnation
    }

    pub const fn arena_epoch(&self) -> ActorArenaEpoch {
        self.arena_epoch
    }

    pub fn owner_runtime_id(&self) -> &str {
        &self.owner_runtime_id
    }

    pub fn owner_lease_id(&self) -> &str {
        &self.owner_lease_id
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        validate_actor_execution_pair(&self.actor, &self.execution)?;
        validate_positive_sequence(self.incarnation.get(), "incarnation")?;
        validate_positive_sequence(self.arena_epoch.get(), "arenaEpoch")?;
        validate_token(&self.owner_runtime_id, "ownerRuntimeId")?;
        validate_token(&self.owner_lease_id, "ownerLeaseId")
    }
}

impl<'de> Deserialize<'de> for ExactActorOwnerFenceFrameMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawExactActorOwnerFenceFrameMetadata::deserialize(deserializer)?;
        Self::new(
            raw.actor,
            raw.execution,
            raw.incarnation,
            raw.arena_epoch,
            raw.owner_runtime_id,
            raw.owner_lease_id,
        )
        .map_err(de::Error::custom)
    }
}

pub(super) fn validate_actor_execution_pair(
    actor: &ActorOwnerLogicalKeyFrameHeader,
    execution: &ExactActorExecutionIdentityFrameMetadata,
) -> Result<(), ActorLifecycleContractError> {
    validate_logical_key(actor).map_err(|error| {
        ActorLifecycleContractError::InvalidActorLogicalKey {
            message: error.to_string(),
        }
    })?;
    validate_sha256_identity(
        actor.actor_id_hash.as_str(),
        SHA256_PREFIX,
        "actor.actorIdHash",
    )?;
    execution.validate()?;
    let deployment_service_id = &execution.deployment_owner().deployment().service_id;
    if actor.service_id != *deployment_service_id {
        return Err(
            ActorLifecycleContractError::ActorDeploymentServiceMismatch {
                actor_service_id: actor.service_id.clone(),
                deployment_service_id: deployment_service_id.clone(),
            },
        );
    }
    Ok(())
}
