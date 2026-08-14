//! Frozen v10 wire-shape snapshot and fail-closed serde tests.

use super::*;

/// Compact golden projection: it freezes every v10 seam that downstream
/// emission/linking consumes without duplicating the canonical fixture's
/// large literal payload.
const GOLDEN_V10_SHAPE: &str = r#"{
  "artifact":["bytecodeIdentity","hostEffectRegistry","image","intrinsicRegistry","isaVersion","magic","nativeValueLifecycleRegistry","opcodeTableFingerprint","platformErrorProjectionRegistry","schemaVersion","valueLifecyclePolicy"],
  "image":["constantRoots","debugTable","frozenConstantGraph","functions","pools"],
  "function":["activeRegions","callLoanLayouts","effectSummaryRef","exceptionRegions","frameLayout","functionKey","maxOperandDepth","origin","relocations","selfTypeRef","sourceMap","statementEntries","switchTables","words"],
  "frame":["parameterSlots","resultCount","resultPlans","resultTypeRefs","slotCount","slotPlans","slotTypeRefs","streamResultTypeRef","writableLocalSlots"],
  "callLoanLayout":["loans"],
  "callLoanBinding":["parameterOrdinal","rootSlot","writablePathRef"],
  "localRelocation":["functionKey","kind","specialization"],
  "specialization":["concreteReceiver","typeArguments"],
  "statementEntry":["attributionId","pc","sequenceOrdinal","site"],
  "statementAttributionId":["expressionIndex","kind","occurrenceOrdinal"],
  "constantEntry":["kind","plan","reference","typeRef"],
  "shape":["fields","plan","typeRef"],
  "resume":["endResumePc","errorMode","expectedStackHeightBeforeResult","functionKey","kind","resultMaterializations","resultPlans","resultTypeRefs","resumePc","sitePc"]
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
fn canonical_fixture_matches_v10_wire_shape_snapshot() {
    let value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    let main = &value["image"]["functions"]["module::main"];
    let projection = serde_json::json!({
        "artifact": sorted_keys(&value),
        "image": sorted_keys(&value["image"]),
        "function": sorted_keys(main),
        "frame": sorted_keys(&main["frameLayout"]),
        "callLoanLayout": sorted_keys(&main["callLoanLayouts"][0]),
        "callLoanBinding": sorted_keys(&main["callLoanLayouts"][0]["loans"][0]),
        "localRelocation": sorted_keys(&main["relocations"][0]),
        "specialization": sorted_keys(&main["relocations"][0]["specialization"]),
        "statementEntry": sorted_keys(&main["statementEntries"][1]),
        "statementAttributionId": sorted_keys(&main["statementEntries"][1]["attributionId"]),
        "constantEntry": sorted_keys(&value["image"]["pools"]["constants"][0]),
        "shape": sorted_keys(&value["image"]["pools"]["shapes"][0]["shape"]),
        "resume": sorted_keys(&value["image"]["pools"]["resume"][0]),
    });
    let golden: serde_json::Value = serde_json::from_str(GOLDEN_V10_SHAPE).expect("golden JSON");
    assert_eq!(projection, golden);

    assert_eq!(value["schemaVersion"], "skiff-bytecode-v10");
    assert_eq!(value["isaVersion"], "skiff-bytecode-isa-v5");
    assert_eq!(
        value["nativeValueLifecycleRegistry"]["fingerprint"],
        crate::NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT
    );
    assert_eq!(
        value["valueLifecyclePolicy"]["fingerprint"],
        crate::VALUE_LIFECYCLE_POLICY_FINGERPRINT
    );
    assert_eq!(
        value["hostEffectRegistry"],
        serde_json::to_value(crate::host_effect_registry_identity()).unwrap()
    );
    assert_eq!(
        value["intrinsicRegistry"],
        serde_json::to_value(crate::intrinsic_registry_identity()).unwrap()
    );
    assert_eq!(
        value["platformErrorProjectionRegistry"],
        serde_json::to_value(crate::current_platform_error_projection_registry_ref()).unwrap()
    );
    assert_eq!(main["origin"]["kind"], "executable");
    assert!(main["selfTypeRef"].is_null());
    assert_eq!(
        main["frameLayout"]["writableLocalSlots"],
        serde_json::json!([1])
    );
    assert_eq!(main["callLoanLayouts"][0]["loans"][0]["rootSlot"], 1);
    assert_eq!(
        value["image"]["pools"]["shapes"][0]["shape"]["plan"],
        serde_json::json!({
            "kind": "snapshotShare",
            "drop": { "kind": "snapshotRelease" }
        })
    );
    assert_eq!(
        value["image"]["functions"]["module::helper"]["callLoanLayouts"],
        serde_json::json!([])
    );
    assert_eq!(
        value["image"]["functions"]["module::helper"]["frameLayout"]["writableLocalSlots"],
        serde_json::json!([])
    );
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
    assert_eq!(value["image"]["constantRoots"]["module.implementation"], 2);
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
    let decoded: BytecodeArtifact =
        serde_json::from_value(value.clone()).expect("explicit null selfTypeRef remains valid");
    assert!(decoded.image.functions["module::main"]
        .self_type_ref
        .is_none());
    for field in [
        "magic",
        "schemaVersion",
        "isaVersion",
        "opcodeTableFingerprint",
        "nativeValueLifecycleRegistry",
        "valueLifecyclePolicy",
        "hostEffectRegistry",
        "intrinsicRegistry",
        "platformErrorProjectionRegistry",
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

    let mut missing_shape_plan = value.clone();
    missing_shape_plan["image"]["pools"]["shapes"][0]["shape"]
        .as_object_mut()
        .unwrap()
        .remove("plan");
    let error = serde_json::from_value::<BytecodeArtifact>(missing_shape_plan)
        .expect_err("shape plan must be required on the wire")
        .to_string();
    assert!(error.contains("missing field"), "{error}");

    for field in [
        "slotTypeRefs",
        "writableLocalSlots",
        "resultTypeRefs",
        "streamResultTypeRef",
    ] {
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

    let mut missing_end_resume_pc = value.clone();
    missing_end_resume_pc["image"]["pools"]["resume"][0]
        .as_object_mut()
        .unwrap()
        .remove("endResumePc");
    let error = serde_json::from_value::<BytecodeArtifact>(missing_end_resume_pc)
        .expect_err("endResumePc must be required on the wire")
        .to_string();
    assert!(error.contains("missing field"), "{error}");

    let mut missing_result_materializations = value.clone();
    missing_result_materializations["image"]["pools"]["resume"][0]
        .as_object_mut()
        .unwrap()
        .remove("resultMaterializations");
    let error = serde_json::from_value::<BytecodeArtifact>(missing_result_materializations)
        .expect_err("resultMaterializations must be required on the wire")
        .to_string();
    assert!(error.contains("missing field"), "{error}");

    for field in ["origin", "selfTypeRef", "callLoanLayouts"] {
        let mut missing = value.clone();
        missing["image"]["functions"]["module::main"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        let error = serde_json::from_value::<BytecodeArtifact>(missing)
            .expect_err("function manifest field must be required")
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
fn schema_requires_complete_call_loan_rows() {
    let layout = CallLoanLayout {
        loans: vec![CallLoanBinding {
            parameter_ordinal: 1,
            root_slot: 2,
            writable_path_ref: 3,
        }],
    };
    let value = serde_json::to_value(&layout).unwrap();
    let mut missing_loans = value.clone();
    missing_loans.as_object_mut().unwrap().remove("loans");
    assert!(serde_json::from_value::<CallLoanLayout>(missing_loans).is_err());

    for field in ["parameterOrdinal", "rootSlot", "writablePathRef"] {
        let mut missing = value.clone();
        missing["loans"][0].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<CallLoanLayout>(missing).is_err());
    }

    assert_eq!(
        serde_json::from_value::<CallLoanLayout>(value).unwrap(),
        layout
    );
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
fn dense_resume_result_materialization_round_trips_as_an_exact_shape_ref() {
    let materialization = ResumeResultMaterialization::DenseRecord { shape_ref: 7 };
    let value = serde_json::to_value(materialization).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"kind": "denseRecord", "shapeRef": 7})
    );
    assert_eq!(
        serde_json::from_value::<ResumeResultMaterialization>(value).unwrap(),
        materialization
    );
}

#[test]
fn version_constants_freeze_schema_v10_and_isa_v5() {
    assert_eq!(BYTECODE_MAGIC, "skiff-bytecode");
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v10");
    assert_eq!(BYTECODE_ISA_VERSION, "skiff-bytecode-isa-v5");
}

#[test]
fn db_operation_reference_round_trips_camel_case_without_metadata_strings() {
    let effect = HostEffectReference {
        target: crate::NativeTarget {
            namespace: "std".to_string(),
            symbol: "db.operation".to_string(),
            binding_key: Some("std.db.operation".to_string()),
            metadata: std::collections::BTreeMap::new(),
        },
        signature: HostEffectSignature {
            parameter_types: vec![string_type()],
            parameter_modes: vec![crate::ParamModeIr::Value],
            parameter_plans: vec![snapshot_share()],
            result_types: vec![string_type()],
            result_plans: vec![snapshot_share()],
            effects: crate::CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![crate::PendingEffectCategory::HostEffect],
                inout_path_effects: Vec::new(),
            },
        },
        db_operation: Some(Box::new(crate::bytecode::dto::DbOperationReference {
            op: crate::bytecode::dto::DbOperationKind::Insert,
            target: crate::DbTargetIr {
                type_ref: string_type(),
                type_name: "fixture".to_string(),
            },
            operand_roles: vec![crate::bytecode::dto::DbOperandRole::ObjectFields],
            result_type: string_type(),
            result_plans: vec![snapshot_share()],
        })),
    };
    let value = serde_json::to_value(&effect).expect("serialize DB host effect");
    assert_eq!(value["dbOperation"]["op"], serde_json::json!("insert"));
    assert_eq!(value["dbOperation"]["target"]["typeName"], "fixture");
    assert_eq!(
        value["dbOperation"]["operandRoles"],
        serde_json::json!(["objectFields"])
    );
    assert_eq!(value["dbOperation"]["resultType"]["kind"], "builtin");
    assert_eq!(
        value["dbOperation"]["resultPlans"][0]["kind"],
        "snapshotShare"
    );
    let decoded: HostEffectReference =
        serde_json::from_value(value).expect("decode DB host effect");
    assert_eq!(decoded, effect);
}

#[test]
fn db_operation_wire_rejects_unsupported_ops_and_roles() {
    assert!(
        serde_json::from_value::<crate::bytecode::dto::DbOperationKind>(serde_json::json!(
            "update"
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<crate::bytecode::dto::DbOperandRole>(serde_json::json!("query"))
            .is_err()
    );
}

#[test]
fn std_db_operation_without_structured_reference_fails_closed() {
    let mut artifact = canonical_artifact();
    let BytecodePoolEntry::HostEffectRef(effect) = &mut artifact.image.pools.effects[0] else {
        unreachable!("canonical fixture has a host effect");
    };
    effect.target = crate::NativeTarget {
        namespace: "std".to_string(),
        symbol: "db.operation".to_string(),
        binding_key: Some("std.db.operation".to_string()),
        metadata: std::collections::BTreeMap::new(),
    };
    effect.db_operation = None;
    let error = structurally_validate(&artifact)
        .expect_err("std.db.operation must require a structured DB operation");
    assert!(error.to_string().contains("dbOperation"), "{error}");
}
