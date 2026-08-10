use skiff_artifact_model::{BytecodePoolEntry, BytecodePools, ResumeDescriptor, ResumeErrorMode};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedInstruction, LinkedInstructionTarget,
    LinkedResolvedOperand, LinkedResumeSite, ResumeSiteIndex,
};

use super::{identities::function_key, EffectGraphCallKind, EffectGraphFunction};

pub(super) const fn max_operand_depth(kind: EffectGraphCallKind) -> u32 {
    match kind {
        EffectGraphCallKind::Resume => 1,
        EffectGraphCallKind::Ordinary | EffectGraphCallKind::Tail | EffectGraphCallKind::InOut => 0,
    }
}

pub(super) fn artifact_pools(functions: &[EffectGraphFunction]) -> BytecodePools {
    let resume = functions
        .iter()
        .enumerate()
        .filter(|(_, function)| matches!(function.call_kind, EffectGraphCallKind::Resume))
        .map(|(ordinal, _)| {
            BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
                function_key: function_key(ordinal),
                site_pc: 0,
                resume_pc: 2,
                expected_stack_height_before_result: 0,
                result_type_refs: Vec::new(),
                result_plans: Vec::new(),
                error_mode: ResumeErrorMode::RaiseAtSite,
            })
        })
        .collect();
    BytecodePools {
        resume,
        ..BytecodePools::default()
    }
}

pub(super) fn linked_sites(functions: &[EffectGraphFunction]) -> Vec<LinkedResumeSite> {
    functions
        .iter()
        .enumerate()
        .filter(|(_, function)| matches!(function.call_kind, EffectGraphCallKind::Resume))
        .enumerate()
        .map(|(resume_ordinal, (function_ordinal, _))| {
            LinkedResumeSite::new(
                ResumeSiteIndex::new(resume_ordinal as u32),
                FunctionIndex::new(function_ordinal as u32),
                InstructionIndex::new(0),
                InstructionIndex::new(1),
                0,
                Box::new([]),
                Box::new([]),
                ResumeErrorMode::RaiseAtSite,
            )
            .unwrap()
        })
        .collect()
}

pub(super) fn linked_emit_stream() -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::EmitStream,
        Box::new([0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
        )]),
        0,
    )
    .unwrap()
}
