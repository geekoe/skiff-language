use std::collections::BTreeMap;

use skiff_artifact_model::{
    ContractDiscriminatedUnionBranch, ContractSchemaType, ContractTypeDescriptor, ContractTypeId,
    ContractTypeNameability, ContractTypeRef, ContractTypeShape, ServiceContract,
};

use super::*;

#[test]
fn discriminator_map_and_representation_facts_are_protocol_identity_inputs() {
    let base = schema_fidelity_fixture();
    let baseline = service_protocol_identity(&base).unwrap();

    let mut field_changed = base.clone();
    let ContractTypeDescriptor::DiscriminatedUnion {
        discriminator_field: field,
        ..
    } = descriptor_mut(&mut field_changed, "event")
    else {
        panic!("event discriminated union")
    };
    *field = "type".to_string();
    for stable_key in ["created", "createdAlternate", "deleted"] {
        let ContractTypeDescriptor::Record { fields } =
            descriptor_mut(&mut field_changed, stable_key)
        else {
            panic!("branch record")
        };
        let literal = fields.remove("kind").unwrap();
        fields.insert("type".to_string(), literal);
    }
    assert_ne!(service_protocol_identity(&field_changed).unwrap(), baseline);

    let mut branch_tag = base.clone();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } =
        descriptor_mut(&mut branch_tag, "event")
    else {
        panic!("event discriminated union")
    };
    branches[0].tag = "added".to_string();
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut branch_tag, "created")
    else {
        panic!("created record")
    };
    fields.insert("kind".to_string(), ContractTypeRef::string_literal("added"));
    assert_ne!(service_protocol_identity(&branch_tag).unwrap(), baseline);

    let mut branch_type = base.clone();
    let alternate_id = type_id(&branch_type, "createdAlternate");
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } =
        descriptor_mut(&mut branch_type, "event")
    else {
        panic!("event discriminated union")
    };
    branches[0].branch_type = ContractTypeRef::contract(alternate_id);
    assert_ne!(service_protocol_identity(&branch_type).unwrap(), baseline);

    let mut map_key_identity = base.clone();
    let other_key_id = type_id(&map_key_identity, "otherUserKey");
    *payload_map_key_mut(&mut map_key_identity) = ContractTypeRef::contract(other_key_id);
    assert_ne!(
        service_protocol_identity(&map_key_identity).unwrap(),
        baseline
    );

    let mut representation_target = base.clone();
    let ContractTypeDescriptor::Representation { target } =
        descriptor_mut(&mut representation_target, "sequence")
    else {
        panic!("sequence representation")
    };
    *target = ContractTypeRef::builtin("bool");
    assert_ne!(
        service_protocol_identity(&representation_target).unwrap(),
        baseline
    );
}

#[test]
fn materialized_contract_rejects_noncanonical_type_and_branch_order() {
    let mut nullable_union = contract_fixture();
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut nullable_union, "payload")
    else {
        panic!("payload record")
    };
    fields.insert(
        "message".to_string(),
        ContractTypeRef::StructuralUnion {
            variants: vec![
                ContractTypeRef::builtin("null"),
                ContractTypeRef::builtin("string"),
            ],
        },
    );
    assert_invalid_contains(
        &nullable_union,
        &["boundarySchema[payload]", "not in canonical"],
    );

    let mut builtin_alias = contract_fixture();
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut builtin_alias, "payload")
    else {
        panic!("payload record")
    };
    fields.insert("message".to_string(), ContractTypeRef::builtin("boolean"));
    assert_invalid_contains(
        &builtin_alias,
        &["boundarySchema[payload]", "not in canonical"],
    );

    let mut branch_order = schema_fidelity_fixture();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } =
        descriptor_mut(&mut branch_order, "event")
    else {
        panic!("event discriminated union")
    };
    branches.reverse();
    assert_invalid_contains(
        &branch_order,
        &["boundarySchema[event]", "not in canonical"],
    );
}

