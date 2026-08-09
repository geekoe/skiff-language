use serde::{de, Deserialize, Deserializer, Serialize};

use super::{
    error::ActorLifecycleContractError,
    identity::{validate_actor_execution_pair, ExactActorExecutionIdentityFrameMetadata},
};
use crate::{
    actor_owner::ActorOwnerLogicalKeyFrameHeader,
    protocol::{validate_task_actor_activation_snapshot, TaskActorActivationSnapshotFrameMetadata},
};

/// Durable activation facts plus the exact Actor code identity that can
/// decode and execute them. Incarnation, arena epoch, runtime and lease are
/// intentionally absent: a durable task competes for a fresh incarnation
/// after the previous in-memory Actor has been destroyed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableActorActivationSnapshotFrameMetadata {
    actor: ActorOwnerLogicalKeyFrameHeader,
    execution: ExactActorExecutionIdentityFrameMetadata,
    activation: TaskActorActivationSnapshotFrameMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawDurableActorActivationSnapshotFrameMetadata {
    actor: ActorOwnerLogicalKeyFrameHeader,
    execution: ExactActorExecutionIdentityFrameMetadata,
    activation: TaskActorActivationSnapshotFrameMetadata,
}

impl DurableActorActivationSnapshotFrameMetadata {
    pub fn new(
        actor: ActorOwnerLogicalKeyFrameHeader,
        execution: ExactActorExecutionIdentityFrameMetadata,
        activation: TaskActorActivationSnapshotFrameMetadata,
    ) -> Result<Self, ActorLifecycleContractError> {
        let snapshot = Self {
            actor,
            execution,
            activation,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn actor(&self) -> &ActorOwnerLogicalKeyFrameHeader {
        &self.actor
    }

    pub fn execution(&self) -> &ExactActorExecutionIdentityFrameMetadata {
        &self.execution
    }

    pub fn activation(&self) -> &TaskActorActivationSnapshotFrameMetadata {
        &self.activation
    }

    pub fn validate(&self) -> Result<(), ActorLifecycleContractError> {
        validate_actor_execution_pair(&self.actor, &self.execution)?;
        validate_task_actor_activation_snapshot(&self.activation)
            .map_err(|message| ActorLifecycleContractError::InvalidActivationSnapshot { message })
    }
}

impl<'de> Deserialize<'de> for DurableActorActivationSnapshotFrameMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawDurableActorActivationSnapshotFrameMetadata::deserialize(deserializer)?;
        Self::new(raw.actor, raw.execution, raw.activation).map_err(de::Error::custom)
    }
}
