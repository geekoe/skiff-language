use serde_json::json;

use super::*;

#[test]
fn callback_operation_excludes_provider_suspension_and_rejects_legacy_wire() {
    let operation = BoundaryCallbackOperation {
        parameters: vec![ContractTypeRef::builtin("string")],
        return_type: ContractTypeRef::builtin("void"),
    };
    let wire = serde_json::to_value(&operation).unwrap();
    assert!(wire.get("maySuspend").is_none());
    assert_eq!(
        serde_json::from_value::<BoundaryCallbackOperation>(wire.clone()).unwrap(),
        operation
    );

    let mut legacy = wire;
    legacy["maySuspend"] = json!(true);
    assert!(serde_json::from_value::<BoundaryCallbackOperation>(legacy).is_err());
}

#[test]
fn contract_any_interface_wire_preserves_exact_nominal_target() {
    let ty = ContractTypeRef::AnyInterface {
        interface: Box::new(ContractTypeRef::package_schema(
            "example.llm-api",
            "LlmClient",
            PackageSchemaTypeId::new("type:llm-client"),
        )),
        arguments: Vec::new(),
    };
    let wire = serde_json::to_value(&ty).unwrap();
    assert_eq!(
        wire,
        json!({
            "kind": "anyInterface",
            "interface": {
                "kind": "packageSchema",
                "packageId": "example.llm-api",
                "stableSchemaKey": "LlmClient",
                "packageSchemaTypeId": "type:llm-client"
            },
            "arguments": []
        })
    );
    assert_eq!(serde_json::from_value::<ContractTypeRef>(wire).unwrap(), ty);
}

#[test]
fn package_schema_records_indexes_and_requirements_have_strict_wire() {
    let type_id = PackageSchemaTypeId::new("type:user");
    let record = PackageSchemaTypeRecord {
        package_id: "example.pkg".to_string(),
        stable_schema_key: "User".to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        },
    };
    let record_wire = serde_json::to_value(&record).unwrap();
    assert_eq!(record_wire["packageId"], "example.pkg");
    assert!(record_wire.get("nameability").is_none());
    assert!(record_wire.get("publicPath").is_none());

    let index = PackageSchemaIndex {
        package_id: "example.pkg".to_string(),
        package_schema_index_identity: "index".into(),
        types: BTreeMap::from([(
            "User".to_string(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id.clone(),
                public_path: Some("api.User".to_string()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        )]),
    };
    serde_json::from_value::<PackageSchemaIndex>(serde_json::to_value(index).unwrap())
        .expect("strict index round trip");

    let requirement = PackageTypeRequirement {
        package_id: "example.pkg".to_string(),
        required_type_ids: vec![type_id],
    };
    let wire = serde_json::to_value(requirement).unwrap();
    for field in ["packageId", "requiredTypeIds"] {
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<PackageTypeRequirement>(missing).is_err());
    }
    let mut extra = wire;
    extra
        .as_object_mut()
        .unwrap()
        .insert("packageSchemaIndexIdentity".to_string(), json!("forbidden"));
    assert!(serde_json::from_value::<PackageTypeRequirement>(extra).is_err());
}

#[test]
fn contract_type_ref_is_strict_and_nominal_id_is_explicit() {
    let reference = ContractTypeRef::package_schema(
        "example.pkg",
        "User",
        PackageSchemaTypeId::new("package-type"),
    );
    assert_eq!(
        serde_json::to_value(reference).unwrap(),
        json!({
            "kind": "packageSchema",
            "packageId": "example.pkg",
            "stableSchemaKey": "User",
            "packageSchemaTypeId": "package-type"
        })
    );
    for invalid in [
        json!({ "kind": "packageSchema" }),
        json!({ "kind": "packageSchema", "packageSchemaTypeId": "package-type" }),
        json!({
            "kind": "packageSchema",
            "packageId": "example.pkg",
            "stableSchemaKey": "User",
            "packageSchemaTypeId": "package-type",
            "displayName": "not semantic"
        }),
    ] {
        assert!(serde_json::from_value::<ContractTypeRef>(invalid).is_err());
    }
}

#[test]
fn v2_literal_and_structural_union_refs_have_strict_typed_wire() {
    let reference = ContractTypeRef::structural_union(vec![
        ContractTypeRef::string_literal("created"),
        ContractTypeRef::builtin("null"),
    ]);
    let wire = json!({
        "kind": "structuralUnion",
        "variants": [
            {
                "kind": "literal",
                "value": { "kind": "string", "value": "created" }
            },
            { "kind": "builtin", "name": "null", "arguments": [] }
        ]
    });
    assert_eq!(serde_json::to_value(&reference).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<ContractTypeRef>(wire.clone()).unwrap(),
        reference
    );

    for invalid in [
        json!({ "kind": "union", "variants": [] }),
        json!({ "kind": "structuralUnion" }),
        json!({
            "kind": "structuralUnion",
            "variants": [],
            "legacyDiscriminator": "kind"
        }),
        json!({ "kind": "literal" }),
        json!({ "kind": "literal", "value": { "kind": "string" } }),
        json!({
            "kind": "literal",
            "value": { "kind": "string", "value": "created", "extra": true }
        }),
    ] {
        assert!(
            serde_json::from_value::<ContractTypeRef>(invalid.clone()).is_err(),
            "non-v2 or incomplete ref must fail closed: {invalid}"
        );
    }
}