#[test]
fn builtin_map_and_discriminator_grammar_fail_closed_with_paths() {
    let mut unknown_builtin = contract_fixture();
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut unknown_builtin, "payload")
    else {
        panic!("payload record")
    };
    fields.insert("message".to_string(), ContractTypeRef::builtin("NewNative"));
    assert_invalid_contains(
        &unknown_builtin,
        &[
            "boundarySchema[payload]",
            "unknown contract builtin `NewNative`",
        ],
    );

    let mut wrong_map_arity = contract_fixture();
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut wrong_map_arity, "payload")
    else {
        panic!("payload record")
    };
    fields.insert(
        "lookup".to_string(),
        ContractTypeRef::Builtin {
            name: "Map".to_string(),
            arguments: vec![ContractTypeRef::builtin("string")],
        },
    );
    assert_invalid_contains(&wrong_map_arity, &["fields[lookup]", "expects 2 arguments"]);

    let mut primitive_map_key = contract_fixture();
    let ContractTypeDescriptor::Record { fields } =
        descriptor_mut(&mut primitive_map_key, "payload")
    else {
        panic!("payload record")
    };
    fields.insert(
        "lookup".to_string(),
        map_type(
            ContractTypeRef::builtin("bool"),
            ContractTypeRef::builtin("string"),
        ),
    );
    assert_invalid_contains(
        &primitive_map_key,
        &[
            "fields[lookup].arguments[0]",
            "Map key must be exact string",
        ],
    );

    let mut alias_map_key = contract_fixture();
    let alias_id = insert_schema(
        &mut alias_map_key,
        "stringAlias",
        shape(ContractTypeDescriptor::Alias {
            target: ContractTypeRef::builtin("string"),
        }),
    );
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut alias_map_key, "payload")
    else {
        panic!("payload record")
    };
    fields.insert(
        "lookup".to_string(),
        map_type(
            ContractTypeRef::contract(alias_id),
            ContractTypeRef::builtin("string"),
        ),
    );
    assert_invalid_contains(
        &alias_map_key,
        &["fields[lookup].arguments[0]", "transparent alias"],
    );

    let mut non_string_representation_key = schema_fidelity_fixture();
    let sequence_id = type_id(&non_string_representation_key, "sequence");
    *payload_map_key_mut(&mut non_string_representation_key) =
        ContractTypeRef::contract(sequence_id);
    assert_invalid_contains(
        &non_string_representation_key,
        &["arguments[0]", "must target exact string"],
    );

    let mut duplicate_tag = schema_fidelity_fixture();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } =
        descriptor_mut(&mut duplicate_tag, "event")
    else {
        panic!("event discriminated union")
    };
    let first_tag = branches[0].tag.clone();
    branches[1].tag = first_tag;
    assert_invalid_contains(&duplicate_tag, &["branches[created]", "duplicate"]);

    let mut empty_tag = schema_fidelity_fixture();
    let ContractTypeDescriptor::DiscriminatedUnion { branches, .. } =
        descriptor_mut(&mut empty_tag, "event")
    else {
        panic!("event discriminated union")
    };
    branches[0].tag.clear();
    assert_invalid_contains(&empty_tag, &["branches[0].tag", "must not be empty"]);

    let mut missing_tag_field = schema_fidelity_fixture();
    let ContractTypeDescriptor::Record { fields } =
        descriptor_mut(&mut missing_tag_field, "created")
    else {
        panic!("created record")
    };
    fields.remove("kind");
    assert_invalid_contains(
        &missing_tag_field,
        &["branches[created].fields[kind]", "missing"],
    );
}

#[test]
fn user_schema_cycles_fail_closed_while_compiler_json_terminals_remain_closed() {
    let mut alias_cycle = contract_fixture();
    let left_id = contract_type_id("example.echo", "1.0.0", "aliasLeft").unwrap();
    let right_id = contract_type_id("example.echo", "1.0.0", "aliasRight").unwrap();
    insert_schema_with_id(
        &mut alias_cycle,
        "aliasLeft",
        left_id.clone(),
        shape(ContractTypeDescriptor::Alias {
            target: ContractTypeRef::contract(right_id.clone()),
        }),
    );
    insert_schema_with_id(
        &mut alias_cycle,
        "aliasRight",
        right_id,
        shape(ContractTypeDescriptor::Alias {
            target: ContractTypeRef::contract(left_id),
        }),
    );
    assert_invalid_contains(&alias_cycle, &["boundarySchema[alias", "transparent alias"]);

    let mut union_cycle = contract_fixture();
    let left_id = contract_type_id("example.echo", "1.0.0", "unionLeft").unwrap();
    let right_id = contract_type_id("example.echo", "1.0.0", "unionRight").unwrap();
    insert_schema_with_id(
        &mut union_cycle,
        "unionLeft",
        left_id.clone(),
        shape(ContractTypeDescriptor::StructuralUnion {
            variants: vec![
                ContractTypeRef::contract(right_id.clone()),
                ContractTypeRef::builtin("string"),
            ],
        }),
    );
    insert_schema_with_id(
        &mut union_cycle,
        "unionRight",
        right_id,
        shape(ContractTypeDescriptor::StructuralUnion {
            variants: vec![
                ContractTypeRef::contract(left_id),
                ContractTypeRef::builtin("number"),
            ],
        }),
    );
    assert_invalid_contains(&union_cycle, &["boundarySchema[union", "recursive"]);

    let mut record_cycle = contract_fixture();
    let record_id = contract_type_id("example.echo", "1.0.0", "recursiveRecord").unwrap();
    insert_schema_with_id(
        &mut record_cycle,
        "recursiveRecord",
        record_id.clone(),
        shape(ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("next".to_string(), ContractTypeRef::contract(record_id))]),
        }),
    );
    assert_invalid_contains(
        &record_cycle,
        &["boundarySchema[recursiveRecord]", "recursive"],
    );

    let mut json_terminal = contract_fixture();
    insert_schema(
        &mut json_terminal,
        "jsonEnvelope",
        shape(ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("json".to_string(), ContractTypeRef::builtin("Json")),
                ("object".to_string(), ContractTypeRef::builtin("JsonObject")),
            ]),
        }),
    );
    service_protocol_identity(&json_terminal).unwrap();
}

