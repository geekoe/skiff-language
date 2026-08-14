//! Focused malformed corpus for the v11 schema / ISA v5 contract. Every case is a
//! hand-written corruption of the canonical fixture and must fail closed.

use crate::bytecode::dto::{
    limits, BytecodeConstantRef, BytecodePoolEntry, BytecodeRelocation, CatchMatcher,
    ExceptionRegion, FrozenBehaviorBinding, FrozenConstantNode, ResumeDescriptor, ResumeErrorMode,
    ShapeFieldDeclaration, SourceMapEntry, SwitchCase, ValueTransferPlan,
};

use super::*;

#[test]
fn corpus_rejects_stale_versions_and_opcode_projection() {
    let mut stale_schema = canonical_artifact();
    stale_schema.schema_version = "skiff-bytecode-v6".to_string();
    assert!(matches!(
        assert_rejected(&stale_schema),
        StructuralValidationError::Header { .. }
    ));

    let mut stale_isa = canonical_artifact();
    stale_isa.isa_version = "skiff-bytecode-isa-v2".to_string();
    assert!(matches!(
        assert_rejected(&stale_isa),
        StructuralValidationError::Header { .. }
    ));

    let mut stale_table = canonical_artifact();
    stale_table.opcode_table_fingerprint = "0".repeat(64);
    let error = assert_rejected(&stale_table);
    assert!(matches!(error, StructuralValidationError::Header { .. }));
    assert!(error.to_string().contains("opcodeTableFingerprint"));
}

#[test]
fn corpus_rejects_unknown_and_truncated_instructions() {
    for words in [vec![0xFF], vec![0x100], vec![0x9C]] {
        let mut artifact = canonical_artifact();
        let helper = artifact.image.functions.get_mut("module::helper").unwrap();
        helper.words = words;
        helper.statement_entries.clear();
        artifact.image.debug_table = None;
        assert!(matches!(
            assert_rejected(&artifact),
            StructuralValidationError::Decode { .. }
        ));
    }

    for words in [vec![0x00], vec![0x20, 0, 0], vec![0x24, 0, 0, 0, 0]] {
        let mut artifact = canonical_artifact();
        let helper = artifact.image.functions.get_mut("module::helper").unwrap();
        helper.words = words;
        helper.statement_entries.clear();
        artifact.image.debug_table = None;
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Decode { .. }));
        assert!(error.to_string().contains("truncated instruction"));
    }
}

#[test]
fn corpus_rejects_bad_operand_indices_and_result_counts() {
    let cases = [
        (3usize, 4u32, "slot index"),
        (1, 99, "pool index"),
        (7, 99, "relocation index"),
        (12, 99, "table index"),
    ];
    for (word_index, value, expected) in cases {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words[word_index] = value;
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Operand { .. }));
        assert!(error.to_string().contains(expected), "{error}");
    }

    let mut too_many_results = canonical_artifact();
    too_many_results
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words[9] = 2;
    let error = assert_rejected(&too_many_results);
    assert!(error.to_string().contains("MAX_RESULTS_PER_CALL"));

    let mut bad_failure_kind = canonical_artifact();
    let helper = bad_failure_kind
        .image
        .functions
        .get_mut("module::helper")
        .unwrap();
    helper.words = vec![0x15, 1];
    helper.statement_entries[0].pc = 0;
    helper.source_map = vec![SourceMapEntry {
        start_pc: 0,
        end_pc: 2,
        site: crate::InstructionSourceSite::Synthetic {
            reason: crate::SyntheticInstructionSiteReason::CompilerDesugaring,
        },
    }];
    bad_failure_kind.image.debug_table = None;
    assert!(assert_rejected(&bad_failure_kind)
        .to_string()
        .contains("unknown trap failure kind"));
}

#[test]
fn corpus_rejects_branch_and_switch_targets_in_operand_words() {
    let mut branch = canonical_artifact();
    branch
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words[14] = 1;
    let error = assert_rejected(&branch);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(error.to_string().contains("instruction header"));

    let mut switch = canonical_artifact();
    switch
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .switch_tables[0]
        .cases[1]
        .target_pc = 21;
    let error = assert_rejected(&switch);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("instruction header"));

    let mut duplicate_tag = canonical_artifact();
    duplicate_tag
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .switch_tables[0]
        .cases[1] = SwitchCase {
        tag_type_ref: 0,
        target_pc: 13,
    };
    assert!(assert_rejected(&duplicate_tag)
        .to_string()
        .contains("strictly ascending"));
}

