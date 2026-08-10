use skiff_artifact_model::{Opcode, ParamModeIr, StatementAttributionId, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, FrameSlotIndex, FunctionIndex, InstructionBoundaryIndex, InstructionIndex,
    LinkedArtifactPoolOrigin, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
    LinkedExactLocalTarget, LinkedFrameLayout, LinkedFunction, LinkedFunctionTables,
    LinkedInstruction, LinkedInstructionTarget, LinkedParameterSlot, LinkedProgramPointState,
    LinkedResolvedOperand, LinkedSlotState, LinkedSourceMapEntry, LinkedStackMapCandidate,
    LinkedStackValue, LinkedStatementEntry, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, SpecializationKey, TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{tail_site, TailMatrixCase};
use crate::tests::fixtures::candidate_parts;

pub(super) fn candidate(
    hydrated: &HydratedDeploymentBytecode,
    case: TailMatrixCase,
) -> LinkedBytecodeCandidate {
    let package = hydrated.packages().values().next().unwrap();
    let build = package.reference().package_build_id.clone();
    let caller_key = super::super::specialization(
        &build,
        super::super::CALLER_FUNCTION,
        super::super::CALLER_CALLABLE,
    );
    let target_key = super::super::specialization(
        &build,
        super::super::TARGET_FUNCTION,
        super::super::TARGET_CALLABLE,
    );
    let functions = vec![
        linked_function(case, true, caller_key.clone()),
        linked_function(case, false, target_key.clone()),
    ];
    let mut parts = candidate_parts(hydrated, None, None);
    parts.types = linked_types(&build);
    parts.functions = functions;
    parts.exact_local_targets = vec![
        LinkedExactLocalTarget::new(caller_key, FunctionIndex::new(0)),
        LinkedExactLocalTarget::new(target_key, FunctionIndex::new(1)),
    ];
    LinkedBytecodeCandidate::try_from_parts(parts).unwrap()
}

fn linked_types(build: &skiff_artifact_model::PackageBuildId) -> Vec<LinkedTypeEntry> {
    [
        TypeRefIr::builtin("string"),
        TypeRefIr::builtin("string"),
        TypeRefIr::builtin("bytes"),
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, ty)| {
        let index = u32::try_from(ordinal).unwrap();
        LinkedTypeEntry::new(
            TypeIndex::new(index),
            LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(index), None)
                .unwrap(),
            ty,
            None,
        )
    })
    .collect()
}

fn linked_function(case: TailMatrixCase, caller: bool, key: SpecializationKey) -> LinkedFunction {
    let (index, instructions, frame, max_depth, effect_owner, states) = if caller {
        (
            FunctionIndex::new(0),
            caller_instructions(case),
            linked_frame(case, true),
            case.caller_max_depth(),
            super::super::CALLER_CALLABLE,
            caller_states(case),
        )
    } else {
        (
            FunctionIndex::new(1),
            target_instructions(case),
            linked_frame(case, false),
            case.target_max_depth(),
            super::super::TARGET_CALLABLE,
            target_states(case),
        )
    };
    let instruction_count = instructions.len();
    let slot_count = frame.slot_types().len();
    let stack_map = LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instruction_count,
        slot_count,
        max_depth,
    )
    .unwrap();
    let statement_entries: Box<[LinkedStatementEntry]> = if caller {
        Box::new([LinkedStatementEntry::new(
            InstructionIndex::new(case.tail_instruction()),
            0,
            StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            tail_site(),
        )])
    } else {
        Box::new([])
    };
    let source_map: Box<[LinkedSourceMapEntry]> = if caller {
        Box::new([LinkedSourceMapEntry::new(
            InstructionIndex::new(case.tail_instruction()),
            InstructionBoundaryIndex::new(case.tail_instruction() + 1),
            tail_site(),
        )])
    } else {
        Box::new([])
    };
    LinkedFunction::new(
        index,
        key,
        instructions.into_boxed_slice(),
        frame,
        max_depth,
        LinkedCallableEffectDeclaration::new(
            super::super::callable(effect_owner),
            super::super::effects::analyzed_no_effects(),
        ),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            statement_entries,
            source_map,
        ),
        stack_map,
    )
}

fn linked_frame(case: TailMatrixCase, caller: bool) -> LinkedFrameLayout {
    let (slots, parameters, writable, results) = if caller {
        (
            case.caller_slots(),
            case.caller_parameters(),
            case.caller_writable(),
            case.caller_results(),
        )
    } else {
        (
            case.target_slots(),
            vec![0],
            Vec::new(),
            case.target_results(),
        )
    };
    let slot_plans = slots.iter().copied().map(exact_plan).collect::<Vec<_>>();
    let parameter_slots = parameters
        .iter()
        .copied()
        .map(|slot| {
            LinkedParameterSlot::new(
                FrameSlotIndex::new(slot),
                ParamModeIr::Value,
                slot_plans[slot as usize].clone(),
            )
        })
        .collect::<Vec<_>>();
    let result_plans = results
        .iter()
        .copied()
        .map(|ty| {
            if !caller && case == TailMatrixCase::ResultPlanCorruption {
                corrupt_plan()
            } else {
                exact_plan(ty)
            }
        })
        .collect::<Vec<_>>();
    LinkedFrameLayout::new(
        slots
            .into_iter()
            .map(TypeIndex::new)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        parameter_slots.into_boxed_slice(),
        writable
            .into_iter()
            .map(FrameSlotIndex::new)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        results
            .into_iter()
            .map(TypeIndex::new)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        slot_plans.into_boxed_slice(),
        result_plans.into_boxed_slice(),
    )
    .unwrap()
}

