//! Golden wire-JSON schema snapshot: the canonical fixture's exact JSON must
//! stay byte-identical; schema mutations (field renames, removed fields,
//! unknown fields, version strings) must fail closed.

use super::*;

const GOLDEN_SNAPSHOT: &str = r#"{"magic":"skiff-bytecode","schemaVersion":"skiff-bytecode-v2","isaVersion":"skiff-bytecode-isa-v1","opcodeTableFingerprint":"3571e40bfccfa802ffa603356326a7699f2ec358561ff20bef7a7abfe1d6b322","bytecodeIdentity":"skiff-bytecode-image-v1:sha256:fixture","image":{"functions":{"module::helper":{"functionKey":"module::helper","words":[20,37],"frameLayout":{"slotCount":2,"slotTypeRefs":[0,1],"resultCount":0,"resultTypeRefs":[],"slotPlans":[{"kind":"snapshotShare"},{"kind":"moveOnly"}]},"maxOperandDepth":2,"effectSummaryRef":"operation:module:helper"},"module::main":{"functionKey":"module::main","words":[0,0,3,0,17,0,32,0,0,20,19,0,0,16,0,114,0,20,115,0,34,2,0,0,17,4294967276,37],"relocations":[{"kind":"localExecutableRef","functionKey":"module::helper"},{"kind":"interfaceRequirementRef","interfaceIdentity":"interface:reader"},{"kind":"serviceOperationRef","operationAbiId":"operation:svc:call"}],"frameLayout":{"slotCount":4,"slotTypeRefs":[0,0,1,1],"parameterSlots":[{"slot":0,"plan":{"kind":"snapshotShare"}}],"resultCount":1,"resultTypeRefs":[1],"resultPlans":[{"kind":"snapshotShare"}],"slotPlans":[{"kind":"snapshotShare"},{"kind":"snapshotShare"},{"kind":"moveOnly"},{"kind":"affineResource"}]},"maxOperandDepth":8,"effectSummaryRef":"operation:module:main","exceptionRegions":[{"startPc":15,"endPc":20,"handlerPc":26,"handlerStackHeight":0,"catchMatchers":[{"kind":"typeRef","typeRef":0}],"catchSlot":1,"cleanupDepth":0}],"switchTables":[{"tagPoolIndex":0,"targets":[4,13]}],"statementEntries":[{"pc":0,"statementId":"s:main:0"},{"pc":9,"statementId":"s:main:1"},{"pc":24,"statementId":"s:main:2"}],"sourceMap":[{"start":0,"end":6,"sourceId":0,"startPosition":{"line":1,"column":1},"endPosition":{"line":3,"column":1}},{"start":7,"end":27,"sourceId":0,"startPosition":{"line":3,"column":1},"endPosition":{"line":9,"column":1}}]}},"pools":{"constants":[{"kind":"frozenConstantRef","nodeIndex":0}],"types":[{"kind":"typeRef","ty":{"kind":"builtin","name":"string"}},{"kind":"typeRef","ty":{"kind":"builtin","name":"number"}}],"shapes":[{"kind":"shapeRef","shape":{"fieldCount":1,"fieldTypes":[0]}}],"effects":[{"kind":"hostEffectRef","effectRef":"effect:llm"}],"resume":[{"kind":"resumeDescriptor","resultTypeRef":1,"expectedStackHeight":2,"resultPlan":{"kind":"snapshotShare"}}],"callbackCapture":[{"kind":"callbackCaptureLayout","functionKey":"module::helper","captures":[{"slot":0,"plan":{"kind":"snapshotShare"}},{"slot":1,"plan":{"kind":"moveOnly"}}]}]},"frozenConstantGraph":{"nodes":[{"kind":"literal","literal":{"kind":"number","value":42}},{"kind":"array","children":[0]},{"kind":"record","shapeIndex":0,"children":[0]},{"kind":"typeRef","typeRef":0},{"kind":"behavior","functionKey":"module::helper"}]},"debugTable":{"bindings":[{"functionKey":"module::main","pc":0,"name":"x","slot":0},{"functionKey":"module::helper","pc":0,"name":"y","slot":1}]}}}"#;

