use std::collections::BTreeMap;

use skiff_artifact_identity::validate_service_contract_identities;
use skiff_artifact_model::{
    ContractDiscriminatedUnionBranch, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape,
};

use super::{
    compile_service_contract_definition, definition_contract_type_ref, definition_fixture,
    ContractDefinitionError,
};

#[test]
fn definition_normalizes_nullable_union_and_builtin_spelling_once() {
    let mut nullable = definition_fixture();
    payload_field_mut(&mut nullable, "message").clone_from(&ContractTypeRef::Nullable {
        inner: Box::new(ContractTypeRef::StructuralUnion {
            variants: vec![
                ContractTypeRef::builtin("bool"),
                ContractTypeRef::builtin("string"),
            ],
        }),
    });

    let mut union = definition_fixture();
    payload_field_mut(&mut union, "message").clone_from(&ContractTypeRef::StructuralUnion {
        variants: vec![
            ContractTypeRef::builtin("null"),
            ContractTypeRef::StructuralUnion {
                variants: vec![
                    ContractTypeRef::builtin("String"),
                    ContractTypeRef::builtin("boolean"),
                ],
            },
        ],
    });

    let nullable = compile_service_contract_definition(nullable).unwrap();
    let union = compile_service_contract_definition(union).unwrap();
    assert_eq!(nullable, union);
    assert_eq!(
        nullable.service_protocol_identity,
        union.service_protocol_identity
    );
}

#[test]
fn transparent_aliases_are_expanded_and_excluded_from_protocol_identity() {
    let mut direct_definition = definition_fixture();
    payload_fields_mut(&mut direct_definition).insert(
        "lookup".to_string(),
        ContractTypeRef::Builtin {
            name: "Map".to_string(),
            arguments: vec![
                ContractTypeRef::builtin("string"),
                ContractTypeRef::builtin("string"),
            ],
        },
    );
    let direct = compile_service_contract_definition(direct_definition.clone()).unwrap();

    let mut aliased = direct_definition;
    let alias_ref = definition_contract_type_ref(
        &aliased.service_id,
        &aliased.contract_version,
        "messageAlias",
    )
    .unwrap();
    aliased.boundary_schema.insert(
        "messageAlias".to_string(),
        shape(ContractTypeDescriptor::Alias {
            target: ContractTypeRef::builtin("string"),
        }),
    );
    *payload_field_mut(&mut aliased, "message") = alias_ref.clone();
    let ContractTypeRef::Builtin { arguments, .. } = payload_field_mut(&mut aliased, "lookup")
    else {
        panic!("lookup map")
    };
    arguments[0] = alias_ref;

    let aliased = compile_service_contract_definition(aliased).unwrap();
    assert_eq!(aliased, direct);
    assert!(aliased.boundary_schema.values().all(|schema| !matches!(
        schema.shape.descriptor,
        ContractTypeDescriptor::Alias { .. }
    )));
}

#[test]
fn transparent_alias_cycle_fails_before_contract_identity_derivation() {
    let mut definition = definition_fixture();
    let left = definition_contract_type_ref(
        &definition.service_id,
        &definition.contract_version,
        "aliasLeft",
    )
    .unwrap();
    let right = definition_contract_type_ref(
        &definition.service_id,
        &definition.contract_version,
        "aliasRight",
    )
    .unwrap();
    definition.boundary_schema.extend([
        (
            "aliasLeft".to_string(),
            shape(ContractTypeDescriptor::Alias { target: right }),
        ),
        (
            "aliasRight".to_string(),
            shape(ContractTypeDescriptor::Alias { target: left }),
        ),
    ]);
    assert_definition_error_contains(
        definition,
        &["boundarySchema[alias", "transparent alias cycle"],
    );
}

