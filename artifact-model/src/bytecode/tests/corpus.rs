//! Malformed corpus (§5.2): ten corruption classes, each with a hand-written
//! (not encoder-generated) malformed fixture that must be rejected and a
//! positive fixture that must pass. Class 9 (identity/content mismatch) is
//! C9, implemented by artifact-identity; `StructuralValidationError::Identity`
//! is the reserved slot.

use crate::bytecode::dto::{
    BytecodePoolEntry, BytecodeRelocation, CatchMatcher, ExceptionRegion, FrozenConstantNode,
    ResumeDescriptor, ShapeDeclaration, SourceMapEntry,
};

use super::*;

/// Class 1: unknown opcode (C1 high bits / C4 table lookup).
#[test]
fn corpus_unknown_opcode_negative_and_positive() {
    assert_validates(&canonical_artifact());

    let mut sentinel = canonical_artifact();
    sentinel
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words = vec![0xFF];
    let error = assert_rejected(&sentinel);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(error.to_string().contains("unknown opcode"), "{error}");

    let mut high_bits = canonical_artifact();
    high_bits
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words = vec![0x100];
    let error = assert_rejected(&high_bits);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));

    let mut reserved = canonical_artifact();
    reserved
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words = vec![0x90, 0x25];
    assert!(assert_rejected(&reserved)
        .to_string()
        .contains("unknown opcode"));
}

/// Class 2: truncated operands (C4).
#[test]
fn corpus_truncated_operands_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // `const` (0x00) requires one operand word; only the header is present.
    let mut truncated = canonical_artifact();
    truncated
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words = vec![0x00];
    let error = assert_rejected(&truncated);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(
        error.to_string().contains("truncated instruction"),
        "{error}"
    );

    // `call_local` (3 words) with only two words available.
    let mut call_truncated = canonical_artifact();
    call_truncated
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words = vec![0x20, 0];
    assert!(matches!(
        assert_rejected(&call_truncated),
        StructuralValidationError::Decode { .. }
    ));
}

/// Class 3: jump lands in an operand word (C6).
#[test]
fn corpus_jump_into_operand_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // jump at pc 13 with delta 1 targets pc 14 — an operand word of
    // enter_region, not a header.
    let mut words = main_function_words();
    words[14] = 1;
    let mut jump_into_operand = canonical_artifact();
    jump_into_operand
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words;
    let error = assert_rejected(&jump_into_operand);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");
}

/// Class 4: index out of bounds (C5) — slot, pool, relocation and table.
#[test]
fn corpus_index_out_of_bounds_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // Slot: store_slot slot4, frame has 4 slots.
    let mut slot = canonical_artifact();
    let mut words = main_function_words();
    words[3] = 4;
    slot.image.functions.get_mut("module::main").unwrap().words = words;
    let error = assert_rejected(&slot);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("slot index"), "{error}");

    // Pool: const pool index 5, constants pool has 1 entry.
    let mut pool = canonical_artifact();
    let mut words = main_function_words();
    words[1] = 5;
    pool.image.functions.get_mut("module::main").unwrap().words = words;
    let error = assert_rejected(&pool);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("pool index"), "{error}");

    // Relocation: call_local reloc 7, function has 3 relocations.
    let mut relocation = canonical_artifact();
    let mut words = main_function_words();
    words[7] = 7;
    relocation
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words;
    let error = assert_rejected(&relocation);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("relocation index"), "{error}");

    // Table: switch_tag table 3, function has 1 switch table.
    let mut table = canonical_artifact();
    let mut words = main_function_words();
    words[11] = 3;
    table.image.functions.get_mut("module::main").unwrap().words = words;
    let error = assert_rejected(&table);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("table index"), "{error}");
}

/// Class 5: wrong relocation kind / pool entry kind (C5).
#[test]
fn corpus_wrong_relocation_kind_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // call_local only admits LocalExecutableRef/PackageCallableRef.
    let mut wrong_kind = canonical_artifact();
    wrong_kind
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[0] = BytecodeRelocation::ServiceOperationRef {
        operation_abi_id: "operation:svc:call".to_string(),
    };
    let error = assert_rejected(&wrong_kind);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("not allowed"), "{error}");

    // const operand requires a FrozenConstantRef entry.
    let mut wrong_entry = canonical_artifact();
    wrong_entry.image.pools.constants[0] = BytecodePoolEntry::TypeRef { ty: string_type() };
    let error = assert_rejected(&wrong_entry);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("entry kind mismatch"), "{error}");
}

