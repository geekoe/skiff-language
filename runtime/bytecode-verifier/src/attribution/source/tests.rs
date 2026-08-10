use skiff_artifact_model::{
    CallableEffectSummary, InstructionSourceSite, Opcode, PackageBuildId, PackageCallableId,
    SourceContract, SourcePosition, SourceSpanRef, SyntheticInstructionSiteReason,
    OPCODE_CONTRACTS,
};
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, ArtifactFunctionKey, FrameSlotIndex, FunctionIndex,
    InstructionBoundaryIndex, InstructionIndex, LinkedActiveRegion, LinkedActiveRegionKind,
    LinkedCallableEffectDeclaration, LinkedFrameLayout, LinkedFunction, LinkedFunctionTables,
    LinkedInstruction, LinkedInstructionTarget, LinkedProgramPointState, LinkedResolvedOperand,
    LinkedSlotState, LinkedSourceMapEntry, LinkedStackMapCandidate, LinkedValueDropPlan,
    LinkedValueTransferPlan, SpecializationKey, TypeIndex,
};

use super::*;

#[test]
fn every_required_contract_rejects_missing_current_coverage() {
    let mut required = 0_usize;
    for contract in OPCODE_CONTRACTS {
        let SourceContract::Required { origin, .. } = contract.source else {
            continue;
        };
        required += 1;
        let error = prove_required_source(contract, origin, None, VerificationLocation::Image)
            .expect_err("every required source contract must reject missing coverage");
        assert!(matches!(
            error,
            VerificationError::SemanticViolation {
                obligation: VerificationObligation::SourceAndStatementAttribution,
                location: VerificationLocation::Image,
                ..
            }
        ));
    }
    assert!(
        required > 0,
        "the canonical opcode table must exercise Required"
    );
}

#[test]
fn source_or_synthetic_accepts_both_current_site_variants() {
    for site in [source_site(), synthetic_site()] {
        let function = function(
            vec![plain(Opcode::ArrayGet, 0)],
            0,
            Vec::new(),
            vec![source_range(0, 1, site)],
        );
        prove_function(&function).expect("ArrayGet accepts source or synthetic attribution");
    }
}

#[test]
fn synthetic_only_rejects_source_and_accepts_synthetic() {
    for opcode in [Opcode::BudgetCheckpoint, Opcode::MapEntryAt] {
        let source = function(
            vec![plain(opcode, 0)],
            0,
            Vec::new(),
            vec![source_range(0, 1, source_site())],
        );
        assert_instruction_violation(
            prove_function(&source).unwrap_err(),
            0,
            "synthetic current source site",
        );

        let synthetic = function(
            vec![plain(opcode, 0)],
            0,
            Vec::new(),
            vec![source_range(0, 1, synthetic_site())],
        );
        prove_function(&synthetic).expect("generated failures accept synthetic attribution");
    }
}

#[test]
fn half_open_ranges_cover_exact_dense_pcs() {
    let missing_second = function(
        vec![plain(Opcode::ArrayGet, 0), plain(Opcode::ArrayGet, 1)],
        0,
        Vec::new(),
        vec![source_range(0, 1, source_site())],
    );
    assert_instruction_violation(
        prove_function(&missing_second).unwrap_err(),
        1,
        "exactly one current",
    );

    let shared_range = function(
        vec![plain(Opcode::ArrayGet, 0), plain(Opcode::ArrayGet, 1)],
        0,
        Vec::new(),
        vec![source_range(0, 2, source_site())],
    );
    prove_function(&shared_range).expect("one half-open row may cover adjacent required PCs");
}

#[test]
fn malformed_linked_ranges_fail_before_opcode_coverage() {
    let cases = [
        (
            vec![plain(Opcode::Return, 0)],
            vec![source_range(0, 0, source_site())],
            "empty or reversed",
        ),
        (
            vec![plain(Opcode::Return, 0)],
            vec![source_range(0, 2, source_site())],
            "outside the dense instruction slice",
        ),
        (
            vec![
                plain(Opcode::Return, 0),
                plain(Opcode::Return, 1),
                plain(Opcode::Return, 2),
            ],
            vec![
                source_range(0, 2, source_site()),
                source_range(1, 3, synthetic_site()),
            ],
            "overlaps or precedes",
        ),
        (
            vec![
                plain(Opcode::Return, 0),
                plain(Opcode::Return, 1),
                plain(Opcode::Return, 2),
            ],
            vec![
                source_range(2, 3, source_site()),
                source_range(0, 1, synthetic_site()),
            ],
            "overlaps or precedes",
        ),
    ];

    for (instructions, ranges, detail) in cases {
        let function = function(instructions, 0, Vec::new(), ranges);
        assert_function_violation(prove_function(&function).unwrap_err(), detail);
    }
}

#[test]
fn source_none_allows_an_incidental_current_row() {
    let function = function(
        vec![plain(Opcode::Return, 0)],
        0,
        Vec::new(),
        vec![source_range(0, 1, source_site())],
    );
    prove_function(&function).expect("SourceContract::None does not forbid a source range");
}