#[test]
fn corpus_rejects_wrong_relocation_and_pool_entry_kinds() {
    let mut relocation = canonical_artifact();
    relocation
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[0] = BytecodeRelocation::ServiceOperationRef {
        service_call: crate::ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: crate::ContractOperationId::new("operation:wrong"),
            expected_protocol_identity: crate::ServiceProtocolIdentity::new("protocol:wrong"),
        },
    };
    let error = assert_rejected(&relocation);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("not allowed"));

    let mut pool = canonical_artifact();
    pool.image.pools.constants[0] = BytecodePoolEntry::TypeRef { ty: string_type() };
    let error = assert_rejected(&pool);
    assert!(matches!(error, StructuralValidationError::Header { .. }));
    assert!(error.to_string().contains("incompatible entry kind"));
}

#[test]
fn corpus_validates_typed_frame_lengths_and_indices_fail_closed() {
    let mutations: [fn(&mut crate::bytecode::dto::FrameLayout); 5] = [
        |frame| frame.slot_type_refs.pop().map_or((), drop),
        |frame| frame.slot_type_refs.push(0),
        |frame| frame.result_type_refs.clear(),
        |frame| frame.result_type_refs.push(0),
        |frame| frame.slot_type_refs[0] = 99,
    ];
    for mutate in mutations {
        let mut artifact = canonical_artifact();
        mutate(
            &mut artifact
                .image
                .functions
                .get_mut("module::main")
                .unwrap()
                .frame_layout,
        );
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Table { .. }));
    }
}

#[test]
fn corpus_validates_positional_specialization_and_service_facts() {
    let mut missing_argument = canonical_artifact();
    let BytecodeRelocation::LocalExecutableRef { specialization, .. } = &mut missing_argument
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[0]
    else {
        unreachable!();
    };
    specialization.type_arguments.clear();
    let error = assert_rejected(&missing_argument);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("specialization arity"));

    let mut bad_slot = canonical_artifact();
    let BytecodeRelocation::ServiceOperationRef { service_call } = &mut bad_slot
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[2]
    else {
        unreachable!();
    };
    service_call.service_requirement_slot = limits::MAX_SERVICE_REQUIREMENTS as u32;
    assert!(assert_rejected(&bad_slot)
        .to_string()
        .contains("MAX_SERVICE_REQUIREMENTS"));

    let mut empty_operation = canonical_artifact();
    let BytecodeRelocation::ServiceOperationRef { service_call } = &mut empty_operation
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[2]
    else {
        unreachable!();
    };
    service_call.contract_operation_id = crate::ContractOperationId::new("");
    assert!(assert_rejected(&empty_operation)
        .to_string()
        .contains("must not be empty"));
}

#[test]
fn corpus_validates_resume_site_bijection_and_result_facts() {
    let mut unused = canonical_artifact();
    unused
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::main".to_string(),
            site_pc: 20,
            resume_pc: 25,
            end_resume_pc: None,
            expected_stack_height_before_result: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            result_materializations: Vec::new(),
            emit_stream_item_shape_ref: None,
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));
    assert!(assert_rejected(&unused)
        .to_string()
        .contains("exactly one pending site"));

    let mut wrong_site = canonical_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut wrong_site.image.pools.resume[0]
    else {
        unreachable!();
    };
    descriptor.site_pc = 25;
    assert!(assert_rejected(&wrong_site)
        .to_string()
        .contains("must bind this function/site"));

    let mut mismatched_result = canonical_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) =
        &mut mismatched_result.image.pools.resume[0]
    else {
        unreachable!();
    };
    descriptor.result_type_refs.clear();
    descriptor.result_plans.clear();
    descriptor.result_materializations.clear();
    assert!(assert_rejected(&mismatched_result)
        .to_string()
        .contains("result arity"));

    let mut mismatched_materializations = canonical_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) =
        &mut mismatched_materializations.image.pools.resume[0]
    else {
        unreachable!();
    };
    descriptor.result_materializations.clear();
    assert!(assert_rejected(&mismatched_materializations)
        .to_string()
        .contains("resultMaterializations"));

    let mut out_of_range_materialization = canonical_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) =
        &mut out_of_range_materialization.image.pools.resume[0]
    else {
        unreachable!();
    };
    descriptor.result_materializations = vec![Some(ResumeResultMaterialization::DenseRecord {
        shape_ref: u32::MAX,
    })];
    assert!(assert_rejected(&out_of_range_materialization)
        .to_string()
        .contains("resultMaterializations[0].shapeRef"));
}

