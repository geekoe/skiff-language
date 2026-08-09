//! Bytecode identity tests: determinism, mutation matrix (every preimage
//! field participates), C1–C8 gating, C9 mismatch rejection and
//! `ValidatedBytecodeArtifact` admission.

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodeArtifact, BytecodeArtifactRef, BytecodeConstantRef, BytecodeFunctionOrigin,
    BytecodeImage, BytecodePoolEntry, BytecodePools, BytecodeRelocation, BytecodeSpecialization,
    DebugBinding, DebugTable, FrameLayout, FrozenConstantGraph, FrozenConstantNode, LiteralIr,
    PackageCallableId, PackageExecutableCoordinate, RelocatableBytecodeFunction, SourceMapEntry,
    StatementChargeKind, StatementEntry, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::*;

fn snapshot_share() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    }
}

/// Hand-built structurally valid artifact (passes C1–C8). Not encoder or
/// emitter produced; `bytecode_identity` starts as a placeholder so tests can
/// prove the preimage excludes the identity field itself.
fn fixture() -> BytecodeArtifact {
    let mut functions = BTreeMap::new();
    functions.insert(
        "module::main".to_string(),
        RelocatableBytecodeFunction {
            function_key: "module::main".to_string(),
            origin: BytecodeFunctionOrigin::Executable {
                executable: PackageExecutableCoordinate {
                    file_ir_identity: format!(
                        "{}:{}",
                        crate::FILE_IR_IDENTITY_PREFIX,
                        "a".repeat(64)
                    ),
                    module_path: "module".to_string(),
                    executable_index: 0,
                },
            },
            type_parameters: Vec::new(),
            self_type_ref: None,
            words: vec![0x00, 0, 0x03, 0, 0x11, 0, 0x20, 0, 0, 0, 0x25],
            relocations: vec![BytecodeRelocation::LocalExecutableRef {
                function_key: "module::main".to_string(),
                specialization: BytecodeSpecialization {
                    type_arguments: Vec::new(),
                    concrete_receiver: None,
                },
            }],
            call_loan_layouts: Vec::new(),
            frame_layout: FrameLayout {
                slot_count: 1,
                slot_type_refs: vec![0],
                parameter_slots: Vec::new(),
                writable_local_slots: Vec::new(),
                result_count: 1,
                result_type_refs: vec![1],
                result_plans: vec![snapshot_share()],
                slot_plans: vec![snapshot_share()],
            },
            max_operand_depth: 2,
            effect_summary_ref: PackageCallableId::new("operation:module:main"),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries: vec![StatementEntry {
                pc: 0,
                statement_id: "s:main:entry".to_string(),
                charge_kind: StatementChargeKind::FunctionEntry,
            }],
            source_map: vec![SourceMapEntry {
                start_pc: 6,
                end_pc: 10,
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
            }],
        },
    );
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::bytecode::opcodes::opcode_table_fingerprint(
        ),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        bytecode_identity: format!("{BYTECODE_IDENTITY_PREFIX}:fixture"),
        image: BytecodeImage {
            functions,
            pools: BytecodePools {
                constants: vec![BytecodePoolEntry::ConstantRef {
                    reference: BytecodeConstantRef::LocalNode { node_index: 0 },
                    type_ref: 1,
                    plan: snapshot_share(),
                }],
                types: vec![
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("string"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("number"),
                    },
                ],
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
                writable_paths: Vec::new(),
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph {
                nodes: vec![FrozenConstantNode::Literal {
                    literal: LiteralIr::Number {
                        value: serde_json::Number::from(42),
                    },
                }],
            },
            debug_table: Some(DebugTable {
                bindings: vec![DebugBinding {
                    function_key: "module::main".to_string(),
                    pc: 0,
                    name: "x".to_string(),
                    slot: 0,
                }],
            }),
        },
    }
}

