use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Private segmented frame descriptor. Values live in the fiber's contiguous
/// storage rather than in this metadata object.
#[derive(Debug)]
pub(crate) struct VmFrame {
    function: FunctionIndex,
    instruction: InstructionIndex,
    operand_base: usize,
    operand_capacity: usize,
    function_entry_pending: bool,
}

impl VmFrame {
    pub(crate) const fn root(
        function: FunctionIndex,
        operand_base: usize,
        operand_capacity: usize,
    ) -> Self {
        Self {
            function,
            instruction: InstructionIndex::new(0),
            operand_base,
            operand_capacity,
            function_entry_pending: true,
        }
    }

    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub(crate) const fn function_entry_pending(&self) -> bool {
        self.function_entry_pending
    }

    pub(crate) fn mark_function_entry_charged(&mut self) {
        self.function_entry_pending = false;
    }

    pub(crate) fn advance_instruction(&mut self) -> bool {
        let Some(next) = self.instruction.get().checked_add(1) else {
            return false;
        };
        self.instruction = InstructionIndex::new(next);
        true
    }

    pub(crate) fn segment_end(&self) -> Option<usize> {
        self.operand_base.checked_add(self.operand_capacity)
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::VmFrame;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn frame_metadata_layout_stays_compact() {
        assert!(size_of::<VmFrame>() <= 64);
    }
}
