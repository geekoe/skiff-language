use serde_json::json;

use super::*;

fn empty_statement_manifest_identity(package_id: &str) -> String {
    crate::derive_bytecode_statement_manifest_identity(package_id, &[])
        .unwrap()
        .to_string()
}

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
fn package_callable_parameter_mode_is_required_and_round_trips() {
    for mode in [crate::ParamModeIr::Value, crate::ParamModeIr::InOut] {
        let parameter = PackageCallableParameter {
            name: "input".to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            mode,
        };
        let wire = serde_json::to_value(&parameter).unwrap();
        assert_eq!(
            wire["mode"]["kind"],
            match mode {
                crate::ParamModeIr::Value => "value",
                crate::ParamModeIr::InOut => "inOut",
            }
        );
        assert_eq!(
            serde_json::from_value::<PackageCallableParameter>(wire.clone()).unwrap(),
            parameter
        );

        let mut missing = wire;
        missing.as_object_mut().unwrap().remove("mode");
        assert!(serde_json::from_value::<PackageCallableParameter>(missing).is_err());
    }
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
    let statement_manifest_identity = empty_statement_manifest_identity("example.pkg");
    let value = json!({
        "schemaVersion": "skiff-package-artifact-v14",
        "packageId": "example.pkg",
        "packageVersion": "1.0.0",
        "packageBuildId": "build",
        "files": [],
        "staticResources": [],
        "bytecodeStatementManifestIdentity": statement_manifest_identity,
        "packageLocalAbi": { "localAbiIdentity": "abi", "publicSymbols": {} },
        "packageSchemaIndex": {
            "packageId": "example.pkg",
            "packageSchemaIndexIdentity": "index"
        },
        "packageSchemaTypeRecords": {},
        "implementationLinks": {},
        "callableLinks": {},
        "syntheticCallbackOwners": [],
        "bytecodeSchemaRecords": {},
        "actorImplementations": [],
        "localInterfaceConformances": [],
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

    for field in [
        "bytecodeStatementManifestIdentity",
        "syntheticCallbackOwners",
        "bytecodeSchemaRecords",
        "actorImplementations",
        "localInterfaceConformances",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<PackageArtifact>(missing).is_err(),
            "missing {field} must fail closed"
        );
    }

    let mut malformed_manifest_identity = value.clone();
    malformed_manifest_identity["bytecodeStatementManifestIdentity"] = json!("manifest:fake");
    assert!(serde_json::from_value::<PackageArtifact>(malformed_manifest_identity).is_err());

    let decoded = serde_json::from_value::<PackageArtifact>(value.clone()).unwrap();
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded, value);
    assert!(encoded.get("serviceCallRoots").is_none());
    assert_eq!(encoded["serviceCallRefs"], json!([]));
}

#[test]
fn package_build_authority_rows_are_required_strict_and_canonical() {
    let actor = json!({ "modulePath": "module", "symbol": "Worker" });
    let actor_row = json!({
        "actor": actor,
        "actorImplementationIdentity": "actor-impl:worker",
        "methods": {
            "actor-method:read": "pkg-callable:worker.read"
        },
        "create": null
    });
    let conformance = json!({
        "typeParameters": [],
        "receiver": { "kind": "builtin", "name": "string" },
        "interface": {
            "interfaceAbiId": "interface:reader"
        },
        "methods": ["pkg-callable:worker.read"]
    });

    let decoded_actor: PackageActorImplementation =
        serde_json::from_value(actor_row.clone()).expect("explicit null create is required-valid");
    assert!(decoded_actor.create.is_none());
    assert_eq!(serde_json::to_value(decoded_actor).unwrap(), actor_row);
    let decoded_conformance: PackageLocalInterfaceConformance =
        serde_json::from_value(conformance.clone()).expect("exact conformance row");
    assert_eq!(
        serde_json::to_value(decoded_conformance).unwrap(),
        conformance
    );

    let mut missing_create = actor_row.clone();
    missing_create.as_object_mut().unwrap().remove("create");
    assert!(serde_json::from_value::<PackageActorImplementation>(missing_create).is_err());

    for (mut invalid, field) in [
        (actor_row.clone(), "providerBuildId"),
        (conformance.clone(), "functionKey"),
    ] {
        invalid[field] = json!("forbidden");
        assert!(
            if field == "providerBuildId" {
                serde_json::from_value::<PackageActorImplementation>(invalid).is_err()
            } else {
                serde_json::from_value::<PackageLocalInterfaceConformance>(invalid).is_err()
            },
            "{field} must not enter build-owned authority rows"
        );
    }
}

