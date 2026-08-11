//! Limit boundary tests: every §4.2 constant gets an at-limit pass and an
//! above-limit rejection where the boundary is tractable in a unit test.
//!
//! Constants whose boundary needs multi-hundred-MB fixtures
//! (`MAX_ARTIFACT_BYTES` 256 MiB) or whose above-limit trigger is shadowed by
//! another check (`MAX_TABLE_ENTRIES`: 1M+1 table entries require 1M+1
//! instructions, which trips `MAX_WORDS_PER_FUNCTION` first) are pinned by
//! value in `limit_constants_match_design_table` and documented here.

use std::collections::BTreeMap;

use crate::bytecode::dto::{
    limits, BytecodeArtifact, BytecodeConstantRef, BytecodeFunctionOrigin, BytecodeImage,
    BytecodePoolEntry, BytecodePools, DebugBinding, DebugTable, FrameLayout, FrozenConstantGraph,
    FrozenConstantNode, RelocatableBytecodeFunction, SwitchCase, SwitchTable, BYTECODE_ISA_VERSION,
    BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use crate::bytecode::opcodes::opcode_table_fingerprint;
use crate::types::{LiteralIr, TypeRefIr};

use super::*;

/// Minimal artifact with the given functions and no pools/graph/debug table.
fn minimal_artifact(functions: BTreeMap<String, RelocatableBytecodeFunction>) -> BytecodeArtifact {
    let needs_type_entry = functions.values().any(|function| {
        !function.frame_layout.slot_type_refs.is_empty()
            || !function.frame_layout.result_type_refs.is_empty()
    });
    let mut pools = BytecodePools::default();
    if needs_type_entry {
        pools
            .types
            .push(BytecodePoolEntry::TypeRef { ty: string_type() });
    }
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry: crate::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: crate::value_lifecycle_policy_identity().clone(),
        host_effect_registry: crate::host_effect_registry_identity().clone(),
        intrinsic_registry: crate::intrinsic_registry_identity().clone(),
        platform_error_projection_registry: crate::current_platform_error_projection_registry_ref()
            .clone(),
        bytecode_identity: String::new(),
        image: BytecodeImage {
            functions,
            pools,
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    }
}

/// Function body of `instruction_count` `return` instructions (each one word,
/// no operands, source events or extra tables).
fn basic_function(
    key: &str,
    instruction_count: usize,
    slot_count: u32,
) -> RelocatableBytecodeFunction {
    let module_path = key.split_once("::").map_or("module", |(module, _)| module);
    let words = vec![descriptor_for_opcode(Opcode::Return).opcode.into(); instruction_count];
    RelocatableBytecodeFunction {
        function_key: key.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: crate::PackageExecutableCoordinate {
                file_ir_identity: format!("file-ir:{module_path}"),
                module_path: module_path.to_string(),
                executable_index: 0,
            },
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words,
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count,
            slot_type_refs: vec![0; slot_count as usize],
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            slot_plans: vec![snapshot_share(); slot_count as usize],
        },
        max_operand_depth: 0,
        effect_summary_ref: crate::PackageCallableId::new("operation:limit"),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn single_function_artifact(
    instruction_count: usize,
    slot_count: u32,
) -> (BytecodeArtifact, String) {
    let mut functions = BTreeMap::new();
    functions.insert(
        "module::f".to_string(),
        basic_function("module::f", instruction_count, slot_count),
    );
    (minimal_artifact(functions), "module::f".to_string())
}

/// The §4.2 constants are pinned to the approved design values.
#[test]
fn limit_constants_match_design_table() {
    assert_eq!(limits::MAX_ARTIFACT_BYTES, 256 * 1024 * 1024);
    assert_eq!(limits::MAX_FUNCTIONS, 100_000);
    assert_eq!(limits::MAX_WORDS_PER_FUNCTION, 1_000_000);
    assert_eq!(limits::MAX_RELOCATIONS_PER_FUNCTION, 100_000);
    assert_eq!(limits::MAX_SERVICE_REQUIREMENTS, 100_000);
    assert_eq!(limits::MAX_TABLE_ENTRIES, 1_000_000);
    assert_eq!(limits::MAX_POOL_ENTRIES, 1_000_000);
    assert_eq!(limits::MAX_SLOTS_PER_FRAME, 65_536);
    assert_eq!(limits::MAX_OPERAND_DEPTH, 65_536);
    assert_eq!(limits::MAX_ARITY, 256);
    assert_eq!(limits::MAX_RESULTS_PER_CALL, 1);
    assert_eq!(limits::MAX_NESTING_DEPTH, 64);
    assert_eq!(limits::MAX_CONSTANT_GRAPH_NODES, 1_000_000);
    assert_eq!(limits::MAX_CONSTANT_GRAPH_BYTES, 64 * 1024 * 1024);
    assert_eq!(limits::MAX_SWITCH_TABLE_TARGETS, 65_536);
    assert_eq!(limits::MAX_TYPE_PARAMETERS, 64);
    assert_eq!(limits::MAX_DEBUG_STRING_BYTES, 1024 * 1024);
    assert_eq!(limits::MAX_DEBUG_TABLE_BYTES, 64 * 1024 * 1024);
}

/// MAX_ARITY: count-class immediate at 256 passes, 257 is rejected.
#[test]
fn max_arity_boundary() {
    let words_at = |arg_count: u32| {
        let mut words = main_function_words();
        words[8] = arg_count; // call_local argCount operand
        words
    };
    let mut at_limit = canonical_artifact();
    at_limit
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words_at(256);
    assert_validates(&at_limit);

    let mut above = canonical_artifact();
    above.image.functions.get_mut("module::main").unwrap().words = words_at(257);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(error.to_string().contains("MAX_ARITY"), "{error}");
}

/// ISA v4 accepts only zero or one result from a non-tail call before the
/// semantic verifier cross-checks the linked callee signature.
#[test]
fn max_results_per_call_boundary() {
    let words_at = |result_count: u32| {
        let mut words = main_function_words();
        words[9] = result_count; // call_local resultCount operand
        words
    };

    let mut at_limit = canonical_artifact();
    at_limit
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .words = words_at(1);
    assert_validates(&at_limit);

    let mut above = canonical_artifact();
    above.image.functions.get_mut("module::main").unwrap().words = words_at(2);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Operand { .. }));
    assert!(
        error.to_string().contains("MAX_RESULTS_PER_CALL"),
        "{error}"
    );
}

