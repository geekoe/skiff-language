use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use crate::compile_identity::{
    ContractOperationId, PackageBuildId, PackageLocalAbiIdentity, ServiceProtocolIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRequirement {
    pub alias: String,
    pub package_id: String,
    pub exact_version: String,
    pub expected_local_abi: PackageLocalAbiIdentity,
    /// Test-service dependencies with `topLevelAlias` bind the exact
    /// implementation build because private symbols are outside the public
    /// Local ABI. Ordinary public dependencies leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_package_build: Option<PackageBuildId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractRequirement {
    pub alias: String,
    pub service_id: String,
    pub contract_version: String,
    pub expected_protocol_identity: ServiceProtocolIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRequirement {
    pub contract_requirement: ContractRequirement,
    pub service_binding_slot: u32,
    pub used_operations: BTreeSet<ContractOperationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceCallRef {
    pub service_requirement_slot: u32,
    pub contract_operation_id: ContractOperationId,
    pub expected_protocol_identity: ServiceProtocolIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageConfigRequirement {
    pub path: String,
    pub access: PackageConfigAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PackageConfigAccess {
    Presence,
    Optional { value_type: String },
    Required { value_type: String },
}

impl PackageConfigAccess {
    pub fn value_type(&self) -> Option<&str> {
        match self {
            Self::Presence => None,
            Self::Optional { value_type } | Self::Required { value_type } => Some(value_type),
        }
    }

    pub fn is_required(&self) -> bool {
        matches!(self, Self::Required { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageConfigRequirementMergeError {
    EmptyPath,
    EmptyValueType {
        path: String,
    },
    ConflictingValueTypes {
        path: String,
        left: String,
        right: String,
    },
}

impl fmt::Display for PackageConfigRequirementMergeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("config requirement path must not be empty"),
            Self::EmptyValueType { path } => {
                write!(
                    formatter,
                    "config requirement {path} has an empty value type"
                )
            }
            Self::ConflictingValueTypes { path, left, right } => write!(
                formatter,
                "config requirement {path} has conflicting value types {left} and {right}"
            ),
        }
    }
}

impl std::error::Error for PackageConfigRequirementMergeError {}

pub fn canonicalize_package_config_requirements(
    requirements: impl IntoIterator<Item = PackageConfigRequirement>,
) -> Result<Vec<PackageConfigRequirement>, PackageConfigRequirementMergeError> {
    let mut canonical = BTreeMap::<String, PackageConfigAccess>::new();
    for requirement in requirements {
        if requirement.path.trim().is_empty() {
            return Err(PackageConfigRequirementMergeError::EmptyPath);
        }
        if requirement
            .access
            .value_type()
            .is_some_and(|value_type| value_type.trim().is_empty())
        {
            return Err(PackageConfigRequirementMergeError::EmptyValueType {
                path: requirement.path,
            });
        }
        let path = requirement.path;
        match canonical.remove(&path) {
            None => {
                canonical.insert(path, requirement.access);
            }
            Some(existing) => {
                canonical.insert(
                    path.clone(),
                    merge_config_access(&path, existing, requirement.access)?,
                );
            }
        }
    }
    Ok(canonical
        .into_iter()
        .map(|(path, access)| PackageConfigRequirement { path, access })
        .collect())
}

fn merge_config_access(
    path: &str,
    left: PackageConfigAccess,
    right: PackageConfigAccess,
) -> Result<PackageConfigAccess, PackageConfigRequirementMergeError> {
    use PackageConfigAccess::{Optional, Presence, Required};

    match (left, right) {
        (Presence, access) | (access, Presence) => Ok(access),
        (
            Optional {
                value_type: left_type,
            },
            Optional {
                value_type: right_type,
            },
        ) if left_type == right_type => Ok(Optional {
            value_type: left_type,
        }),
        (
            Required {
                value_type: left_type,
            },
            Required {
                value_type: right_type,
            },
        ) if left_type == right_type => Ok(Required {
            value_type: left_type,
        }),
        (
            Optional {
                value_type: left_type,
            },
            Required {
                value_type: right_type,
            },
        )
        | (
            Required {
                value_type: right_type,
            },
            Optional {
                value_type: left_type,
            },
        ) if left_type == right_type => Ok(Required {
            value_type: right_type,
        }),
        (left, right) => Err(PackageConfigRequirementMergeError::ConflictingValueTypes {
            path: path.to_string(),
            left: left.value_type().expect("presence was handled").to_string(),
            right: right
                .value_type()
                .expect("presence was handled")
                .to_string(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRuntimeRequirements {
    pub config: Vec<PackageConfigRequirement>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn service_call_ref_rejects_provider_and_missing_contract_identity() {
        let complete = json!({
            "serviceRequirementSlot": 0,
            "contractOperationId": "operation",
            "expectedProtocolIdentity": "protocol"
        });
        serde_json::from_value::<ServiceCallRef>(complete.clone()).unwrap();

        let mut missing = complete.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("expectedProtocolIdentity");
        assert!(serde_json::from_value::<ServiceCallRef>(missing).is_err());

        let mut provider = complete;
        provider
            .as_object_mut()
            .unwrap()
            .insert("providerBuildId".to_string(), json!("forbidden"));
        assert!(serde_json::from_value::<ServiceCallRef>(provider).is_err());
    }

    #[test]
    fn package_runtime_requirements_reject_retired_state_resource_and_capability_fields() {
        let canonical = json!({
            "config": []
        });
        serde_json::from_value::<PackageRuntimeRequirements>(canonical.clone()).unwrap();

        for field in ["state", "resources", "runtimeCapabilities"] {
            let mut retired = canonical.clone();
            retired[field] = json!([]);
            assert!(
                serde_json::from_value::<PackageRuntimeRequirements>(retired).is_err(),
                "{field} unexpectedly survived the package requirement hard cut"
            );
        }
    }

    #[test]
    fn config_access_wire_is_single_tagged_owner() {
        let presence = PackageConfigRequirement {
            path: "provider".to_string(),
            access: PackageConfigAccess::Presence,
        };
        assert_eq!(
            serde_json::to_value(&presence).unwrap(),
            json!({ "path": "provider", "access": { "kind": "presence" } })
        );
        let required = PackageConfigRequirement {
            path: "provider.apiKey".to_string(),
            access: PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
        };
        assert_eq!(
            serde_json::to_value(&required).unwrap(),
            json!({
                "path": "provider.apiKey",
                "access": { "kind": "required", "valueType": "string" }
            })
        );

        let legacy = json!({
            "path": "provider.apiKey",
            "valueType": "string",
            "required": true
        });
        assert!(serde_json::from_value::<PackageConfigRequirement>(legacy).is_err());
    }

    #[test]
    fn config_access_canonicalization_merges_strength_and_rejects_type_conflicts() {
        let requirements = canonicalize_package_config_requirements([
            PackageConfigRequirement {
                path: "z".to_string(),
                access: PackageConfigAccess::Presence,
            },
            PackageConfigRequirement {
                path: "a".to_string(),
                access: PackageConfigAccess::Optional {
                    value_type: "string".to_string(),
                },
            },
            PackageConfigRequirement {
                path: "a".to_string(),
                access: PackageConfigAccess::Presence,
            },
            PackageConfigRequirement {
                path: "a".to_string(),
                access: PackageConfigAccess::Required {
                    value_type: "string".to_string(),
                },
            },
        ])
        .unwrap();
        assert_eq!(
            requirements,
            vec![
                PackageConfigRequirement {
                    path: "a".to_string(),
                    access: PackageConfigAccess::Required {
                        value_type: "string".to_string(),
                    },
                },
                PackageConfigRequirement {
                    path: "z".to_string(),
                    access: PackageConfigAccess::Presence,
                },
            ]
        );

        let conflict = canonicalize_package_config_requirements([
            PackageConfigRequirement {
                path: "a".to_string(),
                access: PackageConfigAccess::Optional {
                    value_type: "string".to_string(),
                },
            },
            PackageConfigRequirement {
                path: "a".to_string(),
                access: PackageConfigAccess::Required {
                    value_type: "number".to_string(),
                },
            },
        ])
        .unwrap_err();
        assert!(matches!(
            conflict,
            PackageConfigRequirementMergeError::ConflictingValueTypes { ref path, .. }
                if path == "a"
        ));
    }
}