/// Class 6: overlapping exception/source ranges (C7).
#[test]
fn corpus_overlapping_ranges_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // Second exception region starts inside the first region [15, 20).
    let mut overlapping_region = canonical_artifact();
    overlapping_region
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .exception_regions
        .push(ExceptionRegion {
            start_pc: 17,
            end_pc: 20,
            handler_pc: 26,
            handler_stack_height: 0,
            catch_matchers: vec![CatchMatcher::CatchAll],
            catch_slot: 1,
            cleanup_depth: 0,
        });
    let error = assert_rejected(&overlapping_region);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("overlaps"), "{error}");

    // Second source map entry overlaps the first ([0, 6)).
    let mut overlapping_source = canonical_artifact();
    overlapping_source
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .source_map[1] = SourceMapEntry {
        start: 4,
        end: 10,
        source_id: 0,
        start_position: crate::SourcePosition::new(3, 1),
        end_position: crate::SourcePosition::new(5, 1),
    };
    let error = assert_rejected(&overlapping_source);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("overlaps"), "{error}");
}

/// Class 7: cyclic/oversized constant graph (C8). Acyclicity is a pure format
/// constraint (`child index < parent index`), so a "cycle" is any child index
/// >= its parent index.
#[test]
fn corpus_constant_graph_cycle_negative_and_positive() {
    assert_validates(&canonical_artifact());

    let mut self_referencing = canonical_artifact();
    self_referencing.image.frozen_constant_graph.nodes[1] =
        FrozenConstantNode::Array { children: vec![1] };
    let error = assert_rejected(&self_referencing);
    assert!(matches!(
        error,
        StructuralValidationError::ConstantGraph { .. }
    ));
    assert!(
        error.to_string().contains("strictly less than parent"),
        "{error}"
    );

    // Forward reference (child index > parent index).
    let mut forward = canonical_artifact();
    forward.image.frozen_constant_graph.nodes[0] = FrozenConstantNode::Array { children: vec![4] };
    assert!(matches!(
        assert_rejected(&forward),
        StructuralValidationError::ConstantGraph { .. }
    ));

    // Behavior node referencing a missing function (C8 pool/function refs).
    let mut missing_function = canonical_artifact();
    missing_function.image.frozen_constant_graph.nodes[4] = FrozenConstantNode::Behavior {
        function_key: "module::missing".to_string(),
    };
    let error = assert_rejected(&missing_function);
    assert!(error.to_string().contains("missing function"), "{error}");
}

/// Class 8: count/offset arithmetic overflow (C3).
#[test]
fn corpus_arithmetic_overflow_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // jump_if_false at pc 24 with delta word 0x8000_0000 (i32::MIN) underflows
    // the checked branch target arithmetic.
    let mut words = main_function_words();
    words[25] = 0x8000_0000;
    let mut overflow = canonical_artifact();
    overflow
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words;
    let error = assert_rejected(&overflow);
    assert!(matches!(
        error,
        StructuralValidationError::Arithmetic { .. }
    ));
    assert!(error.to_string().contains("overflow"), "{error}");
}

/// Class 10: total resource limits (C2). Per-constant boundaries live in
/// `limits.rs`; this is one cheap end-to-end representative.
#[test]
fn corpus_resource_limit_negative_and_positive() {
    assert_validates(&canonical_artifact());

    // Count-class immediate above MAX_ARITY.
    let mut words = main_function_words();
    words[8] = 257;
    let mut arity = canonical_artifact();
    arity.image.functions.get_mut("module::main").unwrap().words = words;
    let error = assert_rejected(&arity);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("MAX_ARITY"), "{error}");
}

/// Class 9 (identity/content mismatch) is C9: reserved for the
/// artifact-identity task. The reserved error slot exists but is never
/// constructed by `structurally_validate`.
#[test]
fn corpus_identity_mismatch_is_reserved_for_artifact_identity() {
    let reserved = StructuralValidationError::Identity {
        message: "reserved".to_string(),
    };
    assert!(reserved.to_string().contains("identity validation failed"));
}

// ---------------------------------------------------------------------------
// Extension: ≥2 new variants per corruption class (§5.2/§8), exercising
// different positions and boundary combinations. `structurally_validate`
// (C1–C8) is the only rejection oracle here; C9 is out of this crate.
// ---------------------------------------------------------------------------