/// MAX_SLOTS_PER_FRAME: slotCount 65_536 passes, 65_537 is rejected.
#[test]
fn max_slots_per_frame_boundary() {
    let at_limit = single_function_artifact(1, 65_536).0;
    assert_validates(&at_limit);

    let above = single_function_artifact(1, 65_537).0;
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_SLOTS_PER_FRAME"), "{error}");
}

/// MAX_OPERAND_DEPTH: declared maxOperandDepth 65_536 passes, 65_537 rejects.
#[test]
fn max_operand_depth_boundary() {
    let (mut at_limit, _) = single_function_artifact(1, 0);
    at_limit
        .image
        .functions
        .get_mut("module::f")
        .unwrap()
        .max_operand_depth = 65_536;
    assert_validates(&at_limit);

    let (mut above, _) = single_function_artifact(1, 0);
    above
        .image
        .functions
        .get_mut("module::f")
        .unwrap()
        .max_operand_depth = 65_537;
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_OPERAND_DEPTH"), "{error}");
}

/// MAX_TYPE_PARAMETERS: 64 type parameters pass, 65 are rejected.
#[test]
fn max_type_parameters_boundary() {
    let (mut at_limit, _) = single_function_artifact(1, 0);
    at_limit
        .image
        .functions
        .get_mut("module::f")
        .unwrap()
        .type_parameters = (0..64).map(|index| format!("T{index}")).collect();
    assert_validates(&at_limit);

    let (mut above, _) = single_function_artifact(1, 0);
    above
        .image
        .functions
        .get_mut("module::f")
        .unwrap()
        .type_parameters = (0..65).map(|index| format!("T{index}")).collect();
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_TYPE_PARAMETERS"), "{error}");
}

/// MAX_NESTING_DEPTH (constant graph): a chain of depth 64 passes, 65 rejects.
#[test]
fn max_nesting_depth_constant_graph_boundary() {
    let graph_at = |node_count: usize| -> FrozenConstantGraph {
        let mut nodes = vec![FrozenConstantNode::Literal {
            literal: LiteralIr::Null,
        }];
        for index in 1..node_count {
            nodes.push(FrozenConstantNode::Array {
                children: vec![(index - 1) as u32],
            });
        }
        FrozenConstantGraph { nodes }
    };

    let (mut at_limit, _) = single_function_artifact(1, 0);
    at_limit.image.frozen_constant_graph = graph_at(64);
    at_limit.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: string_type() }];
    at_limit.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
        reference: BytecodeConstantRef::LocalNode { node_index: 63 },
        type_ref: 0,
        plan: snapshot_share(),
    }];
    assert_validates(&at_limit);

    let (mut above, _) = single_function_artifact(1, 0);
    above.image.frozen_constant_graph = graph_at(65);
    above.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: string_type() }];
    above.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
        reference: BytecodeConstantRef::LocalNode { node_index: 64 },
        type_ref: 0,
        plan: snapshot_share(),
    }];
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_NESTING_DEPTH"), "{error}");
}

