use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
    validate_activation_generation, validate_activation_profile, validate_activation_token,
    validate_expected_activation_generation, validate_runtime_assembly_identity,
    validate_runtime_config_snapshot_ref, validate_transition_generations, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};

pub const ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION: &str = "skiff-assembly-activation-request-v3";

/// Strict tooling -> router request.  The router derives the candidate
/// generation and freezes the participant set; neither can be supplied by a
/// caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyActivationRequest {
    pub schema_version: String,
    pub profile: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAssemblyActivationRequest {
    schema_version: String,
    profile: String,
    activation_id: String,
    #[serde(deserialize_with = "crate::deserialize_activation_generation")]
    expected_generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
}

impl AssemblyActivationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION {
            return Err(format!(
                "assembly activation request schemaVersion must be {ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION}"
            ));
        }
        validate_activation_profile(&self.profile)?;
        validate_activation_token(&self.activation_id, "activationId")?;
        validate_expected_activation_generation(self.expected_generation, "expectedGeneration")?;
        validate_runtime_assembly_ref(&self.assembly)?;
        validate_runtime_config_snapshot_ref(&self.config_snapshot)
    }
}

impl<'de> Deserialize<'de> for AssemblyActivationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawAssemblyActivationRequest::deserialize(deserializer)?;
        let request = Self {
            schema_version: raw.schema_version,
            profile: raw.profile,
            activation_id: raw.activation_id,
            expected_generation: raw.expected_generation,
            assembly: raw.assembly,
            config_snapshot: raw.config_snapshot,
        };
        request.validate().map_err(de::Error::custom)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssemblyActivationRejectReason {
    Resolve,
    Load,
    Link,
    Admission,
    ParticipantDisconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssemblyActivationServiceDb {
    #[serde(deserialize_with = "deserialize_non_empty_mongo_url")]
    pub mongo_url: String,
}

/// Exact router <-> runtime control wire for whole-assembly activation.
///
/// Transition messages are intentionally assembly-scoped. Service ids, build
/// ids, artifact roots and executable targets cannot be represented here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssemblyActivationControl {
    Prepare {
        profile: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Prepared {
        profile: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
    Reject {
        profile: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    },
    Commit {
        profile: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Abort {
        profile: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
    Register {
        profile: String,
        generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum RawAssemblyActivationControl {
    Prepare {
        profile: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        #[serde(default)]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Prepared {
        profile: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
    Reject {
        profile: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    },
    Commit {
        profile: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
        #[serde(default)]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Abort {
        profile: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
    Register {
        profile: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        generation: u64,
        assembly: RuntimeAssemblyRef,
        config_snapshot: RuntimeConfigSnapshotRef,
        replica_id: String,
    },
}

impl AssemblyActivationControl {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Prepare { service_db, .. } | Self::Commit { service_db, .. } => {
                if service_db
                    .as_ref()
                    .is_some_and(|value| value.mongo_url.trim().is_empty())
                {
                    return Err("serviceDb.mongoUrl must be a non-empty string".to_string());
                }
            }
            _ => {}
        }
        match self {
            Self::Prepare {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                ..
            }
            | Self::Prepared {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            }
            | Self::Reject {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                ..
            }
            | Self::Commit {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                ..
            }
            | Self::Abort {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            } => {
                validate_activation_profile(profile)?;
                validate_activation_token(activation_id, "activationId")?;
                validate_transition_generations(*expected_generation, *candidate_generation)?;
                validate_runtime_assembly_ref(assembly)?;
                validate_runtime_config_snapshot_ref(config_snapshot)?;
                validate_activation_token(replica_id, "replicaId")
            }
            Self::Register {
                profile,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            } => {
                validate_activation_profile(profile)?;
                validate_activation_generation(*generation, "generation")?;
                validate_runtime_assembly_ref(assembly)?;
                validate_runtime_config_snapshot_ref(config_snapshot)?;
                validate_activation_token(replica_id, "replicaId")
            }
        }
    }
}

impl<'de> Deserialize<'de> for AssemblyActivationControl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawAssemblyActivationControl::deserialize(deserializer)?;
        let control = match raw {
            RawAssemblyActivationControl::Prepare {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            } => Self::Prepare {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            },
            RawAssemblyActivationControl::Prepared {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            } => Self::Prepared {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            },
            RawAssemblyActivationControl::Reject {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                reason,
            } => Self::Reject {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                reason,
            },
            RawAssemblyActivationControl::Commit {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            } => Self::Commit {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
                service_db,
            },
            RawAssemblyActivationControl::Abort {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            } => Self::Abort {
                profile,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                config_snapshot,
                replica_id,
            },
            RawAssemblyActivationControl::Register {
                profile,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            } => Self::Register {
                profile,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            },
        };
        control.validate().map_err(de::Error::custom)?;
        Ok(control)
    }
}

fn deserialize_non_empty_mongo_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(de::Error::custom(
            "serviceDb.mongoUrl must be a non-empty string",
        ));
    }
    Ok(value)
}

pub fn validate_runtime_assembly_ref(assembly: &RuntimeAssemblyRef) -> Result<(), String> {
    validate_runtime_assembly_identity(assembly.assembly_identity.as_str())
}

#[cfg(test)]
#[path = "assembly_activation_control/tests.rs"]
mod tests;
