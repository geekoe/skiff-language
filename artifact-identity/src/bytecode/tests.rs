//! Bytecode identity tests: determinism, mutation matrix (every preimage
//! field participates), C1–C8 gating, C9 mismatch rejection and
//! `ValidatedBytecodeArtifact` admission.

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodeArtifact, BytecodeArtifactRef, BytecodeConstantRef, BytecodeFunctionOrigin,
    BytecodeImage, BytecodeIntrinsicRef, BytecodePoolEntry, BytecodePools, BytecodeRelocation,
    BytecodeSpecialization, DebugBinding, DebugTable, FrameLayout, FrozenConstantGraph,
    FrozenConstantNode, HostEffectSignature, IntrinsicReference, LiteralIr, PackageCallableId,
    PackageExecutableCoordinate, ParamModeIr, ParameterSlotDecl, RelocatableBytecodeFunction,
    ResumeDescriptor, ResumeErrorMode, ResumeResultMaterialization, ShapeDeclaration,
    SourceMapEntry, StatementAttributionId, StatementEntry, TypeRefIr, ValueDropPlan,
    ValueTransferPlan, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};

use super::*;

type BytecodeMutation = (&'static str, fn(&mut BytecodeArtifact));

fn historical_platform_error_projection_registry_ref(
    character: char,
) -> skiff_artifact_model::PlatformErrorProjectionRegistryRef {
    serde_json::from_value(serde_json::json!({
        "registryId": skiff_artifact_model::PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
        "registryVersion": skiff_artifact_model::PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
        "fingerprint": format!("sha256:{}", character.to_string().repeat(64)),
    }))
    .expect("historical registry descriptor must satisfy the strict general shape")
}

fn snapshot_share() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    }
}