/// MAX_NESTING_DEPTH (type pool): nested type depth 64 passes, 65 rejects.
#[test]
fn max_nesting_depth_type_pool_boundary() {
    let nested = |depth: usize| -> TypeRefIr {
        let mut ty = string_type();
        for _ in 1..depth {
            ty = TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![ty],
            };
        }
        ty
    };

    let (mut at_limit, _) = single_function_artifact(1, 0);
    at_limit.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: nested(64) }];
    assert_validates(&at_limit);

    let (mut above, _) = single_function_artifact(1, 0);
    above.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: nested(65) }];
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_NESTING_DEPTH"), "{error}");
}

/// MAX_SWITCH_TABLE_TARGETS: 65_536 targets pass, 65_537 are rejected (needs a
/// function with that many headers).
#[test]
fn max_switch_table_targets_boundary() {
    let with_targets = |target_count: usize| -> BytecodeArtifact {
        let (mut artifact, _) = single_function_artifact(target_count + 1, 0);
        artifact.image.pools.types = (0..target_count.max(1))
            .map(|_| BytecodePoolEntry::TypeRef { ty: string_type() })
            .collect();
        artifact
            .image
            .functions
            .get_mut("module::f")
            .unwrap()
            .switch_tables = vec![SwitchTable {
            cases: (0..target_count as u32)
                .map(|index| SwitchCase {
                    tag_type_ref: index,
                    target_pc: index,
                })
                .collect(),
            default_pc: 0,
        }];
        artifact
    };

    assert_validates(&with_targets(65_536));

    let above = with_targets(65_537);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_SWITCH_TABLE_TARGETS"),
        "{error}"
    );
}

/// MAX_WORDS_PER_FUNCTION: 1_000_000 words pass, 1_000_001 are rejected.
#[test]
fn max_words_per_function_boundary() {
    let at_limit = single_function_artifact(1_000_000, 0).0;
    assert_validates(&at_limit);

    let above = single_function_artifact(1_000_001, 0).0;
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_WORDS_PER_FUNCTION"),
        "{error}"
    );
}

/// MAX_RELOCATIONS_PER_FUNCTION: 100_000 pass, 100_001 are rejected.
#[test]
fn max_relocations_per_function_boundary() {
    let with_relocations = |count: usize| -> BytecodeArtifact {
        let (mut artifact, _) = single_function_artifact(1, 0);
        artifact
            .image
            .functions
            .get_mut("module::f")
            .unwrap()
            .relocations = (0..count)
            .map(|_| crate::bytecode::dto::BytecodeRelocation::TypeRef { ty: string_type() })
            .collect();
        artifact
    };

    assert_validates(&with_relocations(100_000));

    let above = with_relocations(100_001);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_RELOCATIONS_PER_FUNCTION"),
        "{error}"
    );
}

/// MAX_FUNCTIONS: 100_000 functions pass, 100_001 are rejected.
#[test]
fn max_functions_boundary() {
    let with_functions = |count: usize| -> BytecodeArtifact {
        let mut functions = BTreeMap::new();
        for index in 0..count {
            let key = format!("module::f{index}");
            let mut function = basic_function(&key, 1, 0);
            let BytecodeFunctionOrigin::Executable { executable } = &mut function.origin else {
                unreachable!();
            };
            executable.executable_index = index as u32;
            functions.insert(key, function);
        }
        minimal_artifact(functions)
    };

    assert_validates(&with_functions(100_000));

    let above = with_functions(100_001);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_FUNCTIONS"), "{error}");
}

/// MAX_DEBUG_STRING_BYTES: a 1 MiB debug name passes, 1 MiB + 1 is rejected.
#[test]
fn max_debug_string_bytes_boundary() {
    let with_name_len = |name_len: usize| -> BytecodeArtifact {
        let mut artifact = canonical_artifact();
        artifact.image.debug_table = Some(DebugTable {
            bindings: vec![DebugBinding {
                function_key: "module::helper".to_string(),
                pc: 0,
                name: "a".repeat(name_len),
                slot: 0,
            }],
        });
        artifact
    };

    assert_validates(&with_name_len(1024 * 1024));

    let above = with_name_len(1024 * 1024 + 1);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_DEBUG_STRING_BYTES"),
        "{error}"
    );
}

/// MAX_DEBUG_TABLE_BYTES: 63 × 1 MiB bindings pass (63 MiB + overhead < 64
/// MiB), 65 × 1 MiB are rejected. An exact 64 MiB fixture is not used because
/// the serialization overhead pushes it over the ceiling.
#[test]
fn max_debug_table_bytes_boundary() {
    let with_bindings = |binding_count: usize| -> BytecodeArtifact {
        let mut artifact = canonical_artifact();
        artifact.image.debug_table = Some(DebugTable {
            bindings: (0..binding_count)
                .map(|_| DebugBinding {
                    function_key: "module::helper".to_string(),
                    pc: 0,
                    name: "a".repeat(1024 * 1024),
                    slot: 0,
                })
                .collect(),
        });
        artifact
    };

    assert_validates(&with_bindings(63));

    let above = with_bindings(65);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_DEBUG_TABLE_BYTES"),
        "{error}"
    );
}