#[test]
fn identity_is_deterministic_and_excludes_the_identity_field() {
    let mut first = fixture();
    let mut second = fixture();
    let computed_first = bytecode_identity(&first).unwrap();
    let computed_second = bytecode_identity(&second).unwrap();
    assert_eq!(computed_first, computed_second);
    assert!(
        computed_first.starts_with(&format!("{BYTECODE_IDENTITY_PREFIX}:")),
        "identity must use the canonical prefix"
    );

    // Assigning the identity must be idempotent and the preimage must not
    // include the declared identity field itself (self-reference).
    assign_bytecode_identity(&mut first).unwrap();
    assign_bytecode_identity(&mut second).unwrap();
    assert_eq!(first.bytecode_identity, computed_first);
    assert_eq!(bytecode_identity(&first).unwrap(), computed_first);
    assert_eq!(bytecode_identity(&second).unwrap(), computed_first);
    assert_eq!(first.bytecode_identity, second.bytecode_identity);
}

#[test]
fn schema_isa_fingerprint_and_lifecycle_registry_participate_in_the_preimage() {
    let base = fixture();
    let base_hash = bytecode_identity_after_structural(&base).unwrap();

    let mutations: [fn(&mut BytecodeArtifact); 6] = [
        |artifact: &mut BytecodeArtifact| artifact.schema_version = "skiff-bytecode-v3".to_string(),
        |artifact: &mut BytecodeArtifact| {
            artifact.isa_version = "skiff-bytecode-isa-v3".to_string()
        },
        |artifact: &mut BytecodeArtifact| {
            artifact.opcode_table_fingerprint = "0".repeat(64).to_string()
        },
        |artifact: &mut BytecodeArtifact| {
            artifact
                .native_value_lifecycle_registry
                .registry_id
                .push_str("-changed")
        },
        |artifact: &mut BytecodeArtifact| {
            artifact
                .native_value_lifecycle_registry
                .version
                .push_str("-changed")
        },
        |artifact: &mut BytecodeArtifact| {
            artifact.native_value_lifecycle_registry.fingerprint = "0".repeat(64)
        },
    ];
    for (label, mutate) in [
        ("schemaVersion", mutations[0]),
        ("isaVersion", mutations[1]),
        ("opcodeTableFingerprint", mutations[2]),
        ("nativeValueLifecycleRegistry.registryId", mutations[3]),
        ("nativeValueLifecycleRegistry.version", mutations[4]),
        ("nativeValueLifecycleRegistry.fingerprint", mutations[5]),
    ] {
        let mut mutated = base.clone();
        mutate(&mut mutated);
        let mutated_hash = bytecode_identity_after_structural(&mutated).unwrap();
        assert_ne!(
            mutated_hash, base_hash,
            "{label} mutation must change the bytecode identity"
        );
        // C1 rejects these mutations before any identity work when going
        // through the public gated entry.
        assert!(matches!(
            bytecode_identity(&mutated),
            Err(ArtifactIdentityError::InvalidBytecodeStructural(
                skiff_artifact_model::bytecode::validate::StructuralValidationError::Header { .. }
            ))
        ));
    }

    let mut bad_magic = base;
    bad_magic.magic = "skiff-bytecode-old".to_string();
    assert!(matches!(
        bytecode_identity(&bad_magic),
        Err(ArtifactIdentityError::InvalidBytecodeStructural(
            skiff_artifact_model::bytecode::validate::StructuralValidationError::Header { .. }
        ))
    ));
}