fn caller_instructions(case: TailMatrixCase) -> Vec<LinkedInstruction> {
    let mut instructions = vec![take_slot(0, 0)];
    if case == TailMatrixCase::StackResidue {
        instructions.push(take_slot(1, 2));
    }
    instructions.push(tail_call(case.tail_pc()));
    instructions
}

fn target_instructions(case: TailMatrixCase) -> Vec<LinkedInstruction> {
    if case == TailMatrixCase::ResultPlanCorruption {
        vec![take_slot(0, 0), return_instruction(2)]
    } else {
        vec![return_instruction(0)]
    }
}

fn take_slot(slot: u32, pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::TakeSlot,
        Box::new([slot]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(slot)),
        )]),
        pc,
    )
    .unwrap()
}

fn tail_call(pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::TailCallLocal,
        Box::new([0, 1]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(1)),
        )]),
        pc,
    )
    .unwrap()
}

fn return_instruction(pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(Opcode::Return, Box::new([]), Box::new([]), pc).unwrap()
}

fn caller_states(case: TailMatrixCase) -> Vec<LinkedProgramPointState> {
    match case {
        TailMatrixCase::LiveCleanup => vec![
            state(0, &[], &[SlotHint::Live(0), SlotHint::Live(0)]),
            state(1, &[(0, false)], &[SlotHint::Moved, SlotHint::Live(0)]),
        ],
        TailMatrixCase::MovedAndUninitialized => vec![
            state(0, &[], &[SlotHint::Live(0), SlotHint::Uninitialized]),
            state(
                1,
                &[(0, false)],
                &[SlotHint::Moved, SlotHint::Uninitialized],
            ),
        ],
        TailMatrixCase::StackResidue => vec![
            state(0, &[], &[SlotHint::Live(0), SlotHint::Live(0)]),
            state(1, &[(0, false)], &[SlotHint::Moved, SlotHint::Live(0)]),
            state(
                2,
                &[(0, false), (0, false)],
                &[SlotHint::Moved, SlotHint::Moved],
            ),
        ],
        TailMatrixCase::ArgumentClassCorruption => vec![
            state(0, &[], &[SlotHint::Live(2)]),
            state(1, &[(2, false)], &[SlotHint::Moved]),
        ],
        TailMatrixCase::ArgumentPlanCorruption => vec![
            state(0, &[], &[SlotHint::Live(0)]),
            state(1, &[(0, true)], &[SlotHint::Moved]),
        ],
        TailMatrixCase::ResultPlanCorruption => vec![
            state(0, &[], &[SlotHint::Live(0)]),
            state(1, &[(0, false)], &[SlotHint::Moved]),
        ],
    }
}

fn target_states(case: TailMatrixCase) -> Vec<LinkedProgramPointState> {
    if case == TailMatrixCase::ResultPlanCorruption {
        vec![
            state(0, &[], &[SlotHint::Live(1)]),
            state(1, &[(1, false)], &[SlotHint::Moved]),
        ]
    } else {
        vec![state(0, &[], &[SlotHint::Live(1)])]
    }
}

#[derive(Clone, Copy)]
enum SlotHint {
    Uninitialized,
    Moved,
    Live(u32),
}

fn state(instruction: u32, stack: &[(u32, bool)], slots: &[SlotHint]) -> LinkedProgramPointState {
    LinkedProgramPointState::new(
        InstructionIndex::new(instruction),
        stack
            .iter()
            .map(|(ty, corrupt)| stack_value(*ty, *corrupt))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        slots
            .iter()
            .map(|slot| match slot {
                SlotHint::Uninitialized => LinkedSlotState::Uninitialized,
                SlotHint::Moved => LinkedSlotState::Moved,
                SlotHint::Live(ty) => LinkedSlotState::Live(stack_value(*ty, false)),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        Box::new([]),
        Box::new([]),
    )
}

fn stack_value(ty: u32, corrupt: bool) -> LinkedStackValue {
    LinkedStackValue::new(
        TypeIndex::new(ty),
        if corrupt {
            corrupt_plan()
        } else {
            exact_plan(ty)
        },
    )
}

fn exact_plan(_ty: u32) -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    }
}

fn corrupt_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::Trivial,
    }
}
