use super::*;

fn main_entries(artifact: &mut BytecodeArtifact) -> &mut Vec<StatementEntry> {
    &mut artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .statement_entries
}

#[test]
fn same_pc_events_are_ordered_by_dense_sequence_not_site() {
    let artifact = canonical_artifact();
    let entries = &artifact.image.functions["module::main"].statement_entries;
    assert_eq!((entries[0].pc, entries[0].sequence_ordinal), (6, 0));
    assert_eq!((entries[1].pc, entries[1].sequence_ordinal), (6, 1));
    assert_ne!(entries[0].site, entries[1].site);
    assert_validates(&artifact);
}

#[test]
fn statement_rows_reject_sequence_gaps_reused_ids_and_operand_pcs() {
    let mut sequence_gap = canonical_artifact();
    main_entries(&mut sequence_gap)[1].sequence_ordinal = 2;
    assert!(assert_rejected(&sequence_gap)
        .to_string()
        .contains("sequenceOrdinal"));

    let mut reused_id = canonical_artifact();
    let entries = main_entries(&mut reused_id);
    entries[4].attribution_id = entries[0].attribution_id;
    assert!(assert_rejected(&reused_id)
        .to_string()
        .contains("repeats attribution id"));

    let mut operand_pc = canonical_artifact();
    main_entries(&mut operand_pc)[4].pc = 26;
    assert!(assert_rejected(&operand_pc)
        .to_string()
        .contains("not an instruction header"));
}

#[test]
fn generated_rows_reject_source_sites_and_ordinal_gaps() {
    let mut source_site = canonical_artifact();
    let replacement = main_entries(&mut source_site)[0].site.clone();
    main_entries(&mut source_site)[2].site = replacement;
    assert!(assert_rejected(&source_site)
        .to_string()
        .contains("Generated attribution id with a source site"));

    let mut ordinal_gap = canonical_artifact();
    main_entries(&mut ordinal_gap)[3].attribution_id =
        StatementAttributionId::Generated { ordinal: 2 };
    assert!(assert_rejected(&ordinal_gap)
        .to_string()
        .contains("generated attribution ordinals must be dense"));
}

#[test]
fn opcode_statement_contract_requires_exactly_one_event_of_its_class() {
    let mut missing_expression = canonical_artifact();
    main_entries(&mut missing_expression)[1].attribution_id = StatementAttributionId::Statement {
        statement_index: 2,
        occurrence_ordinal: 0,
    };
    let error = assert_rejected(&missing_expression);
    assert!(error.to_string().contains("call_local at pc 6"), "{error}");
    assert!(error.to_string().contains("found 0"), "{error}");

    let mut duplicate_generated = canonical_artifact();
    let duplicate = StatementEntry {
        pc: 10,
        sequence_ordinal: 1,
        attribution_id: StatementAttributionId::Generated { ordinal: 2 },
        site: crate::InstructionSourceSite::Synthetic {
            reason: crate::SyntheticInstructionSiteReason::CompilerDesugaring,
        },
    };
    main_entries(&mut duplicate_generated).insert(3, duplicate);
    let error = assert_rejected(&duplicate_generated);
    assert!(
        error.to_string().contains("budget_checkpoint at pc 10"),
        "{error}"
    );
    assert!(error.to_string().contains("found 2"), "{error}");
}

#[test]
fn statement_entry_wire_rejects_legacy_fixed_charge_fields() {
    let entry = canonical_artifact().image.functions["module::main"].statement_entries[0].clone();
    let value = serde_json::to_value(entry).unwrap();

    for field in ["sequenceOrdinal", "attributionId", "site"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<StatementEntry>(missing).is_err());
    }
    for field in ["statementId", "chargeKind"] {
        let mut legacy = value.clone();
        legacy[field] = serde_json::json!("legacy");
        assert!(serde_json::from_value::<StatementEntry>(legacy).is_err());
    }
}
