use serde_json::json;

use super::*;
use crate::{
    BytecodeFunctionOrigin, InstructionSourceSite, PackageExecutableCoordinate, SourcePosition,
    SourceSpanRef, SyntheticInstructionSiteReason,
};

fn origin(executable_index: u32) -> BytecodeFunctionOrigin {
    BytecodeFunctionOrigin::Executable {
        executable: PackageExecutableCoordinate {
            file_ir_identity: "skiff-file-ir-v13:sha256:fixture".to_string(),
            module_path: "example.main".to_string(),
            executable_index,
        },
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

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

fn statement_entry(pc: u32, sequence_ordinal: u32) -> StatementEntry {
    StatementEntry {
        pc,
        sequence_ordinal,
        attribution_id: StatementAttributionId::Statement {
            statement_index: 7,
            occurrence_ordinal: sequence_ordinal,
        },
        site: source_site(u64::from(sequence_ordinal) + 1),
    }
}

fn expression_entry(pc: u32, sequence_ordinal: u32, occurrence_ordinal: u32) -> StatementEntry {
    StatementEntry {
        pc,
        sequence_ordinal,
        attribution_id: StatementAttributionId::Expression {
            expression_index: 9,
            occurrence_ordinal,
        },
        site: source_site(u64::from(occurrence_ordinal) + 10),
    }
}

#[test]
fn statement_entry_wire_is_typed_and_sequence_bearing() {
    let entry = statement_entry(4, 0);
    let value = serde_json::to_value(&entry).unwrap();
    assert_eq!(
        value,
        json!({
            "pc": 4,
            "sequenceOrdinal": 0,
            "attributionId": {
                "kind": "statement",
                "statementIndex": 7,
                "occurrenceOrdinal": 0
            },
            "site": {
                "kind": "source",
                "span": {
                    "sourceId": 1,
                    "start": { "line": 1, "column": 1 },
                    "end": { "line": 1, "column": 2 }
                }
            }
        })
    );
    assert_eq!(
        serde_json::from_value::<StatementEntry>(value).unwrap(),
        entry
    );
}

#[test]
fn canonical_entries_allow_dense_same_pc_rows() {
    let entries = vec![statement_entry(0, 0), statement_entry(0, 1)];
    validate_statement_entries_canonical(&entries).unwrap();

    let mut gap = entries.clone();
    gap[1].sequence_ordinal = 2;
    assert!(validate_statement_entries_canonical(&gap).is_err());

    let mut duplicate = entries;
    duplicate[1].attribution_id = duplicate[0].attribution_id;
    assert!(validate_statement_entries_canonical(&duplicate).is_err());

    let mut occurrence_gap = vec![statement_entry(0, 0), statement_entry(1, 0)];
    occurrence_gap[1].attribution_id = StatementAttributionId::Statement {
        statement_index: 7,
        occurrence_ordinal: 2,
    };
    assert!(validate_statement_entries_canonical(&occurrence_gap).is_err());
}

#[test]
fn expression_occurrences_are_dense_per_expression_index_with_deterministic_diagnostics() {
    let mut first_source = expression_entry(0, 0, 0);
    first_source.attribution_id = StatementAttributionId::Expression {
        expression_index: 9,
        occurrence_ordinal: 0,
    };
    let mut lower_source = expression_entry(1, 0, 0);
    lower_source.attribution_id = StatementAttributionId::Expression {
        expression_index: 3,
        occurrence_ordinal: 0,
    };
    let mut first_source_gap = expression_entry(2, 0, 2);
    first_source_gap.attribution_id = StatementAttributionId::Expression {
        expression_index: 9,
        occurrence_ordinal: 2,
    };
    let mut lower_source_gap = expression_entry(3, 0, 2);
    lower_source_gap.attribution_id = StatementAttributionId::Expression {
        expression_index: 3,
        occurrence_ordinal: 2,
    };
    let expression_gap = vec![
        first_source,
        lower_source,
        first_source_gap,
        lower_source_gap,
    ];
    let error = validate_statement_entries_canonical(&expression_gap).unwrap_err();
    assert!(error
        .message()
        .contains("expression attribution occurrences"));
    assert!(error.message().contains("source index 3"));
}

#[test]
fn generated_attribution_requires_a_synthetic_site() {
    let mut entry = statement_entry(0, 0);
    entry.attribution_id = StatementAttributionId::Generated { ordinal: 0 };
    assert!(validate_statement_entries_canonical(&[entry.clone()]).is_err());
    entry.site = synthetic_site();
    validate_statement_entries_canonical(&[entry]).unwrap();

    let generated_gap = vec![
        StatementEntry {
            attribution_id: StatementAttributionId::Generated { ordinal: 0 },
            site: synthetic_site(),
            ..statement_entry(0, 0)
        },
        StatementEntry {
            attribution_id: StatementAttributionId::Generated { ordinal: 2 },
            site: synthetic_site(),
            ..statement_entry(1, 0)
        },
    ];
    assert!(validate_statement_entries_canonical(&generated_gap).is_err());
}

#[test]
fn statement_and_expression_attribution_may_use_synthetic_sites() {
    let mut statement = statement_entry(0, 0);
    statement.site = synthetic_site();
    let mut expression = expression_entry(1, 0, 0);
    expression.site = synthetic_site();
    validate_statement_entries_canonical(&[statement, expression]).unwrap();
}

fn single_entry_manifest() -> BytecodeFunctionStatementManifest {
    BytecodeFunctionStatementManifest::new(origin(0), vec![statement_entry(4, 0)])
}

fn manifest_identity(
    manifest: &BytecodeFunctionStatementManifest,
) -> BytecodeStatementManifestIdentity {
    derive_bytecode_statement_manifest_identity("example.pkg", &[manifest.clone()]).unwrap()
}

#[test]
fn empty_manifest_identity_is_frozen() {
    let empty = derive_bytecode_statement_manifest_identity("example.pkg", &[]).unwrap();
    assert_eq!(
        empty.as_str(),
        "skiff-bytecode-statement-manifest-v1:sha256:8c84ffc258141161ff39a76e7ed4130b13977e71dbbd641920d5ededc0c6ef81"
    );
}

#[test]
fn manifest_identity_commits_zero_event_functions() {
    let empty = derive_bytecode_statement_manifest_identity("example.pkg", &[]).unwrap();
    let function = BytecodeFunctionStatementManifest::new(origin(0), Vec::new());
    let with_empty_function =
        derive_bytecode_statement_manifest_identity("example.pkg", &[function]).unwrap();
    assert_ne!(empty, with_empty_function);
}

#[test]
fn manifest_identity_commits_function_origin() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    let mut changed = manifest;
    changed.origin = origin(1);
    assert_ne!(manifest_identity(&changed), baseline);
}

#[test]
fn manifest_identity_commits_pc() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    let mut changed = manifest;
    changed.statement_entries[0].pc = 5;
    assert_ne!(manifest_identity(&changed), baseline);
}

