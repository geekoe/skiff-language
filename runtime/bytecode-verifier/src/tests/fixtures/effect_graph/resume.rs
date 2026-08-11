use skiff_artifact_model::{BytecodePoolEntry, BytecodePools, ResumeDescriptor, ResumeErrorMode};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedInstruction, LinkedInstructionTarget,
    LinkedProgramPointState, LinkedResolvedOperand, LinkedResumeSite, LinkedSlotState,
    LinkedStackValue, ResumeSiteIndex, TypeIndex,
};

use super::{identities::function_key, EffectGraphCallKind, EffectGraphFunction};

pub(super) const fn max_operand_depth(kind: EffectGraphCallKind) -> u32 {
    match kind {
        EffectGraphCallKind::Resume | EffectGraphCallKind::StreamProducer | EffectGraphCallKind::StreamRead => 1,
        EffectGraphCallKind::StreamReadTwice => 2,
        EffectGraphCallKind::Ordinary | EffectGraphCallKind::Tail | EffectGraphCallKind::InOut => 0,
    }
}

pub(super) fn descriptor_index(functions: &[EffectGraphFunction], ordinal: usize) -> u32 {
    functions[..ordinal]
        .iter()
        .map(|function| function.call_kind.resume_site_count())
        .try_fold(0_u32, u32::checked_add)
        .unwrap()
}

pub(super) fn descriptor_count(functions: &[EffectGraphFunction]) -> u32 {
    functions
        .iter()
        .map(|function| function.call_kind.resume_site_count())
        .try_fold(0_u32, u32::checked_add)
        .unwrap()
}

pub(super) const fn selected_descriptor(descriptor: u32, count: u32, swap: bool) -> u32 {
    if swap {
        count - descriptor - 1
    } else {
        descriptor
    }
}

pub(super) fn artifact_pools(functions: &[EffectGraphFunction]) -> BytecodePools {
    let mut resume = Vec::new();
    for (ordinal, function) in functions.iter().enumerate() {
        match function.call_kind {
            EffectGraphCallKind::Resume => {
                resume.push(artifact_descriptor(ordinal, 0, 2, None, 0, false));
            }
            EffectGraphCallKind::StreamProducer => {
                resume.push(artifact_descriptor(ordinal, 2, 4, None, 0, false));
            }
            EffectGraphCallKind::StreamRead => {
                resume.push(artifact_descriptor(ordinal, 0, 3, Some(4), 0, true));
            }
            EffectGraphCallKind::StreamReadTwice => {
                resume.push(artifact_descriptor(ordinal, 0, 3, Some(10), 0, true));
                resume.push(artifact_descriptor(ordinal, 3, 6, Some(9), 1, true));
            }
            EffectGraphCallKind::Ordinary
            | EffectGraphCallKind::Tail
            | EffectGraphCallKind::InOut => {}
        }
    }
    BytecodePools {
        resume,
        ..BytecodePools::default()
    }
}

fn artifact_descriptor(
    function: usize,
    site_pc: u32,
    resume_pc: u32,
    end_resume_pc: Option<u32>,
    expected_stack_height_before_result: u32,
    stream_read: bool,
) -> BytecodePoolEntry {
    BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
        function_key: function_key(function),
        site_pc,
        resume_pc,
        end_resume_pc,
        expected_stack_height_before_result,
        result_type_refs: if stream_read { vec![0] } else { Vec::new() },
        result_plans: if stream_read {
            vec![super::inout::artifact_item_plan()]
        } else {
            Vec::new()
        },
        error_mode: ResumeErrorMode::RaiseAtSite,
    })
}

pub(super) fn linked_sites(functions: &[EffectGraphFunction]) -> Vec<LinkedResumeSite> {
    let mut rows = Vec::new();
    for (function, spec) in functions.iter().enumerate() {
        match spec.call_kind {
            EffectGraphCallKind::Resume => {
                push_linked_site(&mut rows, function, 0, 1, None, 0, false)
            }
            EffectGraphCallKind::StreamProducer => {
                push_linked_site(&mut rows, function, 1, 2, None, 0, false)
            }
            EffectGraphCallKind::StreamRead => {
                push_linked_site(&mut rows, function, 0, 1, Some(2), 0, true)
            }
            EffectGraphCallKind::StreamReadTwice => {
                push_linked_site(&mut rows, function, 0, 1, Some(6), 0, true);
                push_linked_site(&mut rows, function, 1, 2, Some(5), 1, true);
            }
            EffectGraphCallKind::Ordinary
            | EffectGraphCallKind::Tail
            | EffectGraphCallKind::InOut => {}
        }
    }
    rows
}

