use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Private segmented frame descriptor. Values live in the fiber's contiguous
/// storage rather than in this metadata object.
#[derive(Debug)]
pub(crate) struct VmFrame {
    function: FunctionIndex,
    instruction: InstructionIndex,
    slot_base: usize,
    slot_count: usize,
    operand_base: usize,
    operand_capacity: usize,
    operand_depth: usize,
    function_entry_pending: bool,
}

impl VmFrame {
    pub(crate) const fn root(
        function: FunctionIndex,
        slot_base: usize,
        slot_count: usize,
        operand_base: usize,
        operand_capacity: usize,
    ) -> Self {
        Self {
            function,
            instruction: InstructionIndex::new(0),
            slot_base,
            slot_count,
            operand_base,
            operand_capacity,
            operand_depth: 0,
            function_entry_pending: true,
        }
    }

    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub(crate) const fn slot_base(&self) -> usize {
        self.slot_base
    }

    pub(crate) const fn slot_count(&self) -> usize {
        self.slot_count
    }

    pub(crate) const fn operand_base(&self) -> usize {
        self.operand_base
    }

    pub(crate) const fn operand_capacity(&self) -> usize {
        self.operand_capacity
    }

    pub(crate) const fn operand_depth(&self) -> usize {
        self.operand_depth
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

    pub(crate) fn set_operand_depth(&mut self, depth: usize) {
        self.operand_depth = depth;
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
