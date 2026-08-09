//! Malformed corpus (§5.2): ten corruption classes, each with a hand-written
//! (not encoder-generated) malformed fixture that must be rejected and a
//! positive fixture that must pass. Class 9 (identity/content mismatch) is
//! C9, implemented by artifact-identity; `StructuralValidationError::Identity`
//! is the reserved slot.

use crate::bytecode::dto::{
    BytecodePoolEntry, BytecodeRelocation, CatchMatcher, ExceptionRegion, FrozenConstantNode,
    SourceMapEntry,
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
