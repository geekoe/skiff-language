//! Roundtrip and determinism: encode → decode equivalence, canonical bytes
//! stability, BTreeMap insertion-order insensitivity (阶段页 §4.2 验收).

use std::collections::{BTreeMap, HashSet};

use crate::bytecode::decode::{decode_branch_target, BoundedDecoder};
use crate::bytecode::encode::{
    assemble_artifact, assemble_function, encode_instruction, EncodeError, EncodedInstruction,
};
use crate::bytecode::opcodes::{
    descriptor_for_opcode, opcode_for, opcode_kind, opcode_table_fingerprint, Arity, Opcode,
    OperandRole, OPCODE_TABLE,
};

use super::*;

/// Every opcode of the table: encode_instruction → decode_function yields the
/// same opcode with the same operand words.
#[test]
fn every_opcode_round_trips_through_encode_and_decode() {
    let decoder = BoundedDecoder::new();
    for descriptor in OPCODE_TABLE {
        let operand_count = descriptor.operand_word_count() as usize;
        let operands: Vec<u32> = (0..operand_count as u32).collect();
        let words = encode_instruction(descriptor.opcode, &operands).expect("encode");
        assert_eq!(words.len(), descriptor.instruction_word_count() as usize);
        let decoded = decoder
            .decode_function(&words)
            .expect("decode of encoded instruction must succeed");
        assert_eq!(decoded.instructions.len(), 1);
        let instruction = &decoded.instructions[0];
        assert_eq!(instruction.pc, 0);
        assert_eq!(instruction.descriptor.opcode, descriptor.opcode);
        assert_eq!(instruction.operand_words, operands);
    }
}

