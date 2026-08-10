mod basics;
mod calls;
mod merge;
mod write_and_hints;

use skiff_artifact_model::{ParamModeIr, TypeRefIr};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedInstruction};

use crate::{
    control_flow::prove_stack_and_slot_state_for_test, VerificationError, VerificationLocation,
    VerificationObligation,
};

use super::fixtures::{
    generous_limits,
    stack_state::{fixture, FunctionSpec, Hint, HintSlot, StackFixture},
};

fn prove(fixture: &StackFixture) -> Result<(), VerificationError> {
    prove_stack_and_slot_state_for_test(&fixture.hydrated, &fixture.candidate, &generous_limits())
}

fn branch_fixture(
    types: Vec<TypeRefIr>,
    slots: Vec<u32>,
    instructions: Vec<LinkedInstruction>,
) -> StackFixture {
    let parameters = (0..slots.len())
        .map(|slot| {
            (
                u32::try_from(slot).expect("test slot fits u32"),
                ParamModeIr::Value,
            )
        })
        .collect();
    fixture(types, spec(slots, parameters, Vec::new(), instructions, 1))
}

fn spec(
    slots: Vec<u32>,
    parameters: Vec<(u32, ParamModeIr)>,
    results: Vec<u32>,
    instructions: Vec<LinkedInstruction>,
    declared_max: u32,
) -> FunctionSpec {
    FunctionSpec {
        slots,
        parameters,
        writable: Vec::new(),
        results,
        instructions,
        declared_max,
        hints: None,
    }
}

trait WithHints {
    fn with_hints(self, hints: Vec<Hint>) -> Self;
}

impl WithHints for FunctionSpec {
    fn with_hints(mut self, hints: Vec<Hint>) -> Self {
        self.hints = Some(hints);
        self
    }
}

fn hint(stack: &[u32], slots: &[HintSlot]) -> Hint {
    Hint {
        stack: stack.to_vec(),
        slots: slots.to_vec(),
    }
}

const fn uninitialized() -> HintSlot {
    HintSlot::Uninitialized
}

const fn moved() -> HintSlot {
    HintSlot::Moved
}

const fn live(ty: u32) -> HintSlot {
    HintSlot::Live(ty)
}

fn assert_instruction_violation(error: VerificationError, instruction: u32) {
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::StackAndSlotState,
            location,
            ..
        } if location == instruction_location(instruction)
    ));
}

const fn instruction_location(instruction: u32) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: FunctionIndex::new(0),
        instruction: InstructionIndex::new(instruction),
    }
}
