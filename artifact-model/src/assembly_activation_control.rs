use serde::{de, Deserialize, Deserializer, Serialize};

use crate::RuntimeAssemblyRef;

pub const ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION: &str = "skiff-assembly-activation-request-v1";
pub const MAX_SAFE_ACTIVATION_GENERATION: u64 = 9_007_199_254_740_991;
pub const RUNTIME_ASSEMBLY_IDENTITY_PREFIX: &str = "skiff-runtime-assembly-v1:sha256";

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
        validate_activation_generation(self.expected_generation, "expectedGeneration")?;
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
        expected_generation: u64,
        candidate_generation: u64,
        assembly: RuntimeAssemblyRef,
        replica_id: String,
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

impl AssemblyActivationControl {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Prepare {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
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
            } => Self::Prepare {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
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
            } => Self::Commit {
                environment,
                activation_id,
                expected_generation,
                candidate_generation,
                assembly,
                replica_id,
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

pub fn validate_activation_token(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value != value.trim()
        || value.len() > 200
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "{label} must be non-empty, have no surrounding whitespace or control characters, and be at most 200 bytes"
        ));
    }
    Ok(())
}

pub fn validate_activation_environment(value: &str) -> Result<(), String> {
    validate_activation_token(value, "environment")?;
    if value == "."
        || value == ".."
        || !value.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_'
            )
        })
    {
        return Err(
            "environment must use only letters, digits, dot, dash, or underscore".to_string(),
        );
    }
    Ok(())
}

pub fn validate_activation_generation(generation: u64, label: &str) -> Result<(), String> {
    if generation > MAX_SAFE_ACTIVATION_GENERATION {
        return Err(format!(
            "{label} must be between 0 and {MAX_SAFE_ACTIVATION_GENERATION}"
        ));
    }
    Ok(())
}

pub fn validate_transition_generations(
    expected_generation: u64,
    candidate_generation: u64,
) -> Result<(), String> {
    validate_activation_generation(expected_generation, "expectedGeneration")?;
    validate_activation_generation(candidate_generation, "candidateGeneration")?;
    if expected_generation.checked_add(1) != Some(candidate_generation) {
        return Err("candidateGeneration must equal expectedGeneration + 1".to_string());
    }
    Ok(())
}

pub fn validate_runtime_assembly_ref(assembly: &RuntimeAssemblyRef) -> Result<(), String> {
    let expected_prefix = format!("{RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:");
    let Some(hash) = assembly
        .assembly_identity
        .as_str()
        .strip_prefix(&expected_prefix)
    else {
        return Err(format!(
            "assemblyIdentity must use {RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:<64 lowercase hex>"
        ));
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!(
            "assemblyIdentity must use {RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:<64 lowercase hex>"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[test]
    fn cross_language_golden_request_and_control_wire_decode_strictly() {
        let request_fixture = include_str!(
            "../../cross-system-fixtures/package-service-ecosystem/activation-request.json"
        );
        let request: AssemblyActivationRequest =
            serde_json::from_str(request_fixture).expect("canonical activation request fixture");
        assert_eq!(request.expected_generation, 41);
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::from_str::<Value>(request_fixture).unwrap()
        );

        let control_fixture =
            include_str!("../../cross-system-fixtures/package-service-ecosystem/control-wire.json");
        let controls: Vec<AssemblyActivationControl> =
            serde_json::from_str(control_fixture).expect("canonical control fixture");
        assert_eq!(controls.len(), 6);
        assert_eq!(
            serde_json::to_value(controls).unwrap(),
            serde_json::from_str::<Value>(control_fixture).unwrap()
        );
    }

    #[test]
    fn shared_request_and_control_mutation_corpus_fails_at_decode() {
        let request: Value = serde_json::from_str(include_str!(
            "../../cross-system-fixtures/package-service-ecosystem/activation-request.json"
        ))
        .unwrap();
        let controls: Vec<Value> = serde_json::from_str(include_str!(
            "../../cross-system-fixtures/package-service-ecosystem/control-wire.json"
        ))
        .unwrap();
        let mutations: Value = serde_json::from_str(include_str!(
            "../../cross-system-fixtures/package-service-ecosystem/activation-mutations.json"
        ))
        .unwrap();

        for mutation in mutations["request"].as_array().unwrap() {
            let candidate = apply_mutation(&request, mutation);
            assert!(
                serde_json::from_value::<AssemblyActivationRequest>(candidate).is_err(),
                "request mutation {} must fail",
                mutation["name"]
            );
        }
        for mutation in mutations["control"].as_array().unwrap() {
            let candidate = apply_mutation(&controls[0], mutation);
            assert!(
                serde_json::from_value::<AssemblyActivationControl>(candidate).is_err(),
                "control mutation {} must fail",
                mutation["name"]
            );
        }
    }

    fn apply_mutation(base: &Value, mutation: &Value) -> Value {
        let mut candidate = base.clone();
        let path = mutation["path"].as_array().expect("mutation path");
        let (last, parents) = path.split_last().expect("non-empty mutation path");
        let mut parent = &mut candidate;
        for segment in parents {
            parent = parent
                .as_object_mut()
                .expect("mutation object parent")
                .get_mut(segment.as_str().expect("path string"))
                .expect("mutation path exists");
        }
        let object = parent.as_object_mut().expect("mutation object");
        let field = last.as_str().expect("path string");
        match mutation["operation"].as_str().expect("mutation operation") {
            "replace" => {
                *object.get_mut(field).expect("replace path exists") = mutation["value"].clone();
            }
            "remove" => {
                object.remove(field).expect("remove path exists");
            }
            "add" => {
                assert!(
                    object
                        .insert(field.to_string(), mutation["value"].clone())
                        .is_none(),
                    "add path must be new"
                );
            }
            operation => panic!("unknown mutation operation {operation}"),
        }
        candidate
    }
}