#[test]
fn corpus_validates_nominal_shape_fields_and_plans() {
    let mut unsorted = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut unsorted.image.pools.shapes[0] else {
        unreachable!();
    };
    shape.fields.insert(
        0,
        ShapeFieldDeclaration {
            name: "z".to_string(),
            type_ref: 0,
            plan: snapshot_share(),
        },
    );
    assert!(assert_rejected(&unsorted)
        .to_string()
        .contains("strictly ascending"));

    let mut bad_nominal = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut bad_nominal.image.pools.shapes[0] else {
        unreachable!();
    };
    shape.type_ref = 99;
    assert!(assert_rejected(&bad_nominal)
        .to_string()
        .contains("types pool"));

    let mut bad_plan = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut bad_plan.image.pools.shapes[0] else {
        unreachable!();
    };
    shape.fields[0].plan = ValueTransferPlan::FromType {
        ty: crate::types::TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![string_type()],
        },
    };
    assert!(assert_rejected(&bad_plan)
        .to_string()
        .contains("ordinary shape fields require explicit non-recursive SnapshotShare plans"));
}

#[test]
fn corpus_validates_relational_constant_graph_and_reachability() {
    let mut cycle = canonical_artifact();
    cycle.image.frozen_constant_graph.nodes[1] = FrozenConstantNode::Array { children: vec![1] };
    assert!(assert_rejected(&cycle)
        .to_string()
        .contains("strictly less than parent"));

    let mut orphan = canonical_artifact();
    orphan
        .image
        .frozen_constant_graph
        .nodes
        .push(FrozenConstantNode::Literal {
            literal: crate::types::LiteralIr::Null,
        });
    assert!(assert_rejected(&orphan).to_string().contains("unreachable"));

    let mut missing_behavior = canonical_artifact();
    missing_behavior.image.frozen_constant_graph.nodes[4] = FrozenConstantNode::Implementation {
        record: 2,
        behaviors: vec![FrozenBehaviorBinding {
            function_key: "module::missing".to_string(),
        }],
    };
    assert!(assert_rejected(&missing_behavior)
        .to_string()
        .contains("missing function"));

    let mut duplicate_root = canonical_artifact();
    duplicate_root
        .image
        .pools
        .constants
        .push(BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index: 4 },
            type_ref: 0,
            plan: snapshot_share(),
        });
    assert_validates(&duplicate_root);
}

#[test]
fn corpus_allows_nested_and_adjacent_exception_regions_but_rejects_crossing() {
    let region = |start_pc, end_pc| ExceptionRegion {
        start_pc,
        end_pc,
        handler_pc: 27,
        handler_stack_height: 0,
        catch_matchers: vec![CatchMatcher::CatchAll],
        catch_slot: 1,
        catch_slot_type_ref: 0,
        cleanup_depth: 0,
    };

    let mut nested = canonical_artifact();
    nested
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .exception_regions = vec![region(0, 20), region(15, 18)];
    assert_validates(&nested);

    let mut adjacent = canonical_artifact();
    adjacent
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .exception_regions = vec![region(0, 15), region(15, 20)];
    assert_validates(&adjacent);

    let mut crossing = canonical_artifact();
    crossing
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .exception_regions = vec![region(0, 18), region(15, 20)];
    assert!(assert_rejected(&crossing).to_string().contains("crosses"));
}

#[test]
fn corpus_rejects_source_gaps_for_effectful_instructions() {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .source_map
        .clear();
    assert!(assert_rejected(&artifact)
        .to_string()
        .contains("requires exactly one source"));
}

#[test]
fn corpus_rejects_untyped_or_unplanned_constant_pool_entries() {
    let mut bad_type = canonical_artifact();
    let BytecodePoolEntry::ConstantRef { type_ref, .. } = &mut bad_type.image.pools.constants[0]
    else {
        unreachable!();
    };
    *type_ref = 99;
    assert!(assert_rejected(&bad_type)
        .to_string()
        .contains("types pool"));

    let mut bad_plan = canonical_artifact();
    let BytecodePoolEntry::ConstantRef { plan, .. } = &mut bad_plan.image.pools.constants[0] else {
        unreachable!();
    };
    *plan = ValueTransferPlan::ExplicitCloneLease {
        clone_adapter: crate::bytecode::dto::NativeValueAdapterRef {
            binding_key: String::new(),
        },
        drop: crate::bytecode::dto::ResourceDropPlan::ResourceTableRelease,
    };
    assert!(assert_rejected(&bad_plan)
        .to_string()
        .contains("bindingKey must not be empty"));
}

#[test]
fn corpus_bounds_constant_root_symbols() {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .constant_roots
        .insert("x".repeat((limits::MAX_DEBUG_STRING_BYTES + 1) as usize), 0);
    let error = assert_rejected(&artifact);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_DEBUG_STRING_BYTES"));
}

#[test]
fn corpus_identity_check_remains_reserved_for_artifact_identity() {
    let mut artifact = canonical_artifact();
    artifact.bytecode_identity.clear();
    assert_validates(&artifact);

    let reserved = StructuralValidationError::Identity {
        message: "reserved".to_string(),
    };
    assert!(reserved.to_string().contains("identity validation failed"));
}