/// Class 1 variants: 0x06 (unassigned inside the Value/slot family) and
/// 0xFE (top of the reserved range) are both unknown opcodes.
#[test]
fn corpus_unknown_opcode_family_gap_and_reserved_top() {
    let gap = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x06, 0x25];
        artifact
    };
    let error = assert_rejected(&gap);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(error.to_string().contains("unknown opcode"), "{error}");

    let reserved_top = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0xFE, 0x25];
        artifact
    };
    let error = assert_rejected(&reserved_top);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(error.to_string().contains("unknown opcode"), "{error}");
}

/// Class 1 variant: unknown opcode in the middle of a function body, after
/// several instructions have already been decoded (words[9] is the
/// budget_checkpoint header at pc 9; the shifted re-parse still lands on it).
#[test]
fn corpus_unknown_opcode_in_function_middle() {
    let mut words = main_function_words();
    words[9] = 0x1F; // reserved Control-family slot
    let mut artifact = canonical_artifact();
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words;
    let error = assert_rejected(&artifact);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(error.to_string().contains("unknown opcode"), "{error}");
}

/// Class 2 variants: truncation at the end of a multi-instruction body,
/// truncation of the largest layout (invoke_host, 4 words), truncation on
/// the very last word, and the empty body as the positive boundary.
#[test]
fn corpus_truncated_operands_positions_and_positive_empty() {
    // call_local at the second-to-last word: header + 1 of 2 operands.
    let end_truncated = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x14, 0x20, 0];
        artifact
    };
    let error = assert_rejected(&end_truncated);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(
        error.to_string().contains("truncated instruction"),
        "{error}"
    );

    // const header on the very last word, operand missing.
    let last_word = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x14, 0x14, 0x00];
        artifact
    };
    let error = assert_rejected(&last_word);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(
        error.to_string().contains("truncated instruction"),
        "{error}"
    );

    // invoke_host needs 3 operands; only 2 available.
    let biggest_layout = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x80, 0, 0];
        artifact
    };
    let error = assert_rejected(&biggest_layout);
    assert!(matches!(error, StructuralValidationError::Decode { .. }));
    assert!(
        error.to_string().contains("truncated instruction"),
        "{error}"
    );

    // Positive boundary: an empty body is a legal zero-instruction function.
    // The canonical debug table references helper pc 0, so it must go too.
    let empty = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = Vec::new();
        artifact.image.debug_table = None;
        artifact
    };
    assert_validates(&empty);
}

/// Class 3 variants: negative branch delta landing in an operand word of a
/// previous instruction, and a branch target beyond the function end.
#[test]
fn corpus_branch_targets_extremes() {
    // jump_if_false at pc 24, delta -5: target 21 is an operand word of
    // call_service at pc 20.
    let negative_into_operand = {
        let mut words = main_function_words();
        words[25] = 0xFFFF_FFFB;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        artifact
    };
    let error = assert_rejected(&negative_into_operand);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");

    // jump at pc 0 with delta 3: target 5 is past the 2-word body.
    let beyond_end = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x10, 3];
        artifact
    };
    let error = assert_rejected(&beyond_end);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");
}

/// Class 3 variants: switch table target and exception handler pc in operand
/// words (C7 target membership), and leave_region outside its region (D13).
#[test]
fn corpus_table_targets_and_region_membership_variants() {
    let switch_target_in_operand = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .switch_tables[0]
            .targets = vec![4, 21];
        artifact
    };
    let error = assert_rejected(&switch_target_in_operand);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");

    let handler_in_operand = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .exception_regions[0]
            .handler_pc = 21;
        artifact
    };
    let error = assert_rejected(&handler_in_operand);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");

    // leave_region at pc 18 must live inside region [15, end_pc); shrink the
    // region to [15, 17) so the instruction falls outside it.
    let leave_outside = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .exception_regions[0]
            .end_pc = 17;
        artifact
    };
    let error = assert_rejected(&leave_outside);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(
        error.to_string().contains("outside referenced region"),
        "{error}"
    );
}