/// MAX_CONSTANT_GRAPH_BYTES: a 63 MiB literal passes, 65 MiB is rejected.
#[test]
fn max_constant_graph_bytes_boundary() {
    let with_literal_len = |len: usize| -> BytecodeArtifact {
        let (mut artifact, _) = single_function_artifact(1, 0);
        artifact.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: string_type() }];
        artifact.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index: 0 },
            type_ref: 0,
            plan: snapshot_share(),
        }];
        artifact.image.frozen_constant_graph = FrozenConstantGraph {
            nodes: vec![FrozenConstantNode::Literal {
                literal: LiteralIr::String {
                    value: "a".repeat(len),
                },
            }],
        };
        artifact
    };

    assert_validates(&with_literal_len(63 * 1024 * 1024));

    let above = with_literal_len(65 * 1024 * 1024);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_CONSTANT_GRAPH_BYTES"),
        "{error}"
    );
}

/// MAX_POOL_ENTRIES: 1_000_000 type pool entries pass, 1_000_001 are rejected.
#[test]
fn max_pool_entries_boundary() {
    let with_pool_len = |len: usize| -> BytecodeArtifact {
        let (mut artifact, _) = single_function_artifact(1, 0);
        artifact.image.pools.types = (0..len)
            .map(|_| BytecodePoolEntry::TypeRef { ty: string_type() })
            .collect();
        artifact
    };

    assert_validates(&with_pool_len(1_000_000));

    let above = with_pool_len(1_000_001);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(error.to_string().contains("MAX_POOL_ENTRIES"), "{error}");
}

/// MAX_CONSTANT_GRAPH_NODES: 1_000_000 nodes pass, 1_000_001 are rejected.
#[test]
fn max_constant_graph_nodes_boundary() {
    let with_node_count = |count: usize| -> BytecodeArtifact {
        let (mut artifact, _) = single_function_artifact(1, 0);
        let mut nodes: Vec<_> = (0..count)
            .map(|_| FrozenConstantNode::Literal {
                literal: LiteralIr::Null,
            })
            .collect();
        if count > 1 {
            nodes[count - 1] = FrozenConstantNode::Array {
                children: (0..(count - 1) as u32).collect(),
            };
        }
        artifact.image.frozen_constant_graph = FrozenConstantGraph { nodes };
        if count > 0 {
            artifact.image.pools.types = vec![BytecodePoolEntry::TypeRef { ty: string_type() }];
            artifact.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
                reference: BytecodeConstantRef::LocalNode {
                    node_index: (count - 1) as u32,
                },
                type_ref: 0,
                plan: snapshot_share(),
            }];
        };
        artifact
    };

    assert_validates(&with_node_count(1_000_000));

    let above = with_node_count(1_000_001);
    let error = assert_rejected(&above);
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_CONSTANT_GRAPH_NODES"),
        "{error}"
    );
}

/// MAX_TABLE_ENTRIES: defense-in-depth. An above-limit table (1_000_001
/// statement entries) requires a function with 1_000_001 instruction headers,
/// which trips MAX_WORDS_PER_FUNCTION first, so the boundary is exercised via
/// the words limit; the table check itself is pinned by
/// `limit_constants_match_design_table`.
#[test]
fn max_table_entries_is_defense_in_depth() {
    // 1_000_001 table entries cannot exist without 1_000_001 instruction
    // headers, which trips MAX_WORDS_PER_FUNCTION first.
    let mut function = basic_function("module::f", 1, 0);
    function.words = vec![0x25; 1_000_001];
    let mut functions = BTreeMap::new();
    functions.insert("module::f".to_string(), function);
    let error = assert_rejected(&minimal_artifact(functions));
    assert!(matches!(error, StructuralValidationError::Limits { .. }));
    assert!(
        error.to_string().contains("MAX_WORDS_PER_FUNCTION"),
        "{error}"
    );
}

/// MAX_ARTIFACT_BYTES: the 256 MiB ceiling is not reachable with a tractable
/// unit fixture; the check itself is the same length comparison exercised by
/// every other limit test, and the constant is pinned in
/// `limit_constants_match_design_table`.
#[test]
fn max_artifact_bytes_constant_is_pinned() {
    assert_eq!(limits::MAX_ARTIFACT_BYTES, 256 * 1024 * 1024);
}
