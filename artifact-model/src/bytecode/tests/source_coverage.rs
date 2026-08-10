use crate::bytecode::dto::SourceMapEntry;

use super::*;

fn artifact_with_helper_instruction(
    opcode: Opcode,
    operands: &[u32],
    site: Option<crate::InstructionSourceSite>,
) -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    let descriptor = descriptor_for_opcode(opcode);
    assert_eq!(operands.len(), descriptor.operand_word_count() as usize);
    let mut words = Vec::with_capacity(descriptor.instruction_word_count() as usize + 1);
    words.push(descriptor.opcode.into());
    words.extend_from_slice(operands);
    words.push(descriptor_for_opcode(Opcode::Return).opcode.into());

    let helper = artifact.image.functions.get_mut("module::helper").unwrap();
    helper.words = words;
    helper.source_map = site
        .map(|site| {
            vec![SourceMapEntry {
                start_pc: 0,
                end_pc: descriptor.instruction_word_count(),
                site,
            }]
        })
        .unwrap_or_default();
    artifact
}

#[test]
fn source_coverage_rejects_every_required_opcode_omitted_by_the_legacy_list() {
    let cases: &[(Opcode, &[u32])] = &[
        (Opcode::BudgetCheckpoint, &[]),
        (Opcode::SetWritablePath, &[0, 0, 0]),
        (Opcode::ArrayGet, &[]),
        (Opcode::MapGet, &[]),
        (Opcode::MapEntryAt, &[]),
        (Opcode::Negate, &[]),
        (Opcode::Add, &[]),
        (Opcode::Subtract, &[]),
        (Opcode::Multiply, &[]),
        (Opcode::Divide, &[]),
    ];

    for &(opcode, operands) in cases {
        assert!(matches!(
            contract_for_opcode(opcode).source,
            SourceContract::Required { .. }
        ));
        let artifact = artifact_with_helper_instruction(opcode, operands, None);
        let error = assert_rejected(&artifact);
        assert!(
            error
                .to_string()
                .contains("requires exactly one source/synthetic site (found 0)"),
            "{} must derive source coverage from its contract: {error}",
            opcode.name()
        );
    }
}

#[test]
fn preserve_original_and_active_region_sources_need_no_current_source_row() {
    assert_eq!(
        contract_for_opcode(Opcode::Rethrow).source,
        SourceContract::PreserveOriginal
    );
    let rethrow = artifact_with_helper_instruction(Opcode::Rethrow, &[0], None);
    assert_validates(&rethrow);

    let mut active_region = canonical_artifact();
    let enter = descriptor_for_opcode(Opcode::EnterRegion);
    let leave = descriptor_for_opcode(Opcode::LeaveRegion);
    assert!(matches!(
        contract_for_opcode(enter.kind).source,
        SourceContract::ActiveRegion { .. }
    ));
    assert!(matches!(
        contract_for_opcode(leave.kind).source,
        SourceContract::ActiveRegion { .. }
    ));
    let helper = active_region
        .image
        .functions
        .get_mut("module::helper")
        .unwrap();
    helper.words = vec![
        enter.opcode.into(),
        0,
        leave.opcode.into(),
        0,
        descriptor_for_opcode(Opcode::Return).opcode.into(),
    ];
    helper.active_regions = vec![ActiveRegion {
        start_pc: 0,
        end_pc: 4,
        kind: ActiveRegionKind::Timeout {
            duration_ms: 1,
            site: crate::InstructionSourceSite::Synthetic {
                reason: crate::SyntheticInstructionSiteReason::RuntimeControlFlow,
            },
        },
    }];
    helper.source_map.clear();
    assert_validates(&active_region);
}

#[test]
fn synthetic_only_source_contract_rejects_source_and_accepts_synthetic() {
    assert!(matches!(
        contract_for_opcode(Opcode::BudgetCheckpoint).source,
        SourceContract::Required {
            origin: SourceOriginConstraint::SyntheticOnly,
            ..
        }
    ));
    let source = artifact_with_helper_instruction(
        Opcode::BudgetCheckpoint,
        &[],
        Some(crate::InstructionSourceSite::Source {
            span: crate::SourceSpanRef {
                source_id: 0,
                start: crate::SourcePosition::new(1, 1),
                end: crate::SourcePosition::new(1, 2),
            },
        }),
    );
    assert!(assert_rejected(&source)
        .to_string()
        .contains("requires a synthetic instruction source site"));

    let synthetic = artifact_with_helper_instruction(
        Opcode::BudgetCheckpoint,
        &[],
        Some(crate::InstructionSourceSite::Synthetic {
            reason: crate::SyntheticInstructionSiteReason::RuntimeControlFlow,
        }),
    );
    assert_validates(&synthetic);
}

#[test]
fn source_or_synthetic_contract_accepts_both_site_kinds() {
    assert!(matches!(
        contract_for_opcode(Opcode::ArrayGet).source,
        SourceContract::Required {
            origin: SourceOriginConstraint::SourceOrSynthetic,
            ..
        }
    ));
    for site in [
        crate::InstructionSourceSite::Source {
            span: crate::SourceSpanRef {
                source_id: 0,
                start: crate::SourcePosition::new(1, 1),
                end: crate::SourcePosition::new(1, 2),
            },
        },
        crate::InstructionSourceSite::Synthetic {
            reason: crate::SyntheticInstructionSiteReason::CompilerDesugaring,
        },
    ] {
        let artifact = artifact_with_helper_instruction(Opcode::ArrayGet, &[], Some(site));
        assert_validates(&artifact);
    }
}