#[test]
fn every_image_mutation_changes_the_identity() {
    let base = fixture();
    let base_identity = bytecode_identity(&base).unwrap();

    let mut word_changed = base.clone();
    // jump_if_true target 6 -> target 4 (delta -2): stays on a header, so the
    // mutated artifact still passes C1–C8 while the preimage changes.
    word_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words[5] = 0xFFFF_FFFE;
    assert_ne!(bytecode_identity(&word_changed).unwrap(), base_identity);

    let mut pool_changed = base.clone();
    pool_changed.image.pools.types[0] = BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::builtin("number"),
    };
    assert_ne!(bytecode_identity(&pool_changed).unwrap(), base_identity);

    let mut slot_type_changed = base.clone();
    slot_type_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .slot_type_refs[0] = 1;
    assert_ne!(
        bytecode_identity(&slot_type_changed).unwrap(),
        base_identity,
        "schema v4 frame slot types must participate in the identity"
    );

    let mut result_type_changed = base.clone();
    result_type_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .result_type_refs[0] = 0;
    assert_ne!(
        bytecode_identity(&result_type_changed).unwrap(),
        base_identity,
        "schema v4 frame result types must participate in the identity"
    );

    let mut writable_locals_changed = base.clone();
    writable_locals_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .writable_local_slots = vec![0];
    assert_ne!(
        bytecode_identity(&writable_locals_changed).unwrap(),
        base_identity,
        "schema v4 writable-local frame facts must participate in the identity"
    );

    let mut origin_changed = base.clone();
    let BytecodeFunctionOrigin::Executable { executable } = &mut origin_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .origin
    else {
        unreachable!()
    };
    executable.executable_index = 1;
    assert_ne!(
        bytecode_identity(&origin_changed).unwrap(),
        base_identity,
        "schema v4 executable origins must participate in the identity"
    );

    let mut graph_changed = base.clone();
    graph_changed.image.frozen_constant_graph.nodes[0] = FrozenConstantNode::Literal {
        literal: LiteralIr::Number {
            value: serde_json::Number::from(43),
        },
    };
    assert_ne!(bytecode_identity(&graph_changed).unwrap(), base_identity);

    let mut debug_changed = base.clone();
    debug_changed.image.debug_table.as_mut().unwrap().bindings[0].name = "renamed".to_string();
    assert_ne!(
        bytecode_identity(&debug_changed).unwrap(),
        base_identity,
        "debug table must participate in the identity (D14)"
    );

    let mut key_changed = base.clone();
    let main = key_changed.image.functions.remove("module::main").unwrap();
    key_changed
        .image
        .functions
        .insert("module::renamed".to_string(), main);
    key_changed
        .image
        .functions
        .get_mut("module::renamed")
        .unwrap()
        .function_key = "module::renamed".to_string();
    let BytecodeRelocation::LocalExecutableRef { function_key, .. } = &mut key_changed
        .image
        .functions
        .get_mut("module::renamed")
        .unwrap()
        .relocations[0]
    else {
        unreachable!()
    };
    *function_key = "module::renamed".to_string();
    key_changed.image.debug_table.as_mut().unwrap().bindings[0].function_key =
        "module::renamed".to_string();
    assert_ne!(bytecode_identity(&key_changed).unwrap(), base_identity);
}

#[test]
fn declared_identity_mismatch_is_rejected_by_validate_and_admit() {
    let mut valid = fixture();
    assign_bytecode_identity(&mut valid).unwrap();

    let mut mismatched = valid.clone();
    mismatched.bytecode_identity = format!("{BYTECODE_IDENTITY_PREFIX}:{}", "0".repeat(64));
    assert!(matches!(
        validate_bytecode_identity(&mismatched),
        Err(ArtifactIdentityError::BytecodeIdentityMismatch { .. })
    ));
    assert!(ValidatedBytecodeArtifact::admit(mismatched).is_err());

    assert!(validate_bytecode_identity(&valid).is_ok());
    let admitted = ValidatedBytecodeArtifact::admit(valid.clone()).unwrap();
    assert_eq!(admitted.artifact(), &valid);
    assert_eq!(admitted.view().functions().len(), 1);
    assert_eq!(
        admitted.reference(),
        &BytecodeArtifactRef::new(valid.bytecode_identity.clone())
    );
    assert_eq!(
        admitted.artifact().bytecode_identity,
        valid.bytecode_identity
    );
}

#[test]
fn identity_format_validation_accepts_only_framed_lowercase_sha256() {
    let valid = fixture();
    let identity = bytecode_identity(&valid).unwrap();
    validate_bytecode_identity_format(&identity).unwrap();

    let leaf = identity
        .strip_prefix(&format!("{BYTECODE_IDENTITY_PREFIX}:"))
        .unwrap();
    for invalid in [
        format!("{BYTECODE_IDENTITY_PREFIX}:{}", leaf.to_uppercase()),
        format!("{BYTECODE_IDENTITY_PREFIX}:short"),
        format!("{BYTECODE_IDENTITY_PREFIX}:{}", "z".repeat(64)),
        format!("skiff-bytecode-image-v1:sha256:{leaf}"),
        "unframed".to_string(),
    ] {
        assert!(matches!(
            validate_bytecode_identity_format(&invalid),
            Err(ArtifactIdentityError::InvalidBytecodeIdentity { .. })
        ));
    }
}
