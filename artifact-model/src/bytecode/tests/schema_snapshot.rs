//! Frozen v3 wire-shape snapshot and fail-closed serde tests.

use super::*;

/// Compact golden projection: it freezes every v3 seam that downstream
/// emission/linking consumes without duplicating the canonical fixture's
/// large literal payload.
const GOLDEN_V3_SHAPE: &str = r#"{
  "artifact":["bytecodeIdentity","image","isaVersion","magic","opcodeTableFingerprint","schemaVersion"],
  "image":["constantRoots","debugTable","frozenConstantGraph","functions","pools"],
  "function":["activeRegions","effectSummaryRef","exceptionRegions","frameLayout","functionKey","maxOperandDepth","relocations","sourceMap","statementEntries","switchTables","words"],
  "frame":["parameterSlots","resultCount","resultPlans","resultTypeRefs","slotCount","slotPlans","slotTypeRefs"],
  "localRelocation":["functionKey","kind","specialization"],
  "specialization":["concreteReceiver","typeArguments"],
  "constantEntry":["kind","plan","reference","typeRef"],
  "resume":["errorMode","expectedStackHeightBeforeResult","functionKey","kind","resultPlans","resultTypeRefs","resumePc","sitePc"]
}"#;

fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("snapshot node must be an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

#[test]
fn canonical_fixture_matches_v3_wire_shape_snapshot() {
    let value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    let main = &value["image"]["functions"]["module::main"];
    let projection = serde_json::json!({
        "artifact": sorted_keys(&value),
        "image": sorted_keys(&value["image"]),
        "function": sorted_keys(main),
        "frame": sorted_keys(&main["frameLayout"]),
        "localRelocation": sorted_keys(&main["relocations"][0]),
        "specialization": sorted_keys(&main["relocations"][0]["specialization"]),
        "constantEntry": sorted_keys(&value["image"]["pools"]["constants"][0]),
        "resume": sorted_keys(&value["image"]["pools"]["resume"][0]),
    });
    let golden: serde_json::Value = serde_json::from_str(GOLDEN_V3_SHAPE).expect("golden JSON");
    assert_eq!(projection, golden);

    assert_eq!(value["schemaVersion"], "skiff-bytecode-v3");
    assert_eq!(value["isaVersion"], "skiff-bytecode-isa-v3");
    assert_eq!(
        main["relocations"][0]["specialization"]["typeArguments"][0]["kind"],
        "builtin"
    );
    assert!(main["relocations"][0]["specialization"]["concreteReceiver"].is_null());
    assert_eq!(
        main["relocations"][2]["serviceCall"]["serviceRequirementSlot"],
        0
    );
    assert_eq!(
        main["relocations"][2]["serviceCall"]["contractOperationId"],
        "operation:svc:call"
    );
    assert_eq!(value["image"]["constantRoots"]["const:implementation"], 2);
    assert_eq!(
        value["image"]["pools"]["constants"][0]["reference"]["kind"],
        "localNode"
    );
    assert_eq!(
        value["image"]["pools"]["constants"][0]["plan"]["drop"]["kind"],
        "trivial"
    );
    assert_eq!(
        value["image"]["pools"]["shapes"][0]["shape"]["fields"][0]["name"],
        "value"
    );
    assert_eq!(
        value["image"]["frozenConstantGraph"]["nodes"][3]["kind"],
        "representation"
    );
    assert_eq!(
        value["image"]["frozenConstantGraph"]["nodes"][4]["kind"],
        "implementation"
    );
}

#[test]
fn schema_rejects_unknown_fields() {
    let mut value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    value["runtimeAddressTable"] = serde_json::json!([]);
    let error = serde_json::from_value::<BytecodeArtifact>(value)
        .expect_err("unknown artifact field must fail closed")
        .to_string();
    assert!(error.contains("unknown field"), "{error}");

    let mut pools = serde_json::to_value(canonical_pools()).expect("pools JSON");
    pools["legacyHeap"] = serde_json::json!([]);
    let error = serde_json::from_value::<BytecodePools>(pools)
        .expect_err("unknown pools field must fail closed")
        .to_string();
    assert!(error.contains("unknown field"), "{error}");
}

#[test]
fn schema_rejects_missing_required_header_image_and_frame_fields() {
    let value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    for field in [
        "magic",
        "schemaVersion",
        "isaVersion",
        "opcodeTableFingerprint",
        "bytecodeIdentity",
        "image",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        let error = serde_json::from_value::<BytecodeArtifact>(missing)
            .expect_err("required artifact field must fail closed")
            .to_string();
        assert!(error.contains("missing field"), "{field}: {error}");
    }

    let mut missing_roots = value.clone();
    missing_roots["image"]
        .as_object_mut()
        .unwrap()
        .remove("constantRoots");
    assert!(serde_json::from_value::<BytecodeArtifact>(missing_roots).is_err());

    for field in ["slotTypeRefs", "resultTypeRefs"] {
        let mut missing = value.clone();
        missing["image"]["functions"]["module::main"]["frameLayout"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        let error = serde_json::from_value::<BytecodeArtifact>(missing)
            .expect_err("typed frame field must be required")
            .to_string();
        assert!(error.contains("missing field"), "{field}: {error}");
    }
}

#[test]
fn schema_requires_specialization_and_its_positional_payload() {
    let relocation = BytecodeRelocation::LocalExecutableRef {
        function_key: "module::helper".to_string(),
        specialization: BytecodeSpecialization {
            type_arguments: vec![string_type()],
            concrete_receiver: None,
        },
    };
    let value = serde_json::to_value(&relocation).expect("relocation JSON");
    for path in ["specialization", "typeArguments", "concreteReceiver"] {
        let mut missing = value.clone();
        if path == "specialization" {
            missing.as_object_mut().unwrap().remove(path);
        } else {
            missing["specialization"]
                .as_object_mut()
                .unwrap()
                .remove(path);
        }
        assert!(
            serde_json::from_value::<BytecodeRelocation>(missing).is_err(),
            "{path} must be required"
        );
    }

    let decoded: BytecodeRelocation =
        serde_json::from_value(value).expect("explicit null concreteReceiver remains valid");
    assert_eq!(decoded, relocation);
}

#[test]
fn schema_rejects_unknown_tagged_enum_variants() {
    let mut relocation = serde_json::to_value(BytecodeRelocation::LocalExecutableRef {
        function_key: "module::helper".to_string(),
        specialization: BytecodeSpecialization {
            type_arguments: Vec::new(),
            concrete_receiver: None,
        },
    })
    .expect("relocation JSON");
    relocation["kind"] = serde_json::json!("interfaceMethodRef");
    let error = serde_json::from_value::<BytecodeRelocation>(relocation)
        .expect_err("unknown relocation variant must fail closed")
        .to_string();
    assert!(error.contains("unknown variant"), "{error}");

    let mut plan = serde_json::to_value(snapshot_share()).expect("plan JSON");
    plan["kind"] = serde_json::json!("copyEverything");
    assert!(serde_json::from_value::<ValueTransferPlan>(plan).is_err());
}

#[test]
fn version_constants_are_stable_at_v3() {
    assert_eq!(BYTECODE_MAGIC, "skiff-bytecode");
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v3");
    assert_eq!(BYTECODE_ISA_VERSION, "skiff-bytecode-isa-v3");
}