/// Class 4 variants: extreme indices — u32::MAX slot, one-past-end and
/// near-max relocation, near-max table, one-past-end and max resume pool.
#[test]
fn corpus_index_out_of_bounds_extremes() {
    // Slot at u32::MAX.
    let slot_max = {
        let mut words = main_function_words();
        words[3] = 0xFFFF_FFFF;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        artifact
    };
    let error = assert_rejected(&slot_max);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("slot index"), "{error}");

    // Relocation one-past-end (3 relocations) and near-max.
    for word in [3u32, 0x7FFF_FFFF] {
        let mut words = main_function_words();
        words[7] = word;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Operand { .. }));
        assert!(error.to_string().contains("relocation index"), "{error}");
    }

    // Switch table index near-max.
    let table_max = {
        let mut words = main_function_words();
        words[11] = 0x7FFF_FFFF;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        artifact
    };
    let error = assert_rejected(&table_max);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("table index"), "{error}");

    // Resume pool one-past-end (1 entry) and max (call_service resumeRef).
    for word in [1u32, 0xFFFF_FFFF] {
        let mut words = main_function_words();
        words[23] = word;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Operand { .. }));
        assert!(error.to_string().contains("pool index"), "{error}");
    }
}

/// Class 5 variants: wrong relocation kind on call_actor/invoke_host/
/// call_service, and pool entry kind mismatches for the resume and constants
/// pools.
#[test]
fn corpus_wrong_relocation_and_pool_kind_variants() {
    // call_service word replaced by call_actor: reloc[2] is
    // ServiceOperationRef, call_actor only admits ActorMethodRef.
    let call_actor_wrong = {
        let mut words = main_function_words();
        words[20] = 0x23;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        artifact
    };
    let error = assert_rejected(&call_actor_wrong);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("not allowed"), "{error}");

    // call_service word replaced by invoke_host: reloc[2] must be
    // HostEffectRef.
    let invoke_host_wrong = {
        let mut words = main_function_words();
        words[20] = 0x80;
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .words = words;
        artifact
    };
    let error = assert_rejected(&invoke_host_wrong);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("not allowed"), "{error}");

    // call_service reloc itself wrong: ActorMethodRef is not in the allowed
    // set {ServiceOperationRef}.
    let service_wrong = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .relocations[2] = BytecodeRelocation::ActorMethodRef {
            method_abi_id: "method:x".to_string(),
        };
        artifact
    };
    let error = assert_rejected(&service_wrong);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("not allowed"), "{error}");

    // Resume pool entry is not a ResumeDescriptor.
    let resume_pool_kind = {
        let mut artifact = canonical_artifact();
        artifact.image.pools.resume[0] = BytecodePoolEntry::TypeRef { ty: string_type() };
        artifact
    };
    let error = assert_rejected(&resume_pool_kind);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("entry kind mismatch"), "{error}");

    // Constants pool entry is not a FrozenConstantRef.
    let constants_pool_kind = {
        let mut artifact = canonical_artifact();
        artifact.image.pools.constants[0] = BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                field_count: 0,
                field_types: Vec::new(),
            },
        };
        artifact
    };
    let error = assert_rejected(&constants_pool_kind);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("entry kind mismatch"), "{error}");
}

/// Class 6 variants: inverted exception region, duplicate statement pc,
/// source map range violations, and switch table tag index out of bounds.
#[test]
fn corpus_range_structure_variants() {
    // Inverted exception region: startPc >= endPc. Built on the helper
    // function (which has no enter/leave_region instructions): the C6 region
    // membership check would otherwise fire first on main's region ops.
    let inverted_region = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .exception_regions = vec![ExceptionRegion {
            start_pc: 10,
            end_pc: 9,
            handler_pc: 0,
            handler_stack_height: 0,
            catch_matchers: Vec::new(),
            catch_slot: 0,
            cleanup_depth: 0,
        }];
        artifact
    };
    let error = assert_rejected(&inverted_region);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("startPc"), "{error}");

    // Duplicate statement pc: not strictly ascending.
    let duplicate_statement = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .statement_entries[1]
            .pc = 0;
        artifact
    };
    let error = assert_rejected(&duplicate_statement);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("strictly ascending"), "{error}");

    // Source map entry ends beyond the function word range (27 words).
    let source_beyond = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .source_map[1]
            .end = 28;
        artifact
    };
    let error = assert_rejected(&source_beyond);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(
        error.to_string().contains("outside function word range"),
        "{error}"
    );

    // Inverted source map entry: start >= end.
    let inverted_source = {
        let mut artifact = canonical_artifact();
        let entry = &mut artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .source_map[1];
        entry.start = 10;
        entry.end = 7;
        artifact
    };
    let error = assert_rejected(&inverted_source);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("start"), "{error}");

    // Switch table tag pool index out of bounds of the types pool.
    let tag_oob = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .switch_tables[0]
            .tag_pool_index = 5;
        artifact
    };
    let error = assert_rejected(&tag_oob);
    assert!(matches!(error, StructuralValidationError::Header { .. }));
    assert!(error.to_string().contains("out of bounds"), "{error}");
}