fn type_entry(ty: TypeRefIr) -> BytecodePoolEntry {
    BytecodePoolEntry::TypeRef {
        ty,
        plan: snapshot_share(),
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
                stream_result_type_ref: None,
                slot_plans: vec![snapshot_share()],
            },
            max_operand_depth: 2,
            effect_summary_ref: PackageCallableId::new("operation:module:main"),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries: vec![StatementEntry {
                pc: 6,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Expression {
                    expression_index: 0,
                    occurrence_ordinal: 0,
                },
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
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
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
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
                    type_entry(TypeRefIr::builtin("string")),
                    type_entry(TypeRefIr::builtin("number")),
                ],
                shapes: (0..2)
                    .map(|_| BytecodePoolEntry::ShapeRef {
                        shape: ShapeDeclaration {
                            type_ref: 0,
                            plan: ValueTransferPlan::SnapshotShare {
                                drop: ValueDropPlan::SnapshotRelease,
                            },
                            privileged_affine_composite: None,
                            fields: Vec::new(),
                        },
                    })
                    .collect(),
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
fn schema_isa_and_all_semantic_authorities_participate_in_the_preimage() {
    let base = fixture();
    let base_view = skiff_artifact_model::structurally_validate(&base).unwrap();
    let base_hash = bytecode_identity_after_structural(&base, Some(&base_view)).unwrap();
    let preimage =
        serde_json::to_value(BytecodeIdentityPayload::from_view(&base, Some(&base_view))).unwrap();
    assert_eq!(preimage["schema"], "skiff-bytecode-artifact-v5");
    assert_eq!(
        preimage
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "schema",
            "schemaVersion",
            "isaVersion",
            "opcodeTableFingerprint",
            "nativeValueLifecycleRegistry",
            "valueLifecyclePolicy",
            "hostEffectRegistry",
            "intrinsicRegistry",
            "platformErrorProjectionRegistry",
            "image",
            "intrinsicContracts",
            "functionStreamItems",
        ]
    );
    assert_eq!(
        preimage["platformErrorProjectionRegistry"],
        serde_json::to_value(
            skiff_artifact_model::current_platform_error_projection_registry_ref()
        )
        .unwrap()
    );
    assert_eq!(
        preimage["platformErrorProjectionRegistry"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["registryId", "registryVersion", "fingerprint"]
    );

    let mutations: [BytecodeMutation; 15] = [
        ("schemaVersion", |artifact| {
            artifact.schema_version = "skiff-bytecode-v6".to_string();
        }),
        ("isaVersion", |artifact| {
            artifact.isa_version = "skiff-bytecode-isa-v3".to_string();
        }),
        ("opcodeTableFingerprint", |artifact| {
            artifact.opcode_table_fingerprint = "0".repeat(64);
        }),
        ("nativeValueLifecycleRegistry.registryId", |artifact| {
            artifact
                .native_value_lifecycle_registry
                .registry_id
                .push_str("-changed");
        }),
        ("nativeValueLifecycleRegistry.version", |artifact| {
            artifact
                .native_value_lifecycle_registry
                .version
                .push_str("-changed");
        }),
        ("nativeValueLifecycleRegistry.fingerprint", |artifact| {
            artifact.native_value_lifecycle_registry.fingerprint = "0".repeat(64);
        }),
        ("valueLifecyclePolicy.version", |artifact| {
            artifact.value_lifecycle_policy.version.push_str("-changed");
        }),
        ("valueLifecyclePolicy.fingerprint", |artifact| {
            artifact.value_lifecycle_policy.fingerprint = "0".repeat(64);
        }),
        ("hostEffectRegistry.registryId", |artifact| {
            artifact
                .host_effect_registry
                .registry_id
                .push_str("-changed");
        }),
        ("hostEffectRegistry.version", |artifact| {
            artifact.host_effect_registry.version.push_str("-changed");
        }),
        ("hostEffectRegistry.fingerprint", |artifact| {
            artifact.host_effect_registry.fingerprint = "0".repeat(64);
        }),
        ("intrinsicRegistry.registryId", |artifact| {
            artifact.intrinsic_registry.registry_id.push_str("-changed");
        }),
        ("intrinsicRegistry.version", |artifact| {
            artifact.intrinsic_registry.version.push_str("-changed");
        }),
        ("intrinsicRegistry.fingerprint", |artifact| {
            artifact.intrinsic_registry.fingerprint = "0".repeat(64);
        }),
        ("platformErrorProjectionRegistry.fingerprint", |artifact| {
            artifact.platform_error_projection_registry =
                historical_platform_error_projection_registry_ref('0');
        }),
    ];
    for (label, mutate) in mutations {
        let mut mutated = base.clone();
        mutate(&mut mutated);
        let mutated_hash = bytecode_identity_after_structural(&mutated, None).unwrap();
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
    pool_changed.image.pools.types[0] = type_entry(TypeRefIr::builtin("number"));
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
        "schema v7 frame slot types must participate in the identity"
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
        "schema v7 frame result types must participate in the identity"
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
        "schema v7 writable-local frame facts must participate in the identity"
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
        "schema v7 executable origins must participate in the identity"
    );

    let mut source_event_changed = base.clone();
    let source_event = &mut source_event_changed
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .statement_entries[0];
    source_event.site = skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerDesugaring,
    };
    assert_ne!(
        bytecode_identity(&source_event_changed).unwrap(),
        base_identity,
        "schema v7 source-event rows must participate in the image identity"
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
        format!("skiff-bytecode-image-v4:sha256:{leaf}"),
        "unframed".to_string(),
    ] {
        assert!(matches!(
            validate_bytecode_identity_format(&invalid),
            Err(ArtifactIdentityError::InvalidBytecodeIdentity { .. })
        ));
    }

    let error = validate_bytecode_identity_format("stale").unwrap_err();
    assert!(
        error.to_string().contains(BYTECODE_IDENTITY_PREFIX),
        "format diagnostics must use the current identity prefix"
    );
}

fn authority_fixture() -> BytecodeArtifact {
    let mut artifact = fixture();
    let entry = skiff_artifact_model::intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                BytecodeIntrinsicRef::Static { canonical_key, .. }
                    if canonical_key == "core.array.empty"
            )
        })
        .expect("intrinsic registry contains core.array.empty");
    let intrinsic = IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_plans: Vec::new(),
            result_types: vec![TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            }],
            result_plans: vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }],
            effects: entry.signature.effects.clone(),
        },
    };
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations
        .push(BytecodeRelocation::IntrinsicRef { intrinsic });

    let stream_index = artifact.image.pools.types.len() as u32;
    artifact
        .image
        .pools
        .types
        .push(type_entry(TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        }));
    artifact
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::producer".to_string(),
            site_pc: 0,
            resume_pc: 2,
            end_resume_pc: None,
            expected_stack_height_before_result: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            result_materializations: Vec::new(),
            emit_stream_item_shape_ref: Some(0),
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));
    let producer = RelocatableBytecodeFunction {
        function_key: "module::producer".to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: PackageExecutableCoordinate {
                file_ir_identity: format!("{}:{}", crate::FILE_IR_IDENTITY_PREFIX, "a".repeat(64)),
                module_path: "module".to_string(),
                executable_index: 1,
            },
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![0x61, 0, 0x25],
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
            stream_result_type_ref: Some(stream_index),
            slot_plans: Vec::new(),
        },
        max_operand_depth: 1,
        effect_summary_ref: PackageCallableId::new("operation:module:producer"),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: vec![StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        }],
        source_map: vec![SourceMapEntry {
            start_pc: 0,
            end_pc: 2,
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
        }],
    };
    artifact
        .image
        .functions
        .insert("module::producer".to_string(), producer);
    artifact
}

