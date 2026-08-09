use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_bytecode_identity, BYTECODE_IDENTITY_PREFIX, BYTECODE_IDENTITY_SCHEMA_MARKER,
    FILE_IR_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    descriptor_for_opcode, BytecodeArtifact, BytecodeArtifactRef, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePools, FrameLayout, FrozenConstantGraph, Opcode, PackageCallableId,
    PackageExecutableCoordinate, RelocatableBytecodeFunction, StatementChargeKind, StatementEntry,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::*;

fn canonical_artifact() -> BytecodeArtifact {
    let function_key = "module::main".to_string();
    let return_ = u32::from(descriptor_for_opcode(Opcode::Return).opcode);
    let function = RelocatableBytecodeFunction {
        function_key: function_key.clone(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: PackageExecutableCoordinate {
                file_ir_identity: format!("{FILE_IR_IDENTITY_PREFIX}:{}", "a".repeat(64)),
                module_path: "module".to_string(),
                executable_index: 0,
            },
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![return_],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: Vec::new(),
        },
        max_operand_depth: 0,
        effect_summary_ref: PackageCallableId::new("operation:module:main"),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: vec![StatementEntry {
            pc: 0,
            statement_id: "statement:module:main:entry".to_string(),
            charge_kind: StatementChargeKind::FunctionEntry,
        }],
        source_map: Vec::new(),
    };
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        bytecode_identity: "identity-is-assigned-after-structural-validation".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::from([(function_key, function)]),
            pools: BytecodePools::default(),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    assign_bytecode_identity(&mut artifact).unwrap();
    artifact
}

#[test]
fn canonical_artifact_and_reference_produce_an_exact_receipt() {
    let artifact = canonical_artifact();
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    let handoff = BytecodeCompilationHandoff::try_new(artifact.clone(), reference.clone()).unwrap();
    let authorities = handoff.receipt().authorities();

    assert_eq!(handoff.artifact(), &artifact);
    assert_eq!(handoff.reference(), &reference);
    assert_eq!(handoff.reference().artifact_path, None);
    assert_eq!(
        handoff.receipt().bytecode_identity(),
        artifact.bytecode_identity
    );
    assert_eq!(handoff.receipt().schema_version(), BYTECODE_SCHEMA_VERSION);
    assert_eq!(handoff.receipt().isa_version(), BYTECODE_ISA_VERSION);
    assert_eq!(
        handoff.receipt().opcode_table_fingerprint(),
        artifact.opcode_table_fingerprint
    );
    assert_eq!(
        authorities.native_value_lifecycle_registry(),
        &artifact.native_value_lifecycle_registry
    );
    assert_eq!(
        authorities.value_lifecycle_policy(),
        &artifact.value_lifecycle_policy
    );
    assert_eq!(
        authorities.host_effect_registry(),
        &artifact.host_effect_registry
    );
    assert_eq!(
        authorities.intrinsic_registry(),
        &artifact.intrinsic_registry
    );
    assert_eq!(handoff.receipt().function_count(), 1);
    assert_eq!(handoff.receipt().word_count(), 1);
    assert_eq!(handoff.receipt().relocation_count(), 0);

    let (actual_artifact, actual_reference, actual_receipt) = handoff.into_parts();
    assert_eq!(actual_artifact, artifact);
    assert_eq!(actual_reference, reference);
    assert_eq!(
        actual_receipt.bytecode_identity(),
        artifact.bytecode_identity
    );
}

#[test]
fn fixture_carries_every_required_v5_manifest_field() {
    let artifact = canonical_artifact();
    let wire = serde_json::to_value(&artifact).unwrap();
    let function = &wire["image"]["functions"]["module::main"];

    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v5");
    assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v3:sha256");
    assert_eq!(
        BYTECODE_IDENTITY_SCHEMA_MARKER,
        "skiff-bytecode-artifact-v3"
    );
    assert!(artifact
        .bytecode_identity
        .starts_with("skiff-bytecode-image-v3:sha256:"));
    assert_eq!(wire["magic"], BYTECODE_MAGIC);
    assert_eq!(wire["schemaVersion"], BYTECODE_SCHEMA_VERSION);
    assert_eq!(wire["isaVersion"], BYTECODE_ISA_VERSION);
    assert_eq!(
        wire["opcodeTableFingerprint"],
        artifact.opcode_table_fingerprint
    );
    assert_eq!(
        wire["nativeValueLifecycleRegistry"],
        serde_json::to_value(skiff_artifact_model::native_value_lifecycle_registry_identity())
            .unwrap()
    );
    assert_eq!(
        wire["valueLifecyclePolicy"],
        serde_json::to_value(skiff_artifact_model::value_lifecycle_policy_identity()).unwrap()
    );
    assert_eq!(
        wire["hostEffectRegistry"],
        serde_json::to_value(skiff_artifact_model::host_effect_registry_identity()).unwrap()
    );
    assert_eq!(
        wire["intrinsicRegistry"],
        serde_json::to_value(skiff_artifact_model::intrinsic_registry_identity()).unwrap()
    );
    assert_eq!(wire["image"]["constantRoots"], serde_json::json!({}));
    assert_eq!(
        function["origin"],
        serde_json::json!({
            "kind": "executable",
            "executable": {
                "fileIrIdentity": format!("{FILE_IR_IDENTITY_PREFIX}:{}", "a".repeat(64)),
                "modulePath": "module",
                "executableIndex": 0,
            },
        })
    );
    assert_eq!(function["selfTypeRef"], serde_json::Value::Null);
    assert_eq!(function["callLoanLayouts"], serde_json::json!([]));
    assert_eq!(
        function["frameLayout"]["writableLocalSlots"],
        serde_json::json!([])
    );
}

