use serde_json::json;

use super::*;

#[test]
fn actor_declaration_wire_preserves_field_layout_and_encoding() {
    let wire = json!({
        "actorName": "DocHub",
        "actorIdType": { "kind": "builtin", "name": "string" },
        "fields": [
            {
                "name": "nextSeq",
                "ty": { "kind": "builtin", "name": "number" },
                "encoding": "canonicalValueV1"
            }
        ],
        "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
    });
    let abi: ActorAbiInput = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(abi.fields[0].name, "nextSeq");
    assert_eq!(serde_json::to_value(abi).unwrap(), wire);
}

#[test]
fn actor_abi_wire_rejects_actor_ref_and_noncanonical_shape() {
    let actor_ref = json!({
        "actorName": "DocHub",
        "actorIdType": {
            "kind": "builtin",
            "name": "ActorRef",
            "args": [{ "kind": "builtin", "name": "string" }]
        },
        "fields": [],
        "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
    });
    assert!(serde_json::from_value::<ActorAbiInput>(actor_ref)
        .unwrap_err()
        .to_string()
        .contains("legacy ActorRef"));

    let nested_actor_ref = json!({
        "actorName": "DocHub",
        "actorIdType": {
            "kind": "appliedNominal",
            "base": { "kind": "localType", "typeIndex": 0 },
            "arguments": [{
                "kind": "builtin",
                "name": "ActorRef",
                "args": [{ "kind": "builtin", "name": "string" }]
            }]
        },
        "fields": [],
        "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
    });
    assert!(serde_json::from_value::<ActorAbiInput>(nested_actor_ref)
        .unwrap_err()
        .to_string()
        .contains("legacy ActorRef"));

    let applied_package_schema = json!({
        "actorName": "DocHub",
        "actorIdType": {
            "kind": "appliedNominal",
            "base": {
                "kind": "packageSchema",
                "packageId": "example.model",
                "stableSchemaKey": "ActorId",
                "packageSchemaTypeId": "schema:actor-id"
            },
            "arguments": [{ "kind": "builtin", "name": "string" }]
        },
        "fields": [],
        "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
    });
    assert!(
        serde_json::from_value::<ActorAbiInput>(applied_package_schema)
            .unwrap_err()
            .to_string()
            .contains("not admitted")
    );

    let duplicate_fields = json!({
        "actorName": "DocHub",
        "actorIdType": { "kind": "builtin", "name": "string" },
        "fields": [
            {
                "name": "value",
                "ty": { "kind": "builtin", "name": "string" },
                "encoding": "canonicalValueV1"
            },
            {
                "name": "value",
                "ty": { "kind": "builtin", "name": "number" },
                "encoding": "canonicalValueV1"
            }
        ],
        "actorRuntimeAbiVersion": ACTOR_RUNTIME_ABI_VERSION_V1
    });
    assert!(serde_json::from_value::<ActorAbiInput>(duplicate_fields)
        .unwrap_err()
        .to_string()
        .contains("duplicate actor field"));
}

#[test]
fn actor_declaration_requires_exact_public_method_implementation_map() {
    let method = ActorMethodIdentity::new("skiff-actor-method-v1:sha256:append");
    let declaration = ActorDeclarationIr {
        actor_abi_identity: ActorAbiIdentity::new("actor-abi"),
        actor_implementation_identity: ActorImplementationIdentity::new("actor-impl"),
        abi: ActorAbiInput {
            actor_name: "DocHub".to_string(),
            actor_id_type: TypeRefIr::builtin("string"),
            fields: Vec::new(),
            public_methods: vec![ActorPublicMethodIr {
                method_identity: method.clone(),
                name: "append".to_string(),
                parameters: Vec::new(),
                return_type: TypeRefIr::builtin("void"),
                may_suspend: false,
            }],
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        },
        method_implementations: BTreeMap::from([(method, 0)]),
    };
    let wire = serde_json::to_value(declaration).unwrap();
    assert!(serde_json::from_value::<ActorDeclarationIr>(wire.clone()).is_ok());

    let mut missing = wire.clone();
    missing["methodImplementations"] = json!({});
    assert!(serde_json::from_value::<ActorDeclarationIr>(missing)
        .unwrap_err()
        .to_string()
        .contains("must match"));

    let mut orphan = wire;
    orphan["methodImplementations"] = json!({"orphan": 1});
    assert!(serde_json::from_value::<ActorDeclarationIr>(orphan)
        .unwrap_err()
        .to_string()
        .contains("must match"));
}