#[test]
fn package_build_authority_vectors_reject_duplicate_or_noncanonical_rows() {
    let base_actor = |module_path: &str, symbol: &str, identity: &str| {
        json!({
            "actor": { "modulePath": module_path, "symbol": symbol },
            "actorImplementationIdentity": identity,
            "methods": {},
            "create": null
        })
    };
    for rows in [
        vec![
            base_actor("z", "Worker", "impl:z"),
            base_actor("a", "Worker", "impl:a"),
        ],
        vec![
            base_actor("module", "Worker", "impl:first"),
            base_actor("module", "Worker", "impl:second"),
        ],
    ] {
        let wire = serde_json::to_value(rows).unwrap();
        let statement_manifest_identity = empty_statement_manifest_identity("example.pkg");
        let wrapper = json!({
            "schemaVersion": "skiff-package-artifact-v14",
            "packageId": "example.pkg",
            "packageVersion": "1.0.0",
            "packageBuildId": "build",
            "files": [],
            "staticResources": [],
            "bytecodeStatementManifestIdentity": statement_manifest_identity,
            "packageLocalAbi": { "localAbiIdentity": "abi", "publicSymbols": {} },
            "packageSchemaIndex": {
                "packageId": "example.pkg",
                "packageSchemaIndexIdentity": "index"
            },
            "packageSchemaTypeRecords": {},
            "implementationLinks": {},
            "callableLinks": {},
            "syntheticCallbackOwners": [],
            "bytecodeSchemaRecords": {},
            "actorImplementations": wire,
            "localInterfaceConformances": [],
            "packageRequirements": [],
            "contractRequirements": [],
            "serviceRequirements": [],
            "runtimeRequirements": { "config": [] },
            "callableSemanticFacts": {},
            "boundaryProjections": {},
            "serviceCallRefs": []
        });
        assert!(serde_json::from_value::<PackageArtifact>(wrapper).is_err());
    }
}

#[test]
fn package_local_conformances_reject_duplicate_or_noncanonical_rows() {
    let conformance = |interface_abi_id: &str| {
        json!({
            "typeParameters": [],
            "receiver": { "kind": "builtin", "name": "string" },
            "interface": { "interfaceAbiId": interface_abi_id },
            "methods": []
        })
    };
    let statement_manifest_identity = empty_statement_manifest_identity("example.pkg");
    let base = json!({
        "schemaVersion": "skiff-package-artifact-v14",
        "packageId": "example.pkg",
        "packageVersion": "1.0.0",
        "packageBuildId": "build",
        "files": [],
        "staticResources": [],
        "bytecodeStatementManifestIdentity": statement_manifest_identity,
        "packageLocalAbi": { "localAbiIdentity": "abi", "publicSymbols": {} },
        "packageSchemaIndex": {
            "packageId": "example.pkg",
            "packageSchemaIndexIdentity": "index"
        },
        "packageSchemaTypeRecords": {},
        "implementationLinks": {},
        "callableLinks": {},
        "syntheticCallbackOwners": [],
        "bytecodeSchemaRecords": {},
        "actorImplementations": [],
        "localInterfaceConformances": [],
        "packageRequirements": [],
        "contractRequirements": [],
        "serviceRequirements": [],
        "runtimeRequirements": { "config": [] },
        "callableSemanticFacts": {},
        "boundaryProjections": {},
        "serviceCallRefs": []
    });
    for rows in [
        vec![conformance("interface:z"), conformance("interface:a")],
        vec![conformance("interface:a"), conformance("interface:a")],
    ] {
        let mut wire = base.clone();
        wire["localInterfaceConformances"] = json!(rows);
        assert!(serde_json::from_value::<PackageArtifact>(wire).is_err());
    }

    let mut canonical = base;
    canonical["localInterfaceConformances"] =
        json!([conformance("interface:a"), conformance("interface:z")]);
    let decoded: PackageArtifact = serde_json::from_value(canonical.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), canonical);
}

mod authority;
