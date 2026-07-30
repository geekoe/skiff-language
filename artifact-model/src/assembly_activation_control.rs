use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
    validate_activation_environment, validate_activation_generation, validate_activation_token,
    validate_expected_activation_generation, validate_runtime_assembly_identity,
    validate_transition_generations, RuntimeAssemblyRef,
};

pub const ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION: &str = "skiff-assembly-activation-request-v1";

/// Strict tooling -> router request.  The router derives the candidate
/// generation and freezes the participant set; neither can be supplied by a
/// caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssemblyActivationRequest {
    pub schema_version: String,
    pub environment: String,
    pub activation_id: String,
    pub expected_generation: u64,
    pub assembly: RuntimeAssemblyRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawAssemblyActivationRequest {
    schema_version: String,
    environment: String,
    activation_id: String,
    #[serde(deserialize_with = "crate::deserialize_activation_generation")]
    expected_generation: u64,
    assembly: RuntimeAssemblyRef,
}

impl AssemblyActivationRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION {
            return Err(format!(
                "assembly activation request schemaVersion must be {ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION}"
            ));
        }
        validate_activation_environment(&self.environment)?;
        validate_activation_token(&self.activation_id, "activationId")?;
        validate_expected_activation_generation(self.expected_generation, "expectedGeneration")?;
        validate_runtime_assembly_ref(&self.assembly)
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
            environment: raw.environment,
            activation_id: raw.activation_id,
            expected_generation: raw.expected_generation,
            assembly: raw.assembly,
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
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Prepared {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Reject {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    },
    Commit {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Abort {
        environment: String,
        activation_id: String,
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Register {
        environment: String,
        generation: u64,
        assembly: RuntimeAssemblyRef,
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
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        #[serde(default)]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Prepared {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Reject {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        reason: AssemblyActivationRejectReason,
    },
    Commit {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
        #[serde(default)]
        service_db: Option<AssemblyActivationServiceDb>,
    },
    Abort {
        environment: String,
        activation_id: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        expected_generation: u64,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
    },
    Register {
        environment: String,
        #[serde(deserialize_with = "crate::deserialize_activation_generation")]
        generation: u64,
        assembly: RuntimeAssemblyRef,
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
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                ..
            }
            | Self::Prepared {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            }
            | Self::Reject {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                ..
            }
            | Self::Commit {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                ..
            }
            | Self::Abort {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            } => {
                validate_activation_environment(environment)?;
                validate_activation_token(activation_id, "activationId")?;
                validate_transition_generations(*expected_generation, *candidate_generation)?;
                validate_runtime_assembly_ref(assembly)?;
                validate_activation_token(replica_id, "replicaId")
            }
            Self::Register {
                environment,
                generation,
                assembly,
                replica_id,
            } => {
                validate_activation_environment(environment)?;
                validate_activation_generation(*generation, "generation")?;
                validate_runtime_assembly_ref(assembly)?;
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
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                service_db,
            } => Self::Prepare {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                service_db,
            },
            RawAssemblyActivationControl::Prepared {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            } => Self::Prepared {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            },
            RawAssemblyActivationControl::Reject {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                reason,
            } => Self::Reject {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                reason,
            },
            RawAssemblyActivationControl::Commit {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                service_db,
            } => Self::Commit {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
                service_db,
            },
            RawAssemblyActivationControl::Abort {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            } => Self::Abort {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
            },
            RawAssemblyActivationControl::Register {
                environment,
                generation,
                assembly,
                replica_id,
            } => Self::Register {
                environment,
                generation,
                assembly,
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