#[test]
fn v2_discriminated_union_descriptor_round_trips_strict_branch_entries() {
    let descriptor = ContractTypeDescriptor::DiscriminatedUnion {
        discriminator_field: "kind".to_string(),
        branches: vec![ContractDiscriminatedUnionBranch::new(
            "created",
            ContractTypeRef::Record {
                fields: BTreeMap::from([(
                    "kind".to_string(),
                    ContractTypeRef::string_literal("created"),
                )]),
            },
        )],
    };
    let wire = json!({
        "kind": "discriminatedUnion",
        "discriminatorField": "kind",
        "branches": [{
            "tag": "created",
            "branchType": {
                "kind": "record",
                "fields": {
                    "kind": {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "created" }
                    }
                }
            }
        }]
    });
    assert_eq!(serde_json::to_value(&descriptor).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<ContractTypeDescriptor>(wire).unwrap(),
        descriptor
    );

    for invalid in [
        json!({ "kind": "discriminatedUnion", "branches": [] }),
        json!({ "kind": "discriminatedUnion", "discriminatorField": "kind" }),
        json!({
            "kind": "discriminatedUnion",
            "discriminatorField": "kind",
            "branches": [{
                "branchType": { "kind": "builtin", "name": "string", "arguments": [] }
            }]
        }),
        json!({
            "kind": "discriminatedUnion",
            "discriminatorField": "kind",
            "branches": [{ "tag": "created" }]
        }),
        json!({
            "kind": "discriminatedUnion",
            "discriminatorField": "kind",
            "branches": [{
                "tag": "created",
                "branchType": { "kind": "builtin", "name": "string", "arguments": [] },
                "legacyBranchId": "branch"
            }]
        }),
    ] {
        assert!(
            serde_json::from_value::<ContractTypeDescriptor>(invalid.clone()).is_err(),
            "incomplete discriminated union must fail closed: {invalid}"
        );
    }
}

#[test]
fn v2_structural_union_and_representation_descriptors_reject_legacy_shapes() {
    let structural = ContractTypeDescriptor::StructuralUnion {
        variants: vec![ContractTypeRef::builtin("string")],
    };
    let representation = ContractTypeDescriptor::Representation {
        target: ContractTypeRef::builtin("string"),
    };
    for descriptor in [&structural, &representation] {
        let wire = serde_json::to_value(descriptor).unwrap();
        assert_eq!(
            serde_json::from_value::<ContractTypeDescriptor>(wire).unwrap(),
            *descriptor
        );
    }
    assert_eq!(
        serde_json::to_value(&structural).unwrap(),
        json!({
            "kind": "structuralUnion",
            "variants": [{ "kind": "builtin", "name": "string", "arguments": [] }]
        })
    );
    assert_eq!(
        serde_json::to_value(&representation).unwrap(),
        json!({
            "kind": "representation",
            "target": { "kind": "builtin", "name": "string", "arguments": [] }
        })
    );

    for invalid in [
        json!({ "kind": "union", "variants": [] }),
        json!({ "kind": "structuralUnion" }),
        json!({ "kind": "representation" }),
        json!({
            "kind": "representation",
            "target": { "kind": "builtin", "name": "string", "arguments": [] },
            "transparent": true
        }),
    ] {
        assert!(
            serde_json::from_value::<ContractTypeDescriptor>(invalid.clone()).is_err(),
            "legacy or incomplete descriptor must fail closed: {invalid}"
        );
    }
}

#[test]
fn generic_shape_and_type_parameter_have_strict_wire() {
    let shape = ContractTypeShape {
        nameability: ContractTypeNameability::PublicNameable,
        type_params: vec!["T".to_string()],
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([(
                "value".to_string(),
                ContractTypeRef::TypeParam {
                    name: "T".to_string(),
                },
            )]),
        },
    };
    let wire = json!({
        "nameability": "publicNameable",
        "typeParams": ["T"],
        "descriptor": {
            "kind": "record",
            "fields": {
                "value": { "kind": "typeParam", "name": "T" }
            }
        }
    });
    assert_eq!(serde_json::to_value(&shape).unwrap(), wire);
    assert_eq!(
        serde_json::from_value::<ContractTypeShape>(wire.clone()).unwrap(),
        shape
    );

    for invalid in [
        json!({
            "nameability": "publicNameable",
            "descriptor": wire["descriptor"].clone()
        }),
        json!({
            "nameability": "publicNameable",
            "typeParams": ["T"],
            "descriptor": {
                "kind": "record",
                "fields": { "value": { "kind": "typeParam" } }
            }
        }),
        json!({
            "nameability": "publicNameable",
            "typeParams": ["T"],
            "descriptor": wire["descriptor"].clone(),
            "displayType": "Box<T>"
        }),
    ] {
        assert!(serde_json::from_value::<ContractTypeShape>(invalid).is_err());
    }
}
