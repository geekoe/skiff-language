use skiff_artifact_model::{
    contract_for_opcode, InstructionSourceSite, Opcode, SourcePosition, SourceSpanRef,
    StatementAttributionClass, StatementAttributionId, StatementChargeKind, StatementEntry,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

use super::{
    canonical::prove_canonical_rows, prove_all_instructions_reachable, required_reclassification,
};

#[test]
fn schedule_proof_rejects_noncanonical_same_pc_sequence() {
    let rows = [StatementEntry {
        pc: 0,
        sequence_ordinal: 1,
        attribution_id: StatementAttributionId::Statement {
            statement_index: 0,
            occurrence_ordinal: 0,
        },
        site: synthetic_site(),
    }];
    let error = prove_canonical_rows(&rows, FunctionIndex::new(0), 1, &limits())
        .expect_err("P2 must not rely on candidate-local canonical validation");

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::SourceAndStatementAttribution,
            detail,
            ..
        } if detail.contains("sequenceOrdinal 1")
    ));
}

#[test]
fn linear_canonical_checker_matches_foundation_positive_and_negative_matrix() {
    let cases = vec![
        ("empty", Vec::new(), true),
        (
            "same-pc-sequence-and-reset",
            vec![
                statement(0, 0, 0, 0),
                expression(0, 1, 0, 0),
                generated(1, 0, 0),
            ],
            true,
        ),
        (
            "statement-may-be-synthetic",
            vec![StatementEntry {
                pc: 0,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Statement {
                    statement_index: 0,
                    occurrence_ordinal: 0,
                },
                site: synthetic_site(),
            }],
            true,
        ),
        (
            "sequence-must-start-zero",
            vec![statement(0, 1, 0, 0)],
            false,
        ),
        (
            "instruction-order",
            vec![statement(1, 0, 0, 0), statement(0, 0, 1, 0)],
            false,
        ),
        (
            "same-pc-sequence-gap",
            vec![statement(0, 0, 0, 0), statement(0, 2, 1, 0)],
            false,
        ),
        (
            "new-pc-sequence-reset",
            vec![statement(0, 0, 0, 0), statement(1, 1, 1, 0)],
            false,
        ),
        (
            "duplicate-typed-id",
            vec![statement(0, 0, 7, 0), statement(0, 1, 7, 0)],
            false,
        ),
        (
            "statement-occurrence-gap",
            vec![statement(0, 0, 7, 0), statement(0, 1, 7, 2)],
            false,
        ),
        (
            "expression-occurrence-gap",
            vec![expression(0, 0, 9, 0), expression(0, 1, 9, 2)],
            false,
        ),
        (
            "generated-ordinal-gap",
            vec![generated(0, 0, 0), generated(0, 1, 2)],
            false,
        ),
        (
            "generated-requires-synthetic-site",
            vec![StatementEntry {
                pc: 0,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Generated { ordinal: 0 },
                site: source_site(99),
            }],
            false,
        ),
    ];

    for (name, rows, expected) in cases {
        let foundation = skiff_artifact_model::validate_statement_entries_canonical(&rows).is_ok();
        let verifier = prove_canonical_rows(&rows, FunctionIndex::new(0), 2, &limits()).is_ok();
        assert_eq!(foundation, expected, "foundation matrix case {name}");
        assert_eq!(verifier, expected, "verifier matrix case {name}");
    }
}

#[test]
fn linear_canonical_checker_enforces_instruction_and_per_pc_bounds_itself() {
    let outside = [statement(2, 0, 0, 0)];
    let error = prove_canonical_rows(&outside, FunctionIndex::new(0), 2, &limits())
        .expect_err("P2 must recheck the dense instruction bound");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::SourceAndStatementAttribution,
            detail,
            ..
        } if detail.contains("outside the instruction slice")
    ));

    let same_pc = [statement(0, 0, 0, 0), expression(0, 1, 0, 0)];
    let mut bounded = limits();
    bounded.max_statement_events_per_pc = 1;
    assert!(matches!(
        prove_canonical_rows(&same_pc, FunctionIndex::new(0), 1, &bounded),
        Err(VerificationError::LimitExceeded {
            limit: VerificationLimit::StatementEventsPerPc,
            actual: 2,
            max: 1,
            ..
        })
    ));
}

