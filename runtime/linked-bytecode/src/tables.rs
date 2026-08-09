use skiff_artifact_model::{SourcePosition, ValueTransferPlanKind};

use crate::{FrameSlotIndex, InstructionIndex, ResumeSiteIndex, TypeIndex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedCatchMatcher {
    Type(TypeIndex),
    CatchAll,
}

/// Unverified linked exception-region declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedExceptionRegion {
    start: InstructionIndex,
    end: InstructionIndex,
    handler: InstructionIndex,
    handler_stack_height: u32,
    catch_matchers: Box<[LinkedCatchMatcher]>,
    catch_slot: FrameSlotIndex,
    cleanup_depth: u32,
}

impl LinkedExceptionRegion {
    pub fn new(
        start: InstructionIndex,
        end: InstructionIndex,
        handler: InstructionIndex,
        handler_stack_height: u32,
        catch_matchers: Box<[LinkedCatchMatcher]>,
        catch_slot: FrameSlotIndex,
        cleanup_depth: u32,
    ) -> Self {
        Self {
            start,
            end,
            handler,
            handler_stack_height,
            catch_matchers,
            catch_slot,
            cleanup_depth,
        }
    }

    pub const fn start(&self) -> InstructionIndex {
        self.start
    }

    pub const fn end(&self) -> InstructionIndex {
        self.end
    }

    pub const fn handler(&self) -> InstructionIndex {
        self.handler
    }

    pub const fn handler_stack_height(&self) -> u32 {
        self.handler_stack_height
    }

    pub fn catch_matchers(&self) -> &[LinkedCatchMatcher] {
        &self.catch_matchers
    }

    pub const fn catch_slot(&self) -> FrameSlotIndex {
        self.catch_slot
    }

    pub const fn cleanup_depth(&self) -> u32 {
        self.cleanup_depth
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSwitchTable {
    tag_type: TypeIndex,
    targets: Box<[InstructionIndex]>,
}

impl LinkedSwitchTable {
    pub fn new(tag_type: TypeIndex, targets: Box<[InstructionIndex]>) -> Self {
        Self { tag_type, targets }
    }

    pub const fn tag_type(&self) -> TypeIndex {
        self.tag_type
    }

    pub fn targets(&self) -> &[InstructionIndex] {
        &self.targets
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedResumeSite {
    index: ResumeSiteIndex,
    result_type: TypeIndex,
    expected_stack_height: u32,
    result_plan: ValueTransferPlanKind,
}

impl LinkedResumeSite {
    pub fn new(
        index: ResumeSiteIndex,
        result_type: TypeIndex,
        expected_stack_height: u32,
        result_plan: ValueTransferPlanKind,
    ) -> Self {
        Self {
            index,
            result_type,
            expected_stack_height,
            result_plan,
        }
    }

    pub const fn index(&self) -> ResumeSiteIndex {
        self.index
    }

    pub const fn result_type(&self) -> TypeIndex {
        self.result_type
    }

    pub const fn expected_stack_height(&self) -> u32 {
        self.expected_stack_height
    }

    pub const fn result_plan(&self) -> ValueTransferPlanKind {
        self.result_plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedStatementEntry {
    instruction: InstructionIndex,
    statement_id: String,
}

impl LinkedStatementEntry {
    pub fn new(instruction: InstructionIndex, statement_id: impl Into<String>) -> Self {
        Self {
            instruction,
            statement_id: statement_id.into(),
        }
    }

    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub fn statement_id(&self) -> &str {
        &self.statement_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSourceMapEntry {
    start: InstructionIndex,
    end: InstructionIndex,
    source_id: u64,
    start_position: SourcePosition,
    end_position: SourcePosition,
}

impl LinkedSourceMapEntry {
    pub fn new(
        start: InstructionIndex,
        end: InstructionIndex,
        source_id: u64,
        start_position: SourcePosition,
        end_position: SourcePosition,
    ) -> Self {
        Self {
            start,
            end,
            source_id,
            start_position,
            end_position,
        }
    }

    pub const fn start(&self) -> InstructionIndex {
        self.start
    }

    pub const fn end(&self) -> InstructionIndex {
        self.end
    }

    pub const fn source_id(&self) -> u64 {
        self.source_id
    }

    pub const fn start_position(&self) -> &SourcePosition {
        &self.start_position
    }

    pub const fn end_position(&self) -> &SourcePosition {
        &self.end_position
    }
}