fn schema_fidelity_fixture() -> ServiceContract {
    let mut contract = contract_fixture();
    let service_id = contract.service_id.clone();
    let version = contract.contract_version.clone();
    let ids = [
        "userKey",
        "otherUserKey",
        "sequence",
        "created",
        "createdAlternate",
        "deleted",
        "event",
    ]
    .into_iter()
    .map(|key| (key, contract_type_id(&service_id, &version, key).unwrap()))
    .collect::<BTreeMap<_, _>>();

    for key in ["userKey", "otherUserKey"] {
        insert_schema_with_id(
            &mut contract,
            key,
            ids[key].clone(),
            shape(ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin("string"),
            }),
        );
    }
    insert_schema_with_id(
        &mut contract,
        "sequence",
        ids["sequence"].clone(),
        shape(ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin("number"),
        }),
    );
    for (key, tag, extra_field) in [
        ("created", "created", "payload"),
        ("createdAlternate", "created", "alternate"),
        ("deleted", "deleted", "reason"),
    ] {
        insert_schema_with_id(
            &mut contract,
            key,
            ids[key].clone(),
            shape(ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("kind".to_string(), ContractTypeRef::string_literal(tag)),
                    (extra_field.to_string(), ContractTypeRef::builtin("string")),
                ]),
            }),
        );
    }
    insert_schema_with_id(
        &mut contract,
        "event",
        ids["event"].clone(),
        shape(ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field: "kind".to_string(),
            branches: vec![
                ContractDiscriminatedUnionBranch::new(
                    "created",
                    ContractTypeRef::contract(ids["created"].clone()),
                ),
                ContractDiscriminatedUnionBranch::new(
                    "deleted",
                    ContractTypeRef::contract(ids["deleted"].clone()),
                ),
            ],
        }),
    );
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(&mut contract, "payload") else {
        panic!("payload record")
    };
    fields.insert(
        "byUser".to_string(),
        map_type(
            ContractTypeRef::contract(ids["userKey"].clone()),
            ContractTypeRef::contract(ids["event"].clone()),
        ),
    );
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

fn map_type(key: ContractTypeRef, value: ContractTypeRef) -> ContractTypeRef {
    ContractTypeRef::Builtin {
        name: "Map".to_string(),
        arguments: vec![key, value],
    }
}

fn shape(descriptor: ContractTypeDescriptor) -> ContractTypeShape {
    ContractTypeShape {
        nameability: ContractTypeNameability::PublicNameable,
        descriptor,
    }
}

fn insert_schema(
    contract: &mut ServiceContract,
    stable_key: &str,
    shape: ContractTypeShape,
) -> ContractTypeId {
    let type_id =
        contract_type_id(&contract.service_id, &contract.contract_version, stable_key).unwrap();
    insert_schema_with_id(contract, stable_key, type_id.clone(), shape);
    type_id
}

fn insert_schema_with_id(
    contract: &mut ServiceContract,
    stable_key: &str,
    type_id: ContractTypeId,
    shape: ContractTypeShape,
) {
    let shape =
        normalize_contract_type_shape(shape, &format!("boundarySchema[{stable_key}].shape"))
            .unwrap();
    contract.boundary_schema.insert(
        type_id.clone(),
        ContractSchemaType {
            contract_type_id: type_id,
            stable_key: stable_key.to_string(),
            shape,
        },
    );
}

fn descriptor_mut<'a>(
    contract: &'a mut ServiceContract,
    stable_key: &str,
) -> &'a mut ContractTypeDescriptor {
    &mut contract
        .boundary_schema
        .values_mut()
        .find(|schema| schema.stable_key == stable_key)
        .unwrap()
        .shape
        .descriptor
}

fn type_id(contract: &ServiceContract, stable_key: &str) -> ContractTypeId {
    contract
        .boundary_schema
        .values()
        .find(|schema| schema.stable_key == stable_key)
        .unwrap()
        .contract_type_id
        .clone()
}

fn payload_map_key_mut(contract: &mut ServiceContract) -> &mut ContractTypeRef {
    let ContractTypeDescriptor::Record { fields } = descriptor_mut(contract, "payload") else {
        panic!("payload record")
    };
    let ContractTypeRef::Builtin { name, arguments } = fields.get_mut("byUser").unwrap() else {
        panic!("payload map")
    };
    assert_eq!(name, "Map");
    &mut arguments[0]
}

fn assert_invalid_contains(contract: &ServiceContract, needles: &[&str]) {
    let error = service_protocol_identity(contract).unwrap_err().to_string();
    for needle in needles {
        assert!(
            error.contains(needle),
            "expected `{needle}` in validation error: {error}"
        );
    }
}
