use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Private segmented frame descriptor. Values live in the fiber's contiguous
/// storage rather than in this metadata object.
#[derive(Debug)]
pub(crate) struct VmFrame {
    function: FunctionIndex,
    instruction: InstructionIndex,
    operand_base: usize,
    operand_capacity: usize,
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
        operand_base: usize,
        operand_capacity: usize,
    ) -> Self {
        Self {
            function,
            instruction: InstructionIndex::new(0),
            operand_base,
            operand_capacity,
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
        self.next_statement_event_index = 0;
        self.statement_events_complete = false;
        true
    }

    pub(crate) fn segment_end(&self) -> Option<usize> {
        self.operand_base.checked_add(self.operand_capacity)
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use skiff_runtime_linked_bytecode::FunctionIndex;

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
}
