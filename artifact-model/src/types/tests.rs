use super::*;

#[test]
fn package_schema_type_ref_round_trips_exact_identity() {
    let expected = TypeRefIr::PackageSchema {
        package_id: "skiff.run/llm-api".to_string(),
        stable_schema_key: "LlmRequest".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("schema:request"),
    };
    let wire = serde_json::to_value(&expected).unwrap();
    assert_eq!(wire["kind"], "packageSchema");
    assert_eq!(wire["packageId"], "skiff.run/llm-api");
    assert_eq!(wire["stableSchemaKey"], "LlmRequest");
    assert_eq!(wire["packageSchemaTypeId"], "schema:request");
    assert_eq!(serde_json::from_value::<TypeRefIr>(wire).unwrap(), expected);
}

#[test]
fn package_schema_type_ref_rejects_missing_or_unknown_identity_fields() {
    for wire in [
        serde_json::json!({
            "kind": "packageSchema",
            "packageId": "skiff.run/llm-api",
            "stableSchemaKey": "LlmRequest"
        }),
        serde_json::json!({
            "kind": "packageSchema",
            "packageId": "skiff.run/llm-api",
            "stableSchemaKey": "LlmRequest",
            "packageSchemaTypeId": "schema:request",
            "abiTypeId": "wrong-domain"
        }),
    ] {
        assert!(serde_json::from_value::<TypeRefIr>(wire).is_err());
    }
}

#[test]
fn applied_nominal_has_one_strict_non_empty_wire_shape() {
    let expected = TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index: 2 },
        arguments: vec![TypeRefIr::builtin("string")],
    };
    let wire = serde_json::json!({
        "kind": "appliedNominal",
        "base": { "kind": "localType", "typeIndex": 2 },
        "arguments": [{ "kind": "builtin", "name": "string" }]
    });
    assert_eq!(serde_json::to_value(&expected).unwrap(), wire);
    assert_eq!(serde_json::from_value::<TypeRefIr>(wire).unwrap(), expected);

    for invalid in [
        serde_json::json!({
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 2 }
        }),
        serde_json::json!({
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 2 },
            "arguments": null
        }),
        serde_json::json!({
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 2 },
            "arguments": []
        }),
        serde_json::json!({
            "kind": "appliedNominal",
            "base": { "kind": "builtin", "name": "Array" },
            "arguments": [{ "kind": "builtin", "name": "string" }]
        }),
        serde_json::json!({
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 2, "name": "Box" },
            "arguments": [{ "kind": "builtin", "name": "string" }]
        }),
        serde_json::json!({
            "kind": "localType",
            "typeIndex": 2,
            "arguments": [{ "kind": "builtin", "name": "string" }]
        }),
    ] {
        assert!(
            serde_json::from_value::<TypeRefIr>(invalid.clone()).is_err(),
            "strict applied nominal wire must reject {invalid}"
        );
    }
}

#[test]
fn declaration_descriptors_distinguish_all_canonical_kinds() {
    let descriptors = [
        TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("string"),
        },
        TypeDescriptorIr::Union {
            branches: vec![NamedUnionBranchIr::Literal {
                value: LiteralIr::String {
                    value: "ready".to_string(),
                },
            }],
        },
        TypeDescriptorIr::Alias {
            target: TypeRefIr::builtin("string"),
        },
        TypeDescriptorIr::Interface,
    ];
    let expected_kinds = ["record", "representation", "union", "alias", "interface"];

    for (descriptor, expected_kind) in descriptors.into_iter().zip(expected_kinds) {
        let wire = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(wire["kind"], expected_kind);
        assert_eq!(
            serde_json::from_value::<TypeDescriptorIr>(wire).unwrap(),
            descriptor
        );
    }
}

#[test]
fn named_union_preserves_all_branch_identity_inputs() {
    let descriptor = TypeDescriptorIr::Union {
        branches: vec![
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::LocalType { type_index: 1 },
                    arguments: vec![TypeRefIr::builtin("string")],
                },
            },
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::Record {
                    fields: BTreeMap::new(),
                },
                discriminator_field: "kind".to_string(),
                discriminator_value: "retryable".to_string(),
            },
            NamedUnionBranchIr::Literal {
                value: LiteralIr::Bool { value: true },
            },
        ],
    };
    let wire = serde_json::to_value(&descriptor).unwrap();

    assert_eq!(
        wire["branches"][0]["nominalType"]["arguments"][0]["name"],
        "string"
    );
    assert!(wire["branches"][0].get("typeArguments").is_none());
    assert_eq!(wire["branches"][1]["discriminatorField"], "kind");
    assert_eq!(wire["branches"][2]["value"]["kind"], "bool");
    assert_eq!(
        serde_json::from_value::<TypeDescriptorIr>(wire).unwrap(),
        descriptor
    );

    assert!(
        serde_json::from_value::<NamedUnionBranchIr>(serde_json::json!({
            "kind": "concreteNominal",
            "nominalType": { "kind": "localType", "typeIndex": 1 },
            "typeArguments": {
                "T": { "kind": "builtin", "name": "string" }
            }
        }))
        .is_err()
    );
}