/// The canonical fixture serializes to exactly the approved golden wire JSON.
/// Any schema/field change breaks this test until the golden is updated and
/// reviewed.
#[test]
fn canonical_fixture_matches_golden_snapshot() {
    let actual = serde_json::to_string(&canonical_artifact()).expect("serialize fixture");
    let golden = GOLDEN_SNAPSHOT.trim();
    if actual != golden {
        panic!(
            "canonical fixture no longer matches the golden schema snapshot\n\
             --- golden ---\n{golden}\n\
             --- actual ---\n{actual}"
        );
    }
}

/// Unknown fields are rejected everywhere (deny_unknown_fields).
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

/// Required header fields cannot be dropped or renamed.
#[test]
fn schema_rejects_missing_or_renamed_header_fields() {
    let value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    let mut missing_magic = value.clone();
    missing_magic.as_object_mut().unwrap().remove("magic");
    let error = serde_json::from_value::<BytecodeArtifact>(missing_magic)
        .expect_err("missing magic must fail closed")
        .to_string();
    assert!(error.contains("missing field `magic`"), "{error}");

    let mut renamed = value.clone();
    renamed.as_object_mut().unwrap().remove("magic");
    renamed["magik"] = serde_json::json!("skiff-bytecode");
    let error = serde_json::from_value::<BytecodeArtifact>(renamed)
        .expect_err("renamed magic must fail closed")
        .to_string();
    assert!(
        error.contains("missing field `magic`") || error.contains("unknown field `magik`"),
        "unexpected renamed magic error: {error}"
    );
}

/// Typed slot and result arrays are required even when empty; missing fields
/// cannot silently deserialize to an untyped frame.
#[test]
fn schema_rejects_missing_frame_type_refs() {
    let value = serde_json::to_value(canonical_artifact()).expect("fixture JSON");
    for field in ["slotTypeRefs", "resultTypeRefs"] {
        let mut missing = value.clone();
        missing["image"]["functions"]["module::main"]["frameLayout"]
            .as_object_mut()
            .expect("frame layout object")
            .remove(field);
        let error = serde_json::from_value::<BytecodeArtifact>(missing)
            .expect_err("missing frame type refs must fail closed")
            .to_string();
        assert!(error.contains(&format!("missing field `{field}`")), "{error}");
    }
}

/// Tagged enums reject unknown variants (no silent forward-compat).
#[test]
fn schema_rejects_unknown_enum_variants() {
    let mut relocation = serde_json::to_value(BytecodeRelocation::LocalExecutableRef {
        function_key: "module::helper".to_string(),
    })
    .expect("relocation JSON");
    relocation["kind"] = serde_json::json!("interfaceMethodRef");
    let error = serde_json::from_value::<BytecodeRelocation>(relocation)
        .expect_err("unknown relocation variant must fail closed")
        .to_string();
    assert!(error.contains("unknown variant"), "{error}");

    let mut pool_entry = serde_json::to_value(BytecodePoolEntry::TypeRef { ty: string_type() })
        .expect("pool entry JSON");
    pool_entry["kind"] = serde_json::json!("legacyConstant");
    let error = serde_json::from_value::<BytecodePoolEntry>(pool_entry)
        .expect_err("unknown pool entry variant must fail closed")
        .to_string();
    assert!(error.contains("unknown variant"), "{error}");
}

/// Version strings and the fingerprint are part of the wire contract.
#[test]
fn version_constants_are_stable() {
    assert_eq!(BYTECODE_MAGIC, "skiff-bytecode");
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v2");
    assert_eq!(BYTECODE_ISA_VERSION, "skiff-bytecode-isa-v1");
    assert_eq!(
        crate::bytecode::dto::BYTECODE_SCHEMA_VERSION,
        BYTECODE_SCHEMA_VERSION
    );
}