#[test]
fn semantic_authority_drift_is_rejected_before_handoff() {
    type AuthorityMutation = (&'static str, fn(&mut BytecodeArtifact));

    let mutations: [AuthorityMutation; 4] = [
        ("native value lifecycle registry", |artifact| {
            artifact
                .native_value_lifecycle_registry
                .fingerprint
                .push_str(":corrupt");
        }),
        ("value lifecycle policy", |artifact| {
            artifact
                .value_lifecycle_policy
                .fingerprint
                .push_str(":corrupt");
        }),
        ("host-effect registry", |artifact| {
            artifact
                .host_effect_registry
                .fingerprint
                .push_str(":corrupt");
        }),
        ("intrinsic registry", |artifact| {
            artifact.intrinsic_registry.fingerprint.push_str(":corrupt");
        }),
    ];

    for (authority, mutate) in mutations {
        let mut artifact = canonical_artifact();
        mutate(&mut artifact);
        let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());

        assert!(
            matches!(
                BytecodeCompilationHandoff::try_new(artifact, reference),
                Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
            ),
            "{authority} drift must fail closed"
        );
    }
}

#[test]
fn legacy_schema_and_identity_generations_are_rejected_before_handoff() {
    let mut stale_schema = canonical_artifact();
    stale_schema.schema_version = "skiff-bytecode-v4".to_string();
    let reference = BytecodeArtifactRef::new(stale_schema.bytecode_identity.clone());
    assert!(matches!(
        BytecodeCompilationHandoff::try_new(stale_schema, reference),
        Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
    ));

    let mut stale_identity = canonical_artifact();
    stale_identity.bytecode_identity = format!("skiff-bytecode-image-v2:sha256:{}", "0".repeat(64));
    let reference = BytecodeArtifactRef::new(stale_identity.bytecode_identity.clone());
    assert!(matches!(
        BytecodeCompilationHandoff::try_new(stale_identity, reference),
        Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
    ));
}

#[test]
fn noncanonical_artifact_is_rejected_before_handoff() {
    let mut artifact = canonical_artifact();
    artifact.bytecode_identity = format!("{BYTECODE_IDENTITY_PREFIX}:{}", "0".repeat(64));
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());

    assert!(matches!(
        BytecodeCompilationHandoff::try_new(artifact, reference),
        Err(BytecodeCompilationHandoffError::InvalidCanonicalArtifact { .. })
    ));
}

#[test]
fn reference_must_name_the_admitted_artifact() {
    let artifact = canonical_artifact();
    let reference =
        BytecodeArtifactRef::new(format!("{BYTECODE_IDENTITY_PREFIX}:{}", "0".repeat(64)));

    assert!(matches!(
        BytecodeCompilationHandoff::try_new(artifact, reference),
        Err(BytecodeCompilationHandoffError::ReferenceIdentityMismatch { .. })
    ));
}

#[test]
fn reference_path_is_reserved_for_the_upper_store_writer() {
    let artifact = canonical_artifact();
    let mut reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    reference.artifact_path = Some("records/premature.json".to_string());

    assert!(matches!(
        BytecodeCompilationHandoff::try_new(artifact, reference),
        Err(BytecodeCompilationHandoffError::PrematureArtifactPath { artifact_path })
            if artifact_path == "records/premature.json"
    ));
}

#[test]
fn lane_outcome_never_turns_enabled_failure_into_disabled() {
    let disabled = BytecodeCompilationOutcome::<&str>::disabled();
    assert!(disabled.into_result().unwrap().is_none());

    let artifact = canonical_artifact();
    let handoff = BytecodeCompilationHandoff::try_new(
        artifact.clone(),
        BytecodeArtifactRef::new(artifact.bytecode_identity.clone()),
    )
    .unwrap();
    let enabled = BytecodeCompilationOutcome::<&str>::enabled(handoff);
    assert_eq!(
        enabled
            .into_result()
            .unwrap()
            .unwrap()
            .artifact()
            .bytecode_identity,
        artifact.bytecode_identity
    );

    let failed = BytecodeCompilationOutcome::from_enabled_result(Err("emission failed"));
    assert_eq!(failed.into_result().unwrap_err(), "emission failed");
}