#[test]
fn linear_canonical_dense_diagnostic_uses_first_seen_source_index() {
    let rows = [statement(0, 0, 9, 2), statement(0, 1, 1, 3)];
    let error = prove_canonical_rows(&rows, FunctionIndex::new(0), 1, &limits())
        .expect_err("both source indices have occurrence gaps");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation { detail, .. }
            if detail.contains("source index 9")
    ));
}

#[test]
fn required_event_is_exact_one_and_returns_one_reclassification() {
    let contract = contract_for_opcode(Opcode::CallLocal);
    let location = VerificationLocation::Instruction {
        function: FunctionIndex::new(0),
        instruction: InstructionIndex::new(0),
    };
    for matching in [0, 2] {
        let error =
            required_reclassification(contract.statement, matching, contract.mnemonic, location)
                .expect_err("missing or duplicated required class must fail closed");
        assert!(matches!(
            error,
            VerificationError::SemanticViolation {
                obligation: VerificationObligation::SourceAndStatementAttribution,
                ..
            }
        ));
    }
    assert_eq!(
        required_reclassification(contract.statement, 1, contract.mnemonic, location).unwrap(),
        Some((
            StatementAttributionClass::Expression,
            StatementChargeKind::LocalCall,
        ))
    );
}

#[test]
fn schedule_requires_reachable_flow_fact_for_every_instruction() {
    let error = prove_all_instructions_reachable(FunctionIndex::new(0), 2, |instruction| {
        instruction == InstructionIndex::new(0)
    })
    .expect_err("an unreachable dense instruction must not enter a schedule");

    assert_eq!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::SourceAndStatementAttribution,
            location: VerificationLocation::Instruction {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(1),
            },
            detail: "statement schedule cannot cover an instruction without reachable flow facts"
                .to_string(),
        }
    );
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

fn source_site(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn statement(
    pc: u32,
    sequence_ordinal: u32,
    statement_index: u32,
    occurrence_ordinal: u32,
) -> StatementEntry {
    StatementEntry {
        pc,
        sequence_ordinal,
        attribution_id: StatementAttributionId::Statement {
            statement_index,
            occurrence_ordinal,
        },
        site: source_site(u64::from(statement_index) + 1),
    }
}

fn expression(
    pc: u32,
    sequence_ordinal: u32,
    expression_index: u32,
    occurrence_ordinal: u32,
) -> StatementEntry {
    StatementEntry {
        pc,
        sequence_ordinal,
        attribution_id: StatementAttributionId::Expression {
            expression_index,
            occurrence_ordinal,
        },
        site: source_site(u64::from(expression_index) + 101),
    }
}

fn generated(pc: u32, sequence_ordinal: u32, ordinal: u32) -> StatementEntry {
    StatementEntry {
        pc,
        sequence_ordinal,
        attribution_id: StatementAttributionId::Generated { ordinal },
        site: synthetic_site(),
    }
}

fn limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: u64::MAX,
        max_total_instructions: u64::MAX,
        max_instructions_per_function: u64::MAX,
        max_frame_slots_per_function: u64::MAX,
        max_operand_depth: u64::MAX,
        max_control_flow_edges_per_function: u64::MAX,
        max_exception_regions_per_function: u64::MAX,
        max_switch_targets_per_function: u64::MAX,
        max_statement_events_per_pc: u64::MAX,
        max_statement_events_per_function: u64::MAX,
        max_total_statement_events: u64::MAX,
        max_source_map_entries_per_function: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_arity: u64::MAX,
        max_callback_captures_per_callback: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_value_lifecycle_nodes: u64::MAX,
        max_value_lifecycle_canonical_bytes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}