/// assemble_function produces exactly the hand-written canonical wordcode,
/// and decoding it reproduces the encoded instructions.
#[test]
fn assemble_function_matches_hand_written_wordcode() {
    let instructions = vec![
        EncodedInstruction::new(0x00, vec![0]),
        EncodedInstruction::new(0x03, vec![0]),
        EncodedInstruction::new(0x11, vec![0]),
        EncodedInstruction::new(0x20, vec![0, 0, 0]),
        EncodedInstruction::new(0x14, vec![]),
        EncodedInstruction::new(0x13, vec![0]),
        EncodedInstruction::new(0x10, vec![0]),
        EncodedInstruction::new(0x72, vec![0]),
        EncodedInstruction::new(0x14, vec![]),
        EncodedInstruction::new(0x73, vec![0]),
        EncodedInstruction::new(0x22, vec![2, 0, 1, 0]),
        EncodedInstruction::new(0x11, vec![0xFFFF_FFEB]),
        EncodedInstruction::new(0x25, vec![]),
    ];
    let words = assemble_function(&instructions).expect("assemble");
    assert_eq!(words, main_function_words());

    let decoded = BoundedDecoder::new()
        .decode_function(&words)
        .expect("decode");
    assert_eq!(
        decoded
            .instructions
            .iter()
            .map(|instruction| (
                instruction.descriptor.opcode,
                instruction.operand_words.clone()
            ))
            .collect::<Vec<_>>(),
        instructions
            .iter()
            .map(|instruction| (instruction.opcode, instruction.operands.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        decoded.header_pcs,
        vec![0, 2, 4, 6, 10, 11, 13, 15, 17, 18, 20, 25, 27]
    );
}

/// encode_instruction rejects unknown opcodes and wrong operand counts.
#[test]
fn encode_rejects_unknown_opcode_and_operand_count_mismatch() {
    assert_eq!(
        encode_instruction(0xFF, &[]),
        Err(EncodeError::UnknownOpcode(0xFF))
    );
    assert_eq!(
        encode_instruction(0x9C, &[]),
        Err(EncodeError::UnknownOpcode(0x9C))
    );
    assert_eq!(
        encode_instruction(0x10, &[0, 1]),
        Err(EncodeError::OperandCountMismatch {
            opcode: 0x10,
            expected: 1,
            actual: 2
        })
    );
    assert_eq!(
        encode_instruction(0x14, &[0]),
        Err(EncodeError::OperandCountMismatch {
            opcode: 0x14,
            expected: 0,
            actual: 1
        })
    );
}

/// Canonical bytes round trip through JSON back to the identical artifact.
#[test]
fn assemble_artifact_round_trips_through_json() {
    let artifact = canonical_artifact();
    let bytes = assemble_artifact(&artifact).expect("canonical bytes");
    let decoded: BytecodeArtifact =
        serde_json::from_slice(&bytes).expect("canonical bytes must be valid artifact JSON");
    assert_eq!(decoded, artifact);
    assert_eq!(
        decoded.image.functions["module::main"].effect_summary_ref,
        crate::PackageCallableId::new("operation:module:main")
    );
}

/// Same typed input built with different map insertion orders yields the same
/// canonical bytes (BTreeMap ordering).
#[test]
fn canonical_bytes_are_insertion_order_insensitive() {
    let mut first = BTreeMap::new();
    first.insert("module::main".to_string(), main_function());
    first.insert("module::helper".to_string(), helper_function());
    first.insert("module::main$callback0".to_string(), callback_function());
    let mut second = BTreeMap::new();
    second.insert("module::helper".to_string(), helper_function());
    second.insert("module::main".to_string(), main_function());
    second.insert("module::main$callback0".to_string(), callback_function());

    let bytes_of = |functions: BTreeMap<String, RelocatableBytecodeFunction>| {
        let mut artifact = canonical_artifact();
        artifact.image.functions = functions;
        assemble_artifact(&artifact).expect("canonical bytes")
    };
    assert_eq!(bytes_of(first), bytes_of(second));
}

/// Building the same artifact twice yields identical bytes.
#[test]
fn identical_inputs_produce_identical_bytes() {
    assert_eq!(
        assemble_artifact(&canonical_artifact()).expect("first build"),
        assemble_artifact(&canonical_artifact()).expect("second build")
    );
}

/// Decode is total on word sequences (bounded length): any failure is one of
/// the four structured error kinds, never a panic.
#[test]
fn decode_never_panics_and_always_reports_structured_errors() {
    let decoder = BoundedDecoder::new();
    let mut state = 0x1234_5678u32;
    let word = |state: &mut u32| {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *state >> 8
    };
    for _ in 0..500 {
        let length = (state % 64) as usize;
        let words: Vec<u32> = (0..length).map(|_| word(&mut state)).collect();
        match decoder.decode_function(&words) {
            Ok(decoded) => {
                assert_eq!(decoded.instructions.len(), decoded.header_pcs.len());
                for instruction in &decoded.instructions {
                    assert_eq!(
                        instruction.operand_words.len(),
                        instruction.descriptor.operand_word_count() as usize
                    );
                }
            }
            Err(error) => {
                let text = error.to_string();
                assert!(
                    text.contains("unknown opcode")
                        || text.contains("truncated instruction")
                        || text.contains("arithmetic overflow")
                        || text.contains("limit"),
                    "unexpected decode error: {text}"
                );
            }
        }
    }
}

/// decode_branch_target is overflow-safe in both directions.
#[test]
fn branch_target_decode_is_overflow_safe() {
    assert_eq!(decode_branch_target(0, 1, 0), Some(2));
    assert_eq!(decode_branch_target(4, 1, 0xFFFF_FFFF), Some(5));
    assert_eq!(decode_branch_target(0, 1, 0x7FFF_FFFF), Some(0x8000_0001));
    assert_eq!(decode_branch_target(0, 1, 0x8000_0000), None);
    assert_eq!(decode_branch_target(u32::MAX, 3, 0x7FFF_FFFF), None);
}

/// The table is ascending, complete (63 rows) and lookup-consistent.
#[test]
fn opcode_table_is_complete_sorted_and_lookup_consistent() {
    let expected: Vec<u8> = vec![
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
        0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x30, 0x31, 0x32, 0x33, 0x40, 0x41, 0x42, 0x43,
        0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x60, 0x61,
        0x70, 0x71, 0x72, 0x73, 0x80, 0x81, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98,
        0x99, 0x9A, 0x9B,
    ];
    assert_eq!(OPCODE_TABLE.len(), 63);
    assert_eq!(
        OPCODE_TABLE
            .iter()
            .map(|descriptor| descriptor.opcode)
            .collect::<Vec<_>>(),
        expected
    );
    for &opcode in &expected {
        assert_eq!(
            opcode_for(opcode).map(|descriptor| descriptor.opcode),
            Some(opcode)
        );
    }
    for forbidden in [
        0x09, 0x0F, 0x16, 0x1F, 0x27, 0x34, 0x44, 0x5D, 0x62, 0x74, 0x82, 0x8F, 0x9C, 0xFF,
    ] {
        assert_eq!(
            opcode_for(forbidden),
            None,
            "0x{forbidden:02x} must be unassigned"
        );
    }
    // Every instruction is at least one word.
    for descriptor in OPCODE_TABLE {
        assert!(descriptor.instruction_word_count() >= 1);
    }
}

/// Every numeric encoding and semantic opcode occurs in exactly one
/// descriptor, and numeric semantic lookup is only a projection of that row.
#[test]
fn opcode_numeric_and_semantic_descriptors_are_one_to_one() {
    let mut encoded = HashSet::new();
    let mut semantic = HashSet::new();

    for descriptor in OPCODE_TABLE {
        assert!(
            encoded.insert(descriptor.opcode),
            "duplicate numeric opcode 0x{:02x}",
            descriptor.opcode
        );
        assert!(
            semantic.insert(descriptor.kind),
            "duplicate semantic opcode {:?}",
            descriptor.kind
        );
        assert_eq!(opcode_kind(descriptor.opcode), Some(descriptor.kind));
        assert_eq!(
            opcode_for(descriptor.opcode).map(|resolved| resolved.kind),
            Some(descriptor.kind)
        );
        assert_eq!(descriptor_for_opcode(descriptor.kind), descriptor);
    }

    assert_eq!(encoded.len(), 63);
    assert_eq!(semantic.len(), 63);
    assert_eq!(opcode_kind(0xFF), None);
}

/// The canonical table owns one role for every operand position, and the
/// public role APIs resolve all positions without opcode-specific matching.
#[test]
fn opcode_operand_roles_are_complete_unique_and_readable() {
    for descriptor in OPCODE_TABLE {
        assert_eq!(
            descriptor.operand_roles.len(),
            descriptor.operand_layout.len(),
            "{descriptor} must name every operand position"
        );

        let mut unique_roles = HashSet::new();
        let operand_words: Vec<u32> = (0..descriptor.operand_roles.len())
            .map(|position| 0xA000_0000 | position as u32)
            .collect();
        for (position, (&role, &kind)) in descriptor
            .operand_roles
            .iter()
            .zip(descriptor.operand_layout)
            .enumerate()
        {
            assert!(
                unique_roles.insert(role),
                "{descriptor} assigns {role:?} to multiple operands"
            );
            assert_eq!(role.operand_kind(), kind, "{descriptor} role kind mismatch");
            assert_eq!(descriptor.operand_position(role), Some(position));
            assert_eq!(
                descriptor.operand_word(role, &operand_words),
                Some(operand_words[position])
            );
            assert_eq!(
                descriptor.operand_word(role, &operand_words[..position]),
                None
            );
        }

        for effect in descriptor.stack_in.iter().chain(descriptor.stack_out) {
            if let Arity::Declared(role) = effect.arity {
                assert!(
                    descriptor.operand_position(role).is_some(),
                    "{descriptor} stack effect names absent role {role:?}"
                );
                assert_eq!(role.operand_kind(), OperandKind::Immediate);
            }
        }
    }

    let call = descriptor_for_opcode(Opcode::CallInterface);
    assert_eq!(call.operand_position(OperandRole::InterfaceTarget), Some(0));
    assert_eq!(call.operand_position(OperandRole::MethodOrdinal), Some(1));
    assert_eq!(call.operand_position(OperandRole::ArgCount), Some(2));
    assert_eq!(call.operand_position(OperandRole::ResultCount), Some(3));
    assert_eq!(
        call.operand_word(OperandRole::ResultCount, &[7, 11, 3, 1, 9]),
        Some(1)
    );
    assert_eq!(call.operand_position(OperandRole::BranchTarget), None);
}

/// Slot-to-stack transfer and non-tail call results are explicit semantic
/// contracts; no consumer has to infer them from slots or callee metadata.
#[test]
fn transfer_and_call_result_stack_contracts_are_explicit() {
    for kind in [Opcode::LoadSlot, Opcode::TakeSlot] {
        let descriptor = descriptor_for_opcode(kind);
        assert_eq!(descriptor.operand_roles, &[OperandRole::SourceSlot]);
        assert!(descriptor.stack_in.is_empty());
        assert_eq!(descriptor.stack_out.len(), 1);
        assert_eq!(descriptor.stack_out[0].arity, Arity::Fixed(1));
    }

    for kind in [
        Opcode::CallLocal,
        Opcode::CallLocalInOut,
        Opcode::CallService,
        Opcode::CallActor,
        Opcode::CallInterface,
        Opcode::InvokeCallback,
    ] {
        let descriptor = descriptor_for_opcode(kind);
        assert!(descriptor
            .operand_position(OperandRole::ResultCount)
            .is_some());
        assert_eq!(descriptor.stack_out.len(), 1);
        assert_eq!(
            descriptor.stack_out[0].arity,
            Arity::Declared(OperandRole::ResultCount)
        );
    }

    let tail_call = descriptor_for_opcode(Opcode::TailCallLocal);
    assert_eq!(tail_call.operand_position(OperandRole::ResultCount), None);
    assert!(tail_call.stack_out.is_empty());

    let inout = descriptor_for_opcode(Opcode::CallLocalInOut);
    assert_eq!(inout.opcode, 0x26);
    assert_eq!(
        inout.operand_roles,
        &[
            OperandRole::LocalTarget,
            OperandRole::InputCount,
            OperandRole::ResultCount,
            OperandRole::CallLoanLayout,
        ]
    );
    assert_eq!(inout.operand_position(OperandRole::InputCount), Some(1));
    assert_eq!(inout.operand_position(OperandRole::ResultCount), Some(2));
    assert_eq!(inout.operand_position(OperandRole::CallLoanLayout), Some(3));
    assert_eq!(
        inout.stack_in[0].arity,
        Arity::Declared(OperandRole::InputCount)
    );
}

/// Typed scalar operations have fixed stack effects. Eager logical And/Or
/// deliberately have no opcodes; the emitter preserves short-circuiting with
/// control-flow instructions.
#[test]
fn typed_scalar_opcode_stack_contracts_are_fixed() {
    for kind in [Opcode::Not, Opcode::Negate] {
        let descriptor = descriptor_for_opcode(kind);
        assert!(descriptor.operand_layout.is_empty());
        assert_eq!(descriptor.stack_in[0].arity, Arity::Fixed(1));
        assert_eq!(descriptor.stack_out[0].arity, Arity::Fixed(1));
    }
    for kind in [
        Opcode::Add,
        Opcode::Subtract,
        Opcode::Multiply,
        Opcode::Divide,
        Opcode::Equal,
        Opcode::NotEqual,
        Opcode::LessThan,
        Opcode::LessOrEqual,
        Opcode::GreaterThan,
        Opcode::GreaterOrEqual,
    ] {
        let descriptor = descriptor_for_opcode(kind);
        assert!(descriptor.operand_layout.is_empty());
        assert_eq!(descriptor.stack_in[0].arity, Arity::Fixed(2));
        assert_eq!(descriptor.stack_out[0].arity, Arity::Fixed(1));
    }
    assert_eq!(opcode_for(0x9C), None);
    assert_eq!(opcode_for(0x9D), None);
}

/// Operand roles are an immutable part of the canonical opcode projection.
#[test]
fn opcode_table_fingerprint_with_operand_roles_is_frozen() {
    assert_eq!(
        opcode_table_fingerprint(),
        "c54041ca0091b74490b78175a2e2c568c1e5b073116fa4c8b448030f284ca700"
    );
}

/// The opaque validated view owns every linker fact copied across the checked
/// boundary, so consumers never need to consult the raw artifact again.
#[test]
fn validated_view_retains_linker_facts_after_raw_artifact_is_dropped() {
    let mut artifact = canonical_artifact();
    let function = artifact
        .image
        .functions
        .get_mut("module::main")
        .expect("canonical main function");
    function.type_parameters = vec!["T".to_string(), "Result".to_string()];
    function.effect_summary_ref = crate::PackageCallableId::new("effect-summary:module::main");

    let expected_debug_table = artifact
        .image
        .debug_table
        .clone()
        .expect("canonical debug table");
    let view = structurally_validate(&artifact).expect("canonical artifact validates");
    drop(artifact);

    let validated = view
        .functions()
        .iter()
        .find(|function| function.function_key == "module::main")
        .expect("validated main function");
    assert_eq!(
        validated.type_parameters,
        vec!["T".to_string(), "Result".to_string()]
    );
    assert_eq!(
        validated.effect_summary_ref,
        crate::PackageCallableId::new("effect-summary:module::main")
    );
    assert_eq!(view.debug_table(), Some(&expected_debug_table));
    assert_eq!(view.schema_version(), BYTECODE_SCHEMA_VERSION);
    assert_eq!(view.isa_version(), BYTECODE_ISA_VERSION);
    assert_eq!(
        view.bytecode_identity(),
        "opaque-structural-bytecode-identity"
    );
    assert_eq!(view.opcode_table_fingerprint(), opcode_table_fingerprint());
    assert_eq!(
        view.native_value_lifecycle_registry(),
        crate::native_value_lifecycle_registry_identity()
    );
    assert_eq!(
        view.value_lifecycle_policy(),
        crate::value_lifecycle_policy_identity()
    );
    assert_eq!(
        view.host_effect_registry(),
        crate::host_effect_registry_identity()
    );
    assert_eq!(
        view.intrinsic_registry(),
        crate::intrinsic_registry_identity()
    );
    assert_eq!(view.constant_roots()["module.implementation"], 2);
    assert_eq!(view.resume_sites().len(), 1);
    assert_eq!(view.resume_sites()[0].site_pc, 20);
    assert_eq!(validated.relocations, main_function().relocations);
    assert_eq!(
        validated.call_loan_layouts,
        main_function().call_loan_layouts
    );
    assert_eq!(validated.frame_layout.writable_local_slots, vec![1]);
    assert_eq!(
        validated.origin,
        crate::bytecode::dto::BytecodeFunctionOrigin::Executable {
            executable: executable_coordinate(0)
        }
    );
    assert_eq!(validated.self_type_ref, None);

    let mut without_debug = canonical_artifact();
    without_debug.image.debug_table = None;
    let view = structurally_validate(&without_debug).expect("debug table is optional");
    assert_eq!(view.debug_table(), None);
}

/// Every descriptor operand position expectation is covered by the
/// position-class tables (Pool/Table positions never fall through).
#[test]
fn operand_expectation_tables_cover_all_pool_and_table_positions() {
    for descriptor in OPCODE_TABLE {
        for (position, kind) in descriptor.operand_layout.iter().enumerate() {
            match kind {
                OperandKind::Pool => assert!(
                    pool_operand_category(descriptor.opcode, position).is_some(),
                    "opcode 0x{:02x} pool operand at position {position} lacks an expectation",
                    descriptor.opcode
                ),
                OperandKind::Table => assert!(
                    table_operand_category(descriptor.opcode, position).is_some(),
                    "opcode 0x{:02x} table operand at position {position} lacks an expectation",
                    descriptor.opcode
                ),
                _ => {}
            }
        }
    }
}

/// DecodedFunction headers are strictly ascending.
#[test]
fn decoded_headers_are_strictly_ascending() {
    let decoded = BoundedDecoder::new()
        .decode_function(&main_function_words())
        .expect("canonical wordcode decodes");
    let mut previous: Option<u32> = None;
    for pc in decoded.header_pcs {
        if let Some(previous) = previous {
            assert!(previous < pc);
        }
        previous = Some(pc);
    }
}

/// encode → decode → encode is idempotent: decoding the assembled wordcode
/// and re-assembling the decoded instructions reproduces the exact words, and
/// canonical artifact bytes survive a JSON parse → re-assemble cycle
/// byte-for-byte (阶段页 §4.2 验收).
#[test]
fn encode_decode_encode_is_idempotent() {
    // Function level: assemble → decode → re-assemble.
    let decoded = BoundedDecoder::new()
        .decode_function(&main_function_words())
        .expect("canonical wordcode decodes");
    let re_encoded: Vec<EncodedInstruction> = decoded
        .instructions
        .iter()
        .map(|instruction| {
            EncodedInstruction::new(
                instruction.descriptor.opcode,
                instruction.operand_words.clone(),
            )
        })
        .collect();
    assert_eq!(
        assemble_function(&re_encoded).expect("re-assemble must succeed"),
        main_function_words()
    );

    // Artifact level: canonical bytes → JSON → assemble again → same bytes.
    let bytes = assemble_artifact(&canonical_artifact()).expect("canonical bytes");
    let parsed: BytecodeArtifact = serde_json::from_slice(&bytes).expect("parse canonical bytes");
    assert_eq!(
        assemble_artifact(&parsed).expect("re-assemble must succeed"),
        bytes
    );
}

/// A multi-instruction function made of every zero-operand opcode
/// round-trips through assemble → decode → re-assemble with identical words.
#[test]
fn zero_operand_instruction_mix_round_trips() {
    let instructions: Vec<EncodedInstruction> = [
        0x05, 0x08, 0x14, 0x25, 0x51, 0x52, 0x53, 0x56, 0x57, 0x58, 0x5A, 0x5B, 0x5C, 0x90, 0x91,
        0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B,
    ]
    .iter()
    .map(|&opcode| EncodedInstruction::new(opcode, vec![]))
    .collect();
    let words = assemble_function(&instructions).expect("assemble");
    let decoded = BoundedDecoder::new()
        .decode_function(&words)
        .expect("decode must succeed");
    assert_eq!(decoded.instructions.len(), instructions.len());
    let re_encoded: Vec<EncodedInstruction> = decoded
        .instructions
        .iter()
        .map(|instruction| {
            EncodedInstruction::new(
                instruction.descriptor.opcode,
                instruction.operand_words.clone(),
            )
        })
        .collect();
    assert_eq!(
        assemble_function(&re_encoded).expect("re-assemble must succeed"),
        words
    );
}

/// Identity determinism: the canonical bytecode identity preimage
/// (`BytecodeIdentityPayload` → framed sha256, §6.1) is computed by
/// artifact-identity (C9), not by artifact-model; the artifact-identity
/// bytecode module is a separate task and does not exist in this crate, so
/// the "same fixture ⇒ same identity, any field mutation ⇒ different
/// identity" assertion is **skipped** here. The property that identity is a
/// pure function of — canonical bytes — is pinned by the byte-for-byte
/// determinism tests above (`identical_inputs_produce_identical_bytes`,
/// `canonical_bytes_are_insertion_order_insensitive`,
/// `encode_decode_encode_is_idempotent`).
#[test]
fn identity_determinism_is_deferred_to_artifact_identity() {
    // Skipped by design: identity computation lives in artifact-identity.
    // When it lands, add: build twice ⇒ equal identity; mutate any preimage
    // field (schema/ISA version and opcode fingerprint are part of the
    // preimage, §6.1) ⇒ different identity.
}
