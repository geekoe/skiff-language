use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    boundary::{BoundaryCallableProjection, CallableSemanticFacts},
    compile_identity::{
        PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageSchemaTypeId,
    },
    compile_requirements::{
        ContractRequirement, PackageRequirement, PackageRuntimeRequirements, ServiceCallRef,
        ServiceRequirement,
    },
    contract_types::{PackageSchemaIndexRef, PackageSchemaTypeRecordRef, PackageTypeRef},
    executable_target::OperationTargetRef,
    package_unit::{InterfaceMethodSignature, PackageImplementationLinks},
    refs::FileIrRef,
    resources::PublicationResourceRef,
    types::{TypeDescriptorIr, TypeRefIr},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableParameter {
    pub name: String,
    pub ty: PackageTypeRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableSignature {
    pub parameters: Vec<PackageCallableParameter>,
    pub return_type: PackageTypeRef,
    pub may_suspend: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PackageLocalAbiSymbol {
    Type {
        local_type_id: String,
        descriptor: TypeDescriptorIr,
        /// True only for a transparent source `alias` declaration. Nominal
        /// representation declarations also use an alias-shaped descriptor,
        /// but remain real package types.
        is_alias: bool,
        #[serde(default)]
        is_interface: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        type_params: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        interface_methods: Vec<InterfaceMethodSignature>,
    },
    Callable {
        callable_id: PackageCallableId,
        signature: PackageCallableSignature,
    },
    Constant {
        const_id: String,
        ty: PackageTypeRef,
    },
    PublicInstance {
        instance_id: String,
        declared_receiver_type: TypeRefIr,
        interfaces: Vec<TypeRefIr>,
        methods: BTreeMap<String, PackageCallableId>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageLocalAbi {
    pub local_abi_identity: PackageLocalAbiIdentity,
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    /// Exact implementation source addresses available only to test-service
    /// dependencies declared with `access: topLevel`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCallableLinkFact {
    pub callable_id: PackageCallableId,
    pub target: OperationTargetRef,
}

/// Canonical user-code artifact. No PublicationAbiUnit, PackageUnit or
/// ServiceUnit is embedded; the legacy DTOs remain separate runtime adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifact {
    pub schema_version: String,
    pub package_id: String,
    pub package_version: String,
    pub package_build_id: PackageBuildId,
    pub files: Vec<FileIrRef>,
    pub static_resources: Vec<PublicationResourceRef>,
    pub package_local_abi: PackageLocalAbi,
    pub package_schema_index: PackageSchemaIndexRef,
    pub package_schema_type_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
    pub implementation_links: PackageImplementationLinks,
    pub callable_links: BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    pub package_requirements: Vec<PackageRequirement>,
    pub contract_requirements: Vec<ContractRequirement>,
    pub service_requirements: Vec<ServiceRequirement>,
    pub runtime_requirements: PackageRuntimeRequirements,
    pub callable_semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    pub boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
    pub service_call_refs: Vec<ServiceCallRef>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn package_schema_type_reference_is_not_a_legacy_abi_type() {
        let ty = PackageTypeRef::PackageSchema {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "User".to_string(),
            package_schema_type_id: "skiff-package-schema-type-v1:sha256:abc".into(),
        };
        assert_eq!(
            serde_json::to_value(ty).unwrap(),
            json!({
                "kind": "packageSchema",
                "packageId": "example.pkg",
                "stableSchemaKey": "User",
                "packageSchemaTypeId": "skiff-package-schema-type-v1:sha256:abc"
            })
        );
    }

    #[test]
    fn package_callable_signature_rejects_closed_throw_set_field() {
        let canonical = json!({
            "parameters": [],
            "returnType": {
                "kind": "local",
                "localType": { "kind": "builtin", "name": "void" }
            },
            "maySuspend": false
        });
        serde_json::from_value::<PackageCallableSignature>(canonical.clone()).unwrap();

        let mut legacy = canonical;
        legacy["throwTypes"] = json!([]);
        assert!(serde_json::from_value::<PackageCallableSignature>(legacy).is_err());
    }

    #[test]
    fn any_interface_wire_preserves_exact_nested_package_identity() {
        let ty = PackageTypeRef::Nullable {
            inner: Box::new(PackageTypeRef::Container {
                name: "Array".to_string(),
                arguments: vec![PackageTypeRef::AnyInterface {
                    interface: Box::new(PackageTypeRef::PackageSchema {
                        package_id: "example.llm-api".to_string(),
                        stable_schema_key: "LlmClient".to_string(),
                        package_schema_type_id: "skiff-package-schema-type-v1:sha256:client".into(),
                    }),
                    arguments: Vec::new(),
                }],
            }),
        };
        let wire = serde_json::to_value(&ty).unwrap();
        assert_eq!(
            wire,
            json!({
                "kind": "nullable",
                "inner": {
                    "kind": "container",
                    "name": "Array",
                    "arguments": [{
                        "kind": "anyInterface",
                        "interface": {
                            "kind": "packageSchema",
                            "packageId": "example.llm-api",
                            "stableSchemaKey": "LlmClient",
                            "packageSchemaTypeId":
                                "skiff-package-schema-type-v1:sha256:client"
                        },
                        "arguments": []
                    }]
                }
            })
        );
        assert_eq!(serde_json::from_value::<PackageTypeRef>(wire).unwrap(), ty);
    }

    #[test]
    fn any_interface_wire_rejects_missing_or_opaque_interface_target() {
        let missing = json!({ "kind": "anyInterface" });
        assert!(serde_json::from_value::<PackageTypeRef>(missing).is_err());

        let unknown = json!({
            "kind": "anyInterface",
            "interface": {
                "kind": "packageSchema",
                "packageId": "example.llm-api",
                "stableSchemaKey": "LlmClient",
                "packageSchemaTypeId": "type:client",
                "displayName": "LlmClient"
            }
        });
        assert!(serde_json::from_value::<PackageTypeRef>(unknown).is_err());
    }

    #[test]
    fn package_artifact_wire_rejects_legacy_aggregate_fields() {
        let value = json!({
            "schemaVersion": "skiff-package-artifact-v5",
            "packageId": "example.pkg",
            "packageVersion": "1.0.0",
            "packageBuildId": "build",
            "files": [],
            "staticResources": [],
            "packageLocalAbi": { "localAbiIdentity": "abi", "publicSymbols": {} },
            "packageSchemaIndex": {
                "packageId": "example.pkg",
                "packageSchemaIndexIdentity": "index"
            },
            "packageSchemaTypeRecords": {},
            "implementationLinks": {},
            "callableLinks": {},
            "packageRequirements": [],
            "contractRequirements": [],
            "serviceRequirements": [],
            "runtimeRequirements": {
                "config": [],
                "state": [],
                "resources": [],
                "runtimeCapabilities": []
            },
            "callableSemanticFacts": {},
            "boundaryProjections": {},
            "serviceCallRefs": []
        });
        serde_json::from_value::<PackageArtifact>(value.clone())
            .expect("complete strict package artifact wire");

        for forbidden in ["publicationAbi", "packageUnit", "serviceUnit"] {
            let mut invalid = value.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_string(), json!({}));
            assert!(serde_json::from_value::<PackageArtifact>(invalid).is_err());
        }
    }
}