/// Class 6 positive variant: adjacent (non-overlapping) exception regions are
/// legal — a second region may start exactly where the previous one ends.
#[test]
fn corpus_adjacent_exception_regions_are_legal() {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .exception_regions
        .push(ExceptionRegion {
            start_pc: 20,
            end_pc: 24,
            handler_pc: 26,
            handler_stack_height: 0,
            catch_matchers: vec![CatchMatcher::CatchAll],
            catch_slot: 1,
            cleanup_depth: 0,
        });
    assert_validates(&artifact);
}

/// Class 7 variants: child index beyond the graph, Record shape index and
/// TypeRef node index out of bounds, and an empty behavior function key.
#[test]
fn corpus_constant_graph_boundary_variants() {
    // Child index 5 >= parent index 0 violates the acyclicity encoding (and
    // would be out of bounds anyway).
    let child_beyond = {
        let mut artifact = canonical_artifact();
        artifact.image.frozen_constant_graph.nodes[0] =
            FrozenConstantNode::Array { children: vec![5] };
        artifact
    };
    let error = assert_rejected(&child_beyond);
    assert!(matches!(
        error,
        StructuralValidationError::ConstantGraph { .. }
    ));
    assert!(
        error.to_string().contains("strictly less than parent"),
        "{error}"
    );

    // Record shape index out of bounds of the shapes pool.
    let record_shape_oob = {
        let mut artifact = canonical_artifact();
        artifact.image.frozen_constant_graph.nodes[2] = FrozenConstantNode::Record {
            shape_index: 5,
            children: vec![0],
        };
        artifact
    };
    let error = assert_rejected(&record_shape_oob);
    assert!(matches!(error, StructuralValidationError::Header { .. }));
    assert!(error.to_string().contains("shapes pool"), "{error}");

    // TypeRef node index out of bounds of the types pool.
    let type_ref_oob = {
        let mut artifact = canonical_artifact();
        artifact.image.frozen_constant_graph.nodes[3] = FrozenConstantNode::TypeRef { type_ref: 5 };
        artifact
    };
    let error = assert_rejected(&type_ref_oob);
    assert!(matches!(error, StructuralValidationError::Header { .. }));
    assert!(error.to_string().contains("types pool"), "{error}");

    // Behavior node with an empty function key: missing function.
    let empty_behavior = {
        let mut artifact = canonical_artifact();
        artifact.image.frozen_constant_graph.nodes[4] = FrozenConstantNode::Behavior {
            function_key: String::new(),
        };
        artifact
    };
    let error = assert_rejected(&empty_behavior);
    assert!(matches!(
        error,
        StructuralValidationError::ConstantGraph { .. }
    ));
    assert!(error.to_string().contains("missing function"), "{error}");
}

/// Class 8 variants: branch delta at both extremes from pc 0 (base 2) —
/// i32::MIN and a small negative delta underflow checked arithmetic, while
/// i32::MAX overflows past the function into a C6 target failure.
#[test]
fn corpus_branch_delta_extremes() {
    // delta = -3: 2 - 3 underflows below zero.
    let negative_underflow = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x10, 0xFFFF_FFFD];
        artifact
    };
    let error = assert_rejected(&negative_underflow);
    assert!(matches!(
        error,
        StructuralValidationError::Arithmetic { .. }
    ));
    assert!(error.to_string().contains("overflow"), "{error}");

    // delta = i32::MIN: 2 - 2^31 underflows.
    let min_delta = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x10, 0x8000_0000];
        artifact
    };
    let error = assert_rejected(&min_delta);
    assert!(matches!(
        error,
        StructuralValidationError::Arithmetic { .. }
    ));
    assert!(error.to_string().contains("overflow"), "{error}");

    // delta = i32::MAX: arithmetic succeeds but target 0x8000_0001 is not a
    // header — a C6 failure, not an overflow.
    let max_delta = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::helper")
            .unwrap()
            .words = vec![0x10, 0x7FFF_FFFF];
        artifact
    };
    let error = assert_rejected(&max_delta);
    assert!(matches!(error, StructuralValidationError::Target { .. }));
    assert!(error.to_string().contains("instruction header"), "{error}");
}

