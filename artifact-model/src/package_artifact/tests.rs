use serde_json::json;

use super::*;

#[test]
fn package_schema_type_reference_is_not_a_legacy_abi_type() {
    let ty = PackageTypeRef::PackageSchema {
        package_id: "example.pkg".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: "skiff-package-schema-type-v2:sha256:abc".into(),
    };
    assert_eq!(
        serde_json::to_value(ty).unwrap(),
        json!({
            "kind": "packageSchema",
            "packageId": "example.pkg",
            "stableSchemaKey": "User",
            "packageSchemaTypeId": "skiff-package-schema-type-v2:sha256:abc"
        })
    );
}

#[test]
fn package_callable_signature_rejects_closed_throw_set_field() {
    let canonical = json!({
        "typeParams": [],
        "parameters": [],
        "returnType": {
            "kind": "local",
            "localType": { "kind": "builtin", "name": "void" }
        },
        "maySuspend": false
    });
    serde_json::from_value::<PackageCallableSignature>(canonical.clone()).unwrap();

    let mut missing_scope = canonical.clone();
    missing_scope.as_object_mut().unwrap().remove("typeParams");
    assert!(serde_json::from_value::<PackageCallableSignature>(missing_scope).is_err());

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
                    package_schema_type_id: "skiff-package-schema-type-v2:sha256:client".into(),
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
                            "skiff-package-schema-type-v2:sha256:client"
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
        "schemaVersion": "skiff-package-artifact-v9",
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
            "config": []
        },
        "callableSemanticFacts": {},
        "boundaryProjections": {},
        "serviceCallRefs": []
    });
    serde_json::from_value::<PackageArtifact>(value.clone())
        .expect("complete strict package artifact wire");

    for field in ["state", "resources", "runtimeCapabilities"] {
        let mut retired = value.clone();
        retired["runtimeRequirements"][field] = json!([]);
        assert!(
            serde_json::from_value::<PackageArtifact>(retired).is_err(),
            "{field} unexpectedly survived the package artifact hard cut"
        );
    }

    for forbidden in ["publicationAbi", "packageUnit", "serviceUnit"] {
        let mut invalid = value.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!({}));
        assert!(serde_json::from_value::<PackageArtifact>(invalid).is_err());
    }

    let mut missing_build_identity = value.clone();
    missing_build_identity
        .as_object_mut()
        .unwrap()
        .remove("packageBuildId");
    assert!(serde_json::from_value::<PackageArtifact>(missing_build_identity).is_err());
    let mut missing_local_abi_identity = value.clone();
    missing_local_abi_identity["packageLocalAbi"]
        .as_object_mut()
        .unwrap()
        .remove("localAbiIdentity");
    assert!(serde_json::from_value::<PackageArtifact>(missing_local_abi_identity).is_err());

    let mut legacy_selection = value.clone();
    legacy_selection["serviceCallRoots"] = json!([
        {
            "kind": "function",
            "publicPath": "echo",
            "callableId": "pkg-callable:example.pkg:echo"
        },
        {
            "kind": "publicInstance",
            "publicPath": "worker",
            "methods": {
                "handle": "pkg-callable:example.pkg:worker.handle"
            }
        }
    ]);
    assert!(serde_json::from_value::<PackageArtifact>(legacy_selection).is_err());

    let mut missing_service_call_refs = value.clone();
    missing_service_call_refs
        .as_object_mut()
        .unwrap()
        .remove("serviceCallRefs");
    assert!(serde_json::from_value::<PackageArtifact>(missing_service_call_refs).is_err());

    let decoded = serde_json::from_value::<PackageArtifact>(value.clone()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded, value);
    assert!(encoded.get("serviceCallRoots").is_none());
    assert_eq!(encoded["serviceCallRefs"], json!([]));
}