fn stream_consumer_fixture() -> BytecodeArtifact {
    let mut artifact = fixture();
    artifact
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::consumer".to_string(),
            site_pc: 0,
            resume_pc: 3,
            end_resume_pc: Some(4),
            expected_stack_height_before_result: 0,
            result_type_refs: vec![0],
            result_plans: vec![snapshot_share()],
            result_materializations: vec![None],
            emit_stream_item_shape_ref: None,
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));
    artifact.image.functions.insert(
        "module::consumer".to_string(),
        RelocatableBytecodeFunction {
            function_key: "module::consumer".to_string(),
            origin: BytecodeFunctionOrigin::Executable {
                executable: PackageExecutableCoordinate {
                    file_ir_identity: format!(
                        "{}:{}",
                        crate::FILE_IR_IDENTITY_PREFIX,
                        "a".repeat(64)
                    ),
                    module_path: "module".to_string(),
                    executable_index: 1,
                },
            },
            type_parameters: Vec::new(),
            self_type_ref: None,
            words: vec![0x60, 0, 0, 0x08, 0x08, 0x25],
            relocations: Vec::new(),
            call_loan_layouts: Vec::new(),
            frame_layout: FrameLayout {
                slot_count: 1,
                slot_type_refs: vec![0],
                parameter_slots: Vec::new(),
                writable_local_slots: Vec::new(),
                result_count: 0,
                result_type_refs: Vec::new(),
                result_plans: Vec::new(),
                stream_result_type_ref: None,
                slot_plans: vec![snapshot_share()],
            },
            max_operand_depth: 1,
            effect_summary_ref: PackageCallableId::new("operation:module:consumer"),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries: vec![StatementEntry {
                pc: 0,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Generated { ordinal: 0 },
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
            }],
            source_map: vec![SourceMapEntry {
                start_pc: 0,
                end_pc: 4,
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
            }],
        },
    );
    artifact
}

#[test]
fn derived_execution_authorities_participate_in_the_preimage() {
    let artifact = authority_fixture();
    let view = skiff_artifact_model::structurally_validate(&artifact).unwrap();
    assert_eq!(view.function_stream_items().len(), 1);
    assert_eq!(view.intrinsic_contracts().len(), 1);

    let with_contracts = bytecode_identity_after_structural(&artifact, Some(&view)).unwrap();
    let without_contracts = bytecode_identity_after_structural(&artifact, None).unwrap();
    assert_ne!(
        with_contracts, without_contracts,
        "derived execution authorities must be canonical identity inputs"
    );

    let mut mutated = artifact;
    let number_stream_index = mutated.image.pools.types.len() as u32;
    mutated
        .image
        .pools
        .types
        .push(type_entry(TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        }));
    mutated
        .image
        .functions
        .get_mut("module::producer")
        .unwrap()
        .frame_layout
        .stream_result_type_ref = Some(number_stream_index);
    assert_ne!(
        bytecode_identity(&mutated).unwrap(),
        with_contracts,
        "stream producer item type must participate in bytecode identity"
    );
}

#[test]
fn parameter_dense_record_shape_ref_participates_in_the_preimage() {
    let mut parameter = fixture();
    parameter
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .parameter_slots = vec![ParameterSlotDecl {
        slot: 0,
        mode: ParamModeIr::Value,
        plan: snapshot_share(),
        dense_record_shape_ref: None,
    }];
    let shapeless_identity = bytecode_identity(&parameter).unwrap();
    parameter
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .parameter_slots[0]
        .dense_record_shape_ref = Some(0);
    skiff_artifact_model::structurally_validate(&parameter)
        .expect("Some parameter DenseRecord fixture remains structurally valid");
    assert_ne!(
        bytecode_identity(&parameter).unwrap(),
        shapeless_identity,
        "parameter denseRecordShapeRef None-to-Some must change bytecode identity"
    );
}

#[test]
fn resume_materialization_and_emit_shape_refs_participate_in_the_preimage() {
    let ordinary = stream_consumer_fixture();
    let ordinary_identity = bytecode_identity(&ordinary).unwrap();
    let mut materialized = ordinary;
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut materialized.image.pools.resume[0]
    else {
        unreachable!("stream consumer resume descriptor")
    };
    descriptor.result_materializations = vec![Some(ResumeResultMaterialization::DenseRecord {
        shape_ref: 0,
    })];
    skiff_artifact_model::structurally_validate(&materialized)
        .expect("Some(DenseRecord) fixture remains structurally valid");
    assert_ne!(
        bytecode_identity(&materialized).unwrap(),
        ordinary_identity,
        "resultMaterializations None-to-Some must change bytecode identity"
    );

    let emitted = authority_fixture();
    let emitted_identity = bytecode_identity(&emitted).unwrap();
    let mut retargeted = emitted;
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut retargeted.image.pools.resume[0]
    else {
        unreachable!("EmitStream resume descriptor")
    };
    descriptor.emit_stream_item_shape_ref = Some(1);
    skiff_artifact_model::structurally_validate(&retargeted)
        .expect("alternate bounded EmitStream shape fixture remains structurally valid");
    assert_ne!(
        bytecode_identity(&retargeted).unwrap(),
        emitted_identity,
        "emitStreamItemShapeRef drift must change bytecode identity"
    );
}

#[test]
fn stream_next_end_resume_pc_participates_in_the_preimage() {
    let base = stream_consumer_fixture();
    let baseline = bytecode_identity(&base).unwrap();

    let mut changed = base;
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut changed.image.pools.resume[0] else {
        unreachable!("stream consumer resume descriptor")
    };
    descriptor.end_resume_pc = Some(5);
    assert_ne!(
        bytecode_identity(&changed).unwrap(),
        baseline,
        "StreamNext endResumePc must participate in bytecode identity"
    );
}
