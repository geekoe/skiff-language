use std::collections::BTreeMap;

use skiff_artifact_identity::{assign_bytecode_identity, BYTECODE_IDENTITY_PREFIX};
use skiff_artifact_model::{
    descriptor_for_opcode, BytecodeArtifact, BytecodeArtifactRef, BytecodeImage, BytecodePools,
    BytecodeRelocation, FrameLayout, FrozenConstantGraph, Opcode, PackageCallableId,
    RelocatableBytecodeFunction, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::*;

fn canonical_artifact() -> BytecodeArtifact {
    let function_key = "module::main".to_string();
    let call_local = u32::from(descriptor_for_opcode(Opcode::CallLocal).opcode);
    let return_ = u32::from(descriptor_for_opcode(Opcode::Return).opcode);
    let function = RelocatableBytecodeFunction {
        function_key: function_key.clone(),
        type_parameters: Vec::new(),
        words: vec![call_local, 0, 0, return_],
        relocations: vec![BytecodeRelocation::LocalExecutableRef {
            function_key: function_key.clone(),
        }],
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: Vec::new(),
        },
        max_operand_depth: 1,
        effect_summary_ref: PackageCallableId::new("operation:module:main"),
        exception_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    };
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::opcode_table_fingerprint(),
        bytecode_identity: "identity-is-assigned-after-structural-validation".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::from([(function_key, function)]),
            pools: BytecodePools::default(),
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
    assert_eq!(handoff.receipt().function_count(), 1);
    assert_eq!(handoff.receipt().word_count(), 4);
    assert_eq!(handoff.receipt().relocation_count(), 1);

    let (actual_artifact, actual_reference, actual_receipt) = handoff.into_parts();
    assert_eq!(actual_artifact, artifact);
    assert_eq!(actual_reference, reference);
    assert_eq!(
        actual_receipt.bytecode_identity(),
        artifact.bytecode_identity
    );
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