#[test]
fn code_free_definition_emits_discriminator_literal_representation_and_map() {
    let mut definition = definition_fixture();
    let service_id = definition.service_id.clone();
    let version = definition.contract_version.clone();
    let user_key = definition_contract_type_ref(&service_id, &version, "userKey").unwrap();
    let created = definition_contract_type_ref(&service_id, &version, "created").unwrap();
    let deleted = definition_contract_type_ref(&service_id, &version, "deleted").unwrap();
    let event = definition_contract_type_ref(&service_id, &version, "event").unwrap();

    definition.boundary_schema.extend([
        (
            "userKey".to_string(),
            shape(ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin("String"),
            }),
        ),
        ("created".to_string(), record_shape("created", "payload")),
        ("deleted".to_string(), record_shape("deleted", "reason")),
        (
            "event".to_string(),
            shape(ContractTypeDescriptor::DiscriminatedUnion {
                discriminator_field: "kind".to_string(),
                branches: vec![
                    ContractDiscriminatedUnionBranch::new("deleted", deleted),
                    ContractDiscriminatedUnionBranch::new("created", created),
                ],
            }),
        ),
    ]);
    let payload = payload_fields_mut(&mut definition);
    payload.insert(
        "eventsByUser".to_string(),
        ContractTypeRef::Builtin {
            name: "std.collection.Map".to_string(),
            arguments: vec![user_key, event],
        },
    );

    let mut reversed = definition.clone();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } = &mut reversed
        .boundary_schema
        .get_mut("event")
        .unwrap()
        .descriptor
    else {
        panic!("event discriminated union")
    };
    branches.reverse();

    let contract = compile_service_contract_definition(definition).unwrap();
    let reordered = compile_service_contract_definition(reversed).unwrap();
    assert_eq!(reordered, contract);
    validate_service_contract_identities(&contract).unwrap();
    let event = contract
        .boundary_schema
        .values()
        .find(|schema| schema.stable_key == "event")
        .unwrap();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } = &event.shape.descriptor
    else {
        panic!("event discriminated union")
    };
    assert_eq!(
        branches
            .iter()
            .map(|branch| branch.tag.as_str())
            .collect::<Vec<_>>(),
        vec!["created", "deleted"]
    );

    let round_trip = serde_json::from_value::<skiff_artifact_model::ServiceContract>(
        serde_json::to_value(&contract).unwrap(),
    )
    .unwrap();
    assert_eq!(round_trip, contract);
}

#[test]
fn definition_rejects_duplicate_discriminator_and_invalid_map_key_before_identity() {
    let mut duplicate = definition_fixture();
    duplicate.boundary_schema.insert(
        "event".to_string(),
        shape(ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field: "kind".to_string(),
            branches: vec![
                ContractDiscriminatedUnionBranch::new(
                    "same",
                    ContractTypeRef::Record {
                        fields: BTreeMap::from([(
                            "kind".to_string(),
                            ContractTypeRef::string_literal("same"),
                        )]),
                    },
                ),
                ContractDiscriminatedUnionBranch::new(
                    "same",
                    ContractTypeRef::Record {
                        fields: BTreeMap::from([(
                            "kind".to_string(),
                            ContractTypeRef::string_literal("same"),
                        )]),
                    },
                ),
            ],
        }),
    );
    assert_definition_error_contains(duplicate, &["boundarySchema[event]", "duplicate"]);

    let mut invalid_map = definition_fixture();
    payload_fields_mut(&mut invalid_map).insert(
        "badMap".to_string(),
        ContractTypeRef::Builtin {
            name: "Map".to_string(),
            arguments: vec![
                ContractTypeRef::builtin("number"),
                ContractTypeRef::builtin("string"),
            ],
        },
    );
    assert_definition_error_contains(
        invalid_map,
        &[
            "fields[badMap].arguments[0]",
            "Map key must be exact string",
        ],
    );
}

fn record_shape(tag: &str, payload_field: &str) -> ContractTypeShape {
    shape(ContractTypeDescriptor::Record {
        fields: BTreeMap::from([
            ("kind".to_string(), ContractTypeRef::string_literal(tag)),
            (
                payload_field.to_string(),
                ContractTypeRef::builtin("string"),
            ),
        ]),
    })
}

fn shape(descriptor: ContractTypeDescriptor) -> ContractTypeShape {
    ContractTypeShape {
        nameability: ContractTypeNameability::PublicNameable,
        descriptor,
    }
}

fn payload_fields_mut(
    definition: &mut super::ServiceContractDefinition,
) -> &mut BTreeMap<String, ContractTypeRef> {
    let ContractTypeDescriptor::Record { fields } = &mut definition
        .boundary_schema
        .get_mut("payload")
        .unwrap()
        .descriptor
    else {
        panic!("payload record")
    };
    fields
}

fn payload_field_mut<'a>(
    definition: &'a mut super::ServiceContractDefinition,
    name: &str,
) -> &'a mut ContractTypeRef {
    payload_fields_mut(definition).get_mut(name).unwrap()
}

fn assert_definition_error_contains(
    definition: super::ServiceContractDefinition,
    needles: &[&str],
) {
    let error = compile_service_contract_definition(definition).unwrap_err();
    assert!(
        matches!(
            &error,
            ContractDefinitionError::Identity(
                skiff_artifact_identity::ArtifactIdentityError::InvalidServiceContract { .. }
            )
        ),
        "semantic schema errors remain typed identity validation errors"
    );
    let error = error.to_string();
    for needle in needles {
        assert!(
            error.contains(needle),
            "expected `{needle}` in definition error: {error}"
        );
    }
}