#[test]
fn preserve_original_uses_the_exception_slot_not_the_current_row() {
    for ranges in [Vec::new(), vec![source_range(0, 1, synthetic_site())]] {
        let function = function(vec![rethrow(0)], 1, Vec::new(), ranges);
        prove_function(&function).expect("Rethrow preserves its exception-envelope source slot");
    }

    let out_of_bounds = function(vec![rethrow(1)], 1, Vec::new(), Vec::new());
    assert_instruction_violation(
        prove_function(&out_of_bounds).unwrap_err(),
        0,
        "outside the linked frame",
    );
}

#[test]
fn active_region_contract_uses_the_typed_region_site() {
    for opcode in [Opcode::EnterRegion, Opcode::LeaveRegion] {
        for site in [source_site(), synthetic_site()] {
            let region = active_region(0, site);
            let function = function(
                vec![region_instruction(opcode, 0)],
                0,
                vec![region],
                Vec::new(),
            );
            prove_function(&function)
                .expect("active-region attribution does not require a current source row");
        }
    }
}

#[test]
fn active_region_target_must_be_dense_and_in_bounds() {
    let out_of_bounds = function(
        vec![region_instruction(Opcode::EnterRegion, 1)],
        0,
        vec![active_region(0, source_site())],
        Vec::new(),
    );
    assert_instruction_violation(
        prove_function(&out_of_bounds).unwrap_err(),
        0,
        "out of bounds",
    );

    let mismatched = function(
        vec![region_instruction(Opcode::EnterRegion, 0)],
        0,
        vec![active_region(1, source_site())],
        Vec::new(),
    );
    assert_instruction_violation(
        prove_function(&mismatched).unwrap_err(),
        0,
        "dense table position",
    );
}

fn function(
    instructions: Vec<LinkedInstruction>,
    slot_count: usize,
    active_regions: Vec<LinkedActiveRegion>,
    source_map: Vec<LinkedSourceMapEntry>,
) -> LinkedFunction {
    let slot_types = (0..slot_count)
        .map(|_| TypeIndex::new(0))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let slot_plans = (0..slot_count)
        .map(|_| snapshot_plan())
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let frame = LinkedFrameLayout::new(
        slot_types,
        Box::new([]),
        Box::new([]),
        Box::new([]),
        slot_plans,
        Box::new([]),
    )
    .unwrap();
    let states = (0..instructions.len())
        .map(|ordinal| {
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(ordinal).unwrap()),
                Box::new([]),
                (0..slot_count)
                    .map(|_| LinkedSlotState::Uninitialized)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let stack_map = LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instructions.len(),
        slot_count,
        0,
    )
    .unwrap();

    LinkedFunction::new(
        FunctionIndex::new(0),
        SpecializationKey::new(
            PackageBuildId::new("package-build:source-attribution-test"),
            ArtifactFunctionKey::parse("module::source-attribution").unwrap(),
            PackageCallableId::new("source-attribution"),
            Box::new([]),
            None,
        ),
        instructions.into_boxed_slice(),
        frame,
        0,
        LinkedCallableEffectDeclaration::new(
            PackageCallableId::new("source-attribution"),
            CallableEffectSummary::analysis_pending(),
        ),
        LinkedFunctionTables::new(
            Box::new([]),
            active_regions.into_boxed_slice(),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            source_map.into_boxed_slice(),
        ),
        stack_map,
    )
}

fn plain(opcode: Opcode, artifact_pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(opcode, Box::new([]), Box::new([]), artifact_pc).unwrap()
}

fn rethrow(slot: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::Rethrow,
        Box::new([slot]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(slot)),
        )]),
        0,
    )
    .unwrap()
}

fn region_instruction(opcode: Opcode, region: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([region]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::ActiveRegion(ActiveRegionIndex::new(region)),
        )]),
        0,
    )
    .unwrap()
}

fn active_region(index: u32, site: InstructionSourceSite) -> LinkedActiveRegion {
    LinkedActiveRegion::new(
        ActiveRegionIndex::new(index),
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        LinkedActiveRegionKind::Timeout {
            duration_ms: 1,
            site,
        },
    )
}

fn source_range(start: u32, end: u32, site: InstructionSourceSite) -> LinkedSourceMapEntry {
    LinkedSourceMapEntry::new(
        InstructionIndex::new(start),
        InstructionBoundaryIndex::new(end),
        site,
    )
}

fn source_site() -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 0,
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

fn snapshot_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::Trivial,
    }
}

fn assert_instruction_violation(error: VerificationError, instruction: u32, detail: &str) {
    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail: actual_detail,
    } = error
    else {
        panic!("expected source semantic violation, found {error:?}");
    };
    assert_eq!(
        obligation,
        VerificationObligation::SourceAndStatementAttribution
    );
    assert_eq!(
        location,
        VerificationLocation::Instruction {
            function: FunctionIndex::new(0),
            instruction: InstructionIndex::new(instruction),
        }
    );
    assert!(
        actual_detail.contains(detail),
        "expected detail containing {detail:?}, found {actual_detail:?}"
    );
}

fn assert_function_violation(error: VerificationError, detail: &str) {
    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail: actual_detail,
    } = error
    else {
        panic!("expected source semantic violation, found {error:?}");
    };
    assert_eq!(
        obligation,
        VerificationObligation::SourceAndStatementAttribution
    );
    assert_eq!(
        location,
        VerificationLocation::Function {
            function: FunctionIndex::new(0),
        }
    );
    assert!(
        actual_detail.contains(detail),
        "expected detail containing {detail:?}, found {actual_detail:?}"
    );
}