/// Class 9 (identity/content mismatch) is C9, implemented by
/// artifact-identity. `structurally_validate` (C1–C8) deliberately does not
/// check the declared identity; these variants pin that boundary: any
/// declared value — including an empty or arbitrary one — passes C1–C8 and
/// is rejected only by the artifact-identity admit path.
#[test]
fn corpus_identity_mismatch_is_deferred_to_artifact_identity() {
    let empty_identity = {
        let mut artifact = canonical_artifact();
        artifact.bytecode_identity = String::new();
        artifact
    };
    assert_validates(&empty_identity);

    let arbitrary_identity = {
        let mut artifact = canonical_artifact();
        artifact.bytecode_identity = "skiff-bytecode-image-v1:sha256:deadbeef".to_string();
        artifact
    };
    assert_validates(&arbitrary_identity);
}

/// Class 10 variants: per-position resource limits beyond the MAX_ARITY case
/// — exception handler stack height, cleanup depth, resume expected stack
/// height and a debug statement id (C2).
#[test]
fn corpus_resource_limit_position_variants() {
    // Exception region handler stack height above MAX_OPERAND_DEPTH.
    let handler_height = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .exception_regions[0]
            .handler_stack_height = 65_537;
        artifact
    };
    let error = assert_rejected(&handler_height);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_OPERAND_DEPTH"), "{error}");

    // Exception region cleanup depth above MAX_OPERAND_DEPTH.
    let cleanup_depth = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .exception_regions[0]
            .cleanup_depth = 65_537;
        artifact
    };
    let error = assert_rejected(&cleanup_depth);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_OPERAND_DEPTH"), "{error}");

    // Resume descriptor expected stack height above MAX_OPERAND_DEPTH.
    let resume_height = {
        let mut artifact = canonical_artifact();
        artifact.image.pools.resume[0] = BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            result_type_ref: 1,
            expected_stack_height: 65_537,
            result_plan: snapshot_share(),
        });
        artifact
    };
    let error = assert_rejected(&resume_height);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_OPERAND_DEPTH"), "{error}");

    // Statement id above MAX_DEBUG_STRING_BYTES.
    let long_statement = {
        let mut artifact = canonical_artifact();
        artifact
            .image
            .functions
            .get_mut("module::main")
            .unwrap()
            .statement_entries[1]
            .statement_id = "a".repeat(1024 * 1024 + 1);
        artifact
    };
    let error = assert_rejected(&long_statement);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_DEBUG_STRING_BYTES"),
        "{error}"
    );
}

/// Typed frame declarations are complete and every entry is a checked index
/// into the artifact's types pool.
#[test]
fn corpus_frame_type_refs_fail_closed() {
    let mut short_slots = canonical_artifact();
    short_slots
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .slot_type_refs
        .pop();
    let error = assert_rejected(&short_slots);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("slotTypeRefs len"), "{error}");

    let mut short_results = canonical_artifact();
    short_results
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .result_type_refs
        .clear();
    let error = assert_rejected(&short_results);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("resultTypeRefs len"), "{error}");

    let mut slot_out_of_bounds = canonical_artifact();
    slot_out_of_bounds
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .slot_type_refs[0] = 2;
    let error = assert_rejected(&slot_out_of_bounds);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("out of bounds"), "{error}");

    let mut result_out_of_bounds = canonical_artifact();
    result_out_of_bounds
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .result_type_refs[0] = 2;
    let error = assert_rejected(&result_out_of_bounds);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("out of bounds"), "{error}");

    let mut wrong_entry_kind = canonical_artifact();
    wrong_entry_kind
        .image
        .pools
        .types
        .push(BytecodePoolEntry::HostEffectRef {
            effect_ref: "effect:not-a-type".to_string(),
        });
    wrong_entry_kind
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .result_type_refs[0] = 2;
    let error = assert_rejected(&wrong_entry_kind);
    assert!(matches!(error, StructuralValidationError::Table { .. }));
    assert!(error.to_string().contains("TypeRef entry"), "{error}");
}