fn push_linked_site(
    rows: &mut Vec<LinkedResumeSite>,
    function: usize,
    site: u32,
    resume: u32,
    end_resume: Option<u32>,
    expected_stack_height_before_result: u32,
    stream_read: bool,
) {
    rows.push(
        LinkedResumeSite::new(
            ResumeSiteIndex::new(rows.len() as u32),
            FunctionIndex::new(function as u32),
            InstructionIndex::new(site),
            InstructionIndex::new(resume),
            end_resume.map(InstructionIndex::new),
            expected_stack_height_before_result,
            if stream_read {
                Box::new([TypeIndex::new(0)])
            } else {
                Box::new([])
            },
            if stream_read {
                Box::new([super::inout::linked_item_plan()])
            } else {
                Box::new([])
            },
            ResumeErrorMode::RaiseAtSite,
        )
        .unwrap(),
    );
}

pub(super) fn linked_emit_stream(descriptor: u32, target: u32, artifact_pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::EmitStream,
        Box::new([descriptor]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(target)),
        )]),
        artifact_pc,
    )
    .unwrap()
}

pub(super) fn linked_stream_next(
    descriptor: u32,
    target: u32,
    artifact_pc: u32,
) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::StreamNext,
        Box::new([0, descriptor]),
        Box::new([
            LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::FrameSlot(
                    skiff_runtime_linked_bytecode::FrameSlotIndex::new(0),
                ),
            ),
            LinkedResolvedOperand::new(
                1,
                LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(target)),
            ),
        ]),
        artifact_pc,
    )
    .unwrap()
}

pub(super) fn linked_states(
    kind: EffectGraphCallKind,
    instruction_count: usize,
) -> Vec<LinkedProgramPointState> {
    if matches!(kind, EffectGraphCallKind::StreamProducer) {
        return vec![
            program_point(0, Box::new([]), kind),
            program_point(
                1,
                Box::new([LinkedStackValue::new(
                    TypeIndex::new(0),
                    super::inout::linked_item_plan(),
                )]),
                kind,
            ),
            program_point(2, Box::new([]), kind),
        ];
    }
    if matches!(kind, EffectGraphCallKind::StreamRead) {
        return vec![
            program_point(0, Box::new([]), kind),
            program_point_with_slots(
                1,
                Box::new([LinkedStackValue::new(
                    TypeIndex::new(0),
                    super::inout::linked_item_plan(),
                )]),
                moved_slot_state(kind),
            ),
            program_point_with_slots(2, Box::new([]), moved_slot_state(kind)),
        ];
    }
    if matches!(kind, EffectGraphCallKind::StreamReadTwice) {
        return vec![
            program_point(0, Box::new([]), kind),
            program_point_with_slots(
                1,
                Box::new([LinkedStackValue::new(
                    TypeIndex::new(0),
                    super::inout::linked_item_plan(),
                )]),
                moved_slot_state(kind),
            ),
            program_point_with_slots(
                2,
                Box::new([
                    LinkedStackValue::new(TypeIndex::new(0), super::inout::linked_item_plan()),
                    LinkedStackValue::new(TypeIndex::new(0), super::inout::linked_item_plan()),
                ]),
                moved_slot_state(kind),
            ),
            program_point_with_slots(
                3,
                Box::new([LinkedStackValue::new(
                    TypeIndex::new(0),
                    super::inout::linked_item_plan(),
                )]),
                moved_slot_state(kind),
            ),
            program_point_with_slots(4, Box::new([]), moved_slot_state(kind)),
            program_point_with_slots(
                5,
                Box::new([LinkedStackValue::new(
                    TypeIndex::new(0),
                    super::inout::linked_item_plan(),
                )]),
                moved_slot_state(kind),
            ),
            program_point_with_slots(6, Box::new([]), moved_slot_state(kind)),
        ];
    }
    (0..instruction_count)
        .map(|instruction| program_point(instruction as u32, Box::new([]), kind))
        .collect()
}

fn moved_slot_state(kind: EffectGraphCallKind) -> Box<[LinkedSlotState]> {
    (0..super::inout::slot_count(kind))
        .map(|_| LinkedSlotState::Moved)
        .collect()
}


fn program_point(
    instruction: u32,
    stack: Box<[LinkedStackValue]>,
    kind: EffectGraphCallKind,
) -> LinkedProgramPointState {
    LinkedProgramPointState::new(
        InstructionIndex::new(instruction),
        stack,
        super::inout::linked_slot_states(kind),
        Box::new([]),
        Box::new([]),
    )
}

fn program_point_with_slots(
    instruction: u32,
    stack: Box<[LinkedStackValue]>,
    slots: Box<[LinkedSlotState]>,
) -> LinkedProgramPointState {
    LinkedProgramPointState::new(
        InstructionIndex::new(instruction),
        stack,
        slots,
        Box::new([]),
        Box::new([]),
    )
}
