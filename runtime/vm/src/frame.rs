use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Private segmented frame descriptor. Values live in the fiber's contiguous
/// storage rather than in this metadata object.
#[derive(Debug, Clone)]
pub(crate) struct VmFrame {
    function: FunctionIndex,
    instruction: InstructionIndex,
    slot_base: usize,
    slot_count: usize,
    operand_capacity: usize,
    operand_height: usize,
    resume_instruction: Option<InstructionIndex>,
    // Cleared once per invocation. Only constructing/replacing a frame can
    // rearm it; ordinary control flow and resume retain the cleared state.
    function_entry_pending: bool,
    // Same-PC source-event progress commits one successful charge at a time.
    // Moving to a new PC resets it; resume deliberately retains it.
    next_statement_event_index: usize,
    statement_events_complete: bool,
}

impl VmFrame {
    pub(crate) const fn root(
        function: FunctionIndex,
        slot_count: usize,
        operand_capacity: usize,
    ) -> Self {
        Self {
            function,
            instruction: InstructionIndex::new(0),
            slot_base: 0,
            slot_count,
            operand_capacity,
            operand_height: 0,
            resume_instruction: None,
            function_entry_pending: true,
            next_statement_event_index: 0,
            statement_events_complete: false,
        }
    }

    pub(crate) const fn child(
        function: FunctionIndex,
        instruction: InstructionIndex,
        slot_base: usize,
        slot_count: usize,
        operand_capacity: usize,
        resume_instruction: InstructionIndex,
    ) -> Self {
        Self {
            function,
            instruction,
            slot_base,
            slot_count,
            operand_capacity,
            operand_height: 0,
            resume_instruction: Some(resume_instruction),
            function_entry_pending: true,
            next_statement_event_index: 0,
            statement_events_complete: false,
        }
    }

    pub(crate) const fn replacement(
        function: FunctionIndex,
        slot_base: usize,
        slot_count: usize,
        operand_capacity: usize,
        resume_instruction: Option<InstructionIndex>,
    ) -> Self {
        Self {
            function,
            instruction: InstructionIndex::new(0),
            slot_base,
            slot_count,
            operand_capacity,
            operand_height: 0,
            resume_instruction,
            function_entry_pending: true,
            next_statement_event_index: 0,
            statement_events_complete: false,
        }
    }

    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub(crate) const fn operand_base(&self) -> usize {
        self.slot_base + self.slot_count
    }

    pub(crate) const fn slot_base(&self) -> usize {
        self.slot_base
    }

    pub(crate) const fn operand_capacity(&self) -> usize {
        self.operand_capacity
    }

    pub(crate) const fn operand_height(&self) -> usize {
        self.operand_height
    }

    pub(crate) fn set_operand_height(&mut self, height: usize) {
        self.operand_height = height;
    }

    pub(crate) const fn resume_instruction(&self) -> Option<InstructionIndex> {
        self.resume_instruction
    }

    pub(crate) const fn function_entry_pending(&self) -> bool {
        self.function_entry_pending
    }

    pub(crate) fn mark_function_entry_charged(&mut self) {
        self.function_entry_pending = false;
    }

    pub(crate) const fn statement_events_pending(&self) -> bool {
        !self.statement_events_complete
    }

    pub(crate) const fn next_statement_event_index(&self) -> usize {
        self.next_statement_event_index
    }

    pub(crate) fn advance_statement_event(&mut self) -> bool {
        let Some(next) = self.next_statement_event_index.checked_add(1) else {
            return false;
        };
        self.next_statement_event_index = next;
        true
    }

    pub(crate) fn mark_statement_events_complete(&mut self) {
        self.statement_events_complete = true;
    }

    pub(crate) fn advance_instruction(&mut self) -> bool {
        let Some(next) = self.instruction.get().checked_add(1) else {
            return false;
        };
        self.instruction = InstructionIndex::new(next);
        self.reset_statement_events();
        true
    }

    pub(crate) fn jump_to(&mut self, instruction: InstructionIndex) {
        self.instruction = instruction;
        self.reset_statement_events();
    }

    pub(crate) fn resume_to(&mut self, instruction: InstructionIndex) {
        self.instruction = instruction;
        self.reset_statement_events();
    }

    fn reset_statement_events(&mut self) {
        self.next_statement_event_index = 0;
        self.statement_events_complete = false;
    }

    pub(crate) fn segment_end(&self) -> Option<usize> {
        self.slot_base
            .checked_add(self.slot_count)
            .and_then(|slots_end| slots_end.checked_add(self.operand_capacity))
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

    use super::VmFrame;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn frame_metadata_layout_stays_compact() {
        assert!(size_of::<VmFrame>() <= 64);
    }

    #[test]
    fn advancing_rearms_source_events_without_rearming_function_entry() {
        let mut frame = VmFrame::root(FunctionIndex::new(0), 0, 0);
        frame.mark_function_entry_charged();
        assert!(frame.advance_statement_event());
        frame.mark_statement_events_complete();

        assert!(frame.advance_instruction());

        assert!(!frame.function_entry_pending());
        assert!(frame.statement_events_pending());
        assert_eq!(frame.next_statement_event_index(), 0);
    }

    #[test]
    fn child_frame_carries_return_pc_and_cleared_operand_stack() {
        let mut frame = VmFrame::child(
            FunctionIndex::new(1),
            InstructionIndex::new(0),
            8,
            3,
            4,
            InstructionIndex::new(7),
        );

        assert_eq!(frame.operand_height(), 0);
        assert_eq!(frame.operand_base(), 11);
        assert_eq!(frame.segment_end(), Some(15));
        assert_eq!(frame.resume_instruction(), Some(InstructionIndex::new(7)));
        assert!(frame.function_entry_pending());

        frame.set_operand_height(2);
        assert_eq!(frame.operand_height(), 2);
    }

    #[test]
    fn jumping_and_resuming_rearm_statement_events_without_function_entry() {
        let mut frame = VmFrame::root(FunctionIndex::new(0), 0, 0);
        frame.mark_function_entry_charged();
        frame.mark_statement_events_complete();

        frame.jump_to(InstructionIndex::new(3));
        assert_eq!(frame.instruction(), InstructionIndex::new(3));
        assert!(frame.statement_events_pending());
        assert!(!frame.function_entry_pending());

        frame.mark_statement_events_complete();
        frame.resume_to(InstructionIndex::new(9));
        assert_eq!(frame.instruction(), InstructionIndex::new(9));
        assert!(frame.statement_events_pending());
        assert!(!frame.function_entry_pending());
    }
}
