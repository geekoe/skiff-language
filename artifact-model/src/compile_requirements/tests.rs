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
