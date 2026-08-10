use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedInstruction, LinkedInstructionTarget, LinkedResolvedOperand,
};

pub(super) fn linked_call(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::CallLocal,
        Box::new([0, 0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

pub(super) fn linked_tail_call(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::TailCallLocal,
        Box::new([0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

pub(super) fn linked_budget() -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::BudgetCheckpoint,
        Box::new([]),
        Box::new([]),
        4,
    )
    .unwrap()
}

pub(super) fn linked_return(pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::Return,
        Box::new([]),
        Box::new([]),
        pc,
    )
    .unwrap()
}

pub(super) fn linked_pop(pc: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        skiff_artifact_model::Opcode::Pop,
        Box::new([]),
        Box::new([]),
        pc,
    )
    .unwrap()
}