#[test]
fn manifest_identity_commits_sequence_ordinal() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    let mut changed = manifest;
    changed.statement_entries[0].sequence_ordinal = 1;
    assert!(
        derive_bytecode_statement_manifest_identity("example.pkg", &[changed.clone()]).is_err()
    );
    assert_ne!(
        super::identity::derive_manifest_identity_from_projection_for_test(
            "example.pkg",
            &[changed],
        )
        .unwrap(),
        baseline
    );
}

#[test]
fn manifest_identity_commits_attribution_id() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    let mut changed = manifest;
    changed.statement_entries[0].attribution_id = StatementAttributionId::Statement {
        statement_index: 8,
        occurrence_ordinal: 0,
    };
    assert_ne!(manifest_identity(&changed), baseline);
}

#[test]
fn manifest_identity_commits_site() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    let mut changed = manifest;
    changed.statement_entries[0].site = source_site(99);
    assert_ne!(manifest_identity(&changed), baseline);
}

#[test]
fn manifest_identity_commits_package_id_and_validates_exact_identity() {
    let manifest = single_entry_manifest();
    let baseline = manifest_identity(&manifest);
    validate_bytecode_statement_manifest_identity("example.pkg", &[manifest.clone()], &baseline)
        .unwrap();
    assert_ne!(
        derive_bytecode_statement_manifest_identity("other.pkg", &[manifest]).unwrap(),
        baseline
    );
}

#[test]
fn manifest_identity_domain_is_frozen() {
    assert_eq!(
        BYTECODE_STATEMENT_MANIFEST_SCHEMA_MARKER,
        "skiff-bytecode-statement-manifest-v1"
    );
    assert_eq!(
        BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX,
        "skiff-bytecode-statement-manifest-v1:sha256"
    );
}

#[test]
fn manifest_identity_rejects_noncanonical_function_order_and_digest_text() {
    let functions = vec![
        BytecodeFunctionStatementManifest::new(origin(1), Vec::new()),
        BytecodeFunctionStatementManifest::new(origin(0), Vec::new()),
    ];
    assert!(derive_bytecode_statement_manifest_identity("example.pkg", &functions).is_err());
    assert!(BytecodeStatementManifestIdentity::parse(format!(
        "{BYTECODE_STATEMENT_MANIFEST_IDENTITY_PREFIX}:{}",
        "A".repeat(64)
    ))
    .is_err());
}
