use std::fmt;

use skiff_artifact_model::{InstructionSourceSite, ResumeErrorMode, StatementAttributionId};

use crate::{
    ActiveRegionIndex, ArtifactWritablePathIndex, FrameSlotIndex, FunctionIndex,
    InstructionBoundaryIndex, InstructionIndex, LinkedArtifactPoolOrigin, LinkedValueTransferPlan,
    ResumeSiteIndex, ShapeIndex, TypeIndex, WritablePathIndex,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedCatchMatcher {
    Type(TypeIndex),
    CatchAll,
}

/// Linked exception-region declaration with image-local typed references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedExceptionRegion {
    start: InstructionIndex,
    end: InstructionBoundaryIndex,
    handler: InstructionIndex,
    handler_stack_height: u32,
    catch_matchers: Box<[LinkedCatchMatcher]>,
    catch_slot: FrameSlotIndex,
    catch_slot_type: TypeIndex,
    cleanup_depth: u32,
}

impl LinkedExceptionRegion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        start: InstructionIndex,
        end: InstructionBoundaryIndex,
        handler: InstructionIndex,
        handler_stack_height: u32,
        catch_matchers: Box<[LinkedCatchMatcher]>,
        catch_slot: FrameSlotIndex,
        catch_slot_type: TypeIndex,
        cleanup_depth: u32,
    ) -> Self {
        Self {
            start,
            end,
            handler,
            handler_stack_height,
            catch_matchers,
            catch_slot,
            catch_slot_type,
            cleanup_depth,
        }
    }

    pub const fn start(&self) -> InstructionIndex {
        self.start
    }

    pub const fn end(&self) -> InstructionBoundaryIndex {
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

    pub const fn catch_slot_type(&self) -> TypeIndex {
        self.catch_slot_type
    }

    pub const fn cleanup_depth(&self) -> u32 {
        self.cleanup_depth
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkedSwitchCase {
    tag_type: TypeIndex,
    target: InstructionIndex,
}

impl LinkedSwitchCase {
    pub const fn new(tag_type: TypeIndex, target: InstructionIndex) -> Self {
        Self { tag_type, target }
    }

    pub const fn tag_type(&self) -> TypeIndex {
        self.tag_type
    }

    pub const fn target(&self) -> InstructionIndex {
        self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSwitchTable {
    cases: Box<[LinkedSwitchCase]>,
    default_target: InstructionIndex,
}

impl LinkedSwitchTable {
    pub fn try_new(
        cases: Box<[LinkedSwitchCase]>,
        default_target: InstructionIndex,
    ) -> Result<Self, LinkedSwitchTableError> {
        let mut previous = None;
        for case in &cases {
            if let Some(previous) = previous {
                if case.tag_type() <= previous {
                    return Err(LinkedSwitchTableError::NonCanonicalTagOrder {
                        previous,
                        current: case.tag_type(),
                    });
                }
            }
            previous = Some(case.tag_type());
        }
        Ok(Self {
            cases,
            default_target,
        })
    }

    pub fn cases(&self) -> &[LinkedSwitchCase] {
        &self.cases
    }

    pub const fn default_target(&self) -> InstructionIndex {
        self.default_target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedSwitchTableError {
    NonCanonicalTagOrder {
        previous: TypeIndex,
        current: TypeIndex,
    },
}

impl fmt::Display for LinkedSwitchTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalTagOrder { previous, current } => write!(
                formatter,
                "switch tag type {} must sort after {}",
                current.get(),
                previous.get()
            ),
        }
    }
}

impl std::error::Error for LinkedSwitchTableError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedResumeSite {
    index: ResumeSiteIndex,
    function: FunctionIndex,
    site: InstructionIndex,
    resume: InstructionIndex,
    end_resume: Option<InstructionIndex>,
    expected_stack_height_before_result: u32,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    error_mode: ResumeErrorMode,
}

impl LinkedResumeSite {
    /// The referenced function binds this descriptor to the exact
    /// [`crate::SpecializationKey`] that supplied any substituted types and
    /// lifecycle plans. The row remains untrusted until artifact comparison.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: ResumeSiteIndex,
        function: FunctionIndex,
        site: InstructionIndex,
        resume: InstructionIndex,
        end_resume: Option<InstructionIndex>,
        expected_stack_height_before_result: u32,
        result_types: Box<[TypeIndex]>,
        result_plans: Box<[LinkedValueTransferPlan]>,
        error_mode: ResumeErrorMode,
    ) -> Result<Self, LinkedResumeSiteError> {
        if result_types.len() != result_plans.len() {
            return Err(LinkedResumeSiteError::ResultPlanCountMismatch {
                result_type_count: result_types.len(),
                result_plan_count: result_plans.len(),
            });
        }
        Ok(Self {
            index,
            function,
            site,
            resume,
            end_resume,
            expected_stack_height_before_result,
            result_types,
            result_plans,
            error_mode,
        })
    }

    pub const fn index(&self) -> ResumeSiteIndex {
        self.index
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn site(&self) -> InstructionIndex {
        self.site
    }

    pub const fn resume(&self) -> InstructionIndex {
        self.resume
    }

    pub const fn end_resume(&self) -> Option<InstructionIndex> {
        self.end_resume
    }

    pub const fn expected_stack_height_before_result(&self) -> u32 {
        self.expected_stack_height_before_result
    }

    pub fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    pub const fn error_mode(&self) -> ResumeErrorMode {
        self.error_mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedResumeSiteError {
    ResultPlanCountMismatch {
        result_type_count: usize,
        result_plan_count: usize,
    },
}

impl fmt::Display for LinkedResumeSiteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResultPlanCountMismatch {
                result_type_count,
                result_plan_count,
            } => write!(
                formatter,
                "resume site has {result_type_count} result types but {result_plan_count} result plans"
            ),
        }
    }
}

impl std::error::Error for LinkedResumeSiteError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedStatementEntry {
    instruction: InstructionIndex,
    sequence_ordinal: u32,
    attribution_id: StatementAttributionId,
    site: InstructionSourceSite,
}

impl LinkedStatementEntry {
    pub fn new(
        instruction: InstructionIndex,
        sequence_ordinal: u32,
        attribution_id: StatementAttributionId,
        site: InstructionSourceSite,
    ) -> Self {
        Self {
            instruction,
            sequence_ordinal,
            attribution_id,
            site,
        }
    }

    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub const fn sequence_ordinal(&self) -> u32 {
        self.sequence_ordinal
    }

    pub const fn attribution_id(&self) -> StatementAttributionId {
        self.attribution_id
    }

    pub const fn site(&self) -> &InstructionSourceSite {
        &self.site
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSourceMapEntry {
    start: InstructionIndex,
    end: InstructionBoundaryIndex,
    site: InstructionSourceSite,
}

impl LinkedSourceMapEntry {
    pub fn new(
        start: InstructionIndex,
        end: InstructionBoundaryIndex,
        site: InstructionSourceSite,
    ) -> Self {
        Self { start, end, site }
    }

    pub const fn start(&self) -> InstructionIndex {
        self.start
    }

    pub const fn end(&self) -> InstructionBoundaryIndex {
        self.end
    }

    pub const fn site(&self) -> &InstructionSourceSite {
        &self.site
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedActiveRegionKind {
    Timeout {
        duration_ms: u64,
        site: InstructionSourceSite,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedActiveRegion {
    index: ActiveRegionIndex,
    start: InstructionIndex,
    end: InstructionBoundaryIndex,
    kind: LinkedActiveRegionKind,
}

impl LinkedActiveRegion {
    pub fn new(
        index: ActiveRegionIndex,
        start: InstructionIndex,
        end: InstructionBoundaryIndex,
        kind: LinkedActiveRegionKind,
    ) -> Self {
        Self {
            index,
            start,
            end,
            kind,
        }
    }

    pub const fn index(&self) -> ActiveRegionIndex {
        self.index
    }

    pub const fn start(&self) -> InstructionIndex {
        self.start
    }

    pub const fn end(&self) -> InstructionBoundaryIndex {
        self.end
    }

    pub const fn kind(&self) -> &LinkedActiveRegionKind {
        &self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedWritablePathSegment {
    DenseField {
        shape: ShapeIndex,
        field_ordinal: u32,
    },
    ArrayIndex {
        selector_ordinal: u32,
        element_type: TypeIndex,
    },
    MapKey {
        selector_ordinal: u32,
        key_type: TypeIndex,
        value_type: TypeIndex,
    },
}

impl LinkedWritablePathSegment {
    pub const fn selector_ordinal(&self) -> Option<u32> {
        match self {
            Self::DenseField { .. } => None,
            Self::ArrayIndex {
                selector_ordinal, ..
            }
            | Self::MapKey {
                selector_ordinal, ..
            } => Some(*selector_ordinal),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedWritablePathEntry {
    index: WritablePathIndex,
    origin: LinkedArtifactPoolOrigin<ArtifactWritablePathIndex>,
    root_type: TypeIndex,
    leaf_type: TypeIndex,
    segments: Box<[LinkedWritablePathSegment]>,
    selector_count: u32,
}

impl LinkedWritablePathEntry {
    pub fn new(
        index: WritablePathIndex,
        origin: LinkedArtifactPoolOrigin<ArtifactWritablePathIndex>,
        root_type: TypeIndex,
        leaf_type: TypeIndex,
        segments: Box<[LinkedWritablePathSegment]>,
    ) -> Result<Self, LinkedWritablePathError> {
        let mut next_selector = 0;
        for (segment_index, segment) in segments.iter().enumerate() {
            if let Some(actual) = segment.selector_ordinal() {
                if actual != next_selector {
                    return Err(LinkedWritablePathError::NonDenseSelector {
                        segment_index,
                        expected: next_selector,
                        actual,
                    });
                }
                next_selector = next_selector
                    .checked_add(1)
                    .ok_or(LinkedWritablePathError::SelectorCountOverflow)?;
            }
        }
        Ok(Self {
            index,
            origin,
            root_type,
            leaf_type,
            segments,
            selector_count: next_selector,
        })
    }

    pub const fn index(&self) -> WritablePathIndex {
        self.index
    }

    pub const fn origin(&self) -> &LinkedArtifactPoolOrigin<ArtifactWritablePathIndex> {
        &self.origin
    }

    pub const fn root_type(&self) -> TypeIndex {
        self.root_type
    }

    pub const fn leaf_type(&self) -> TypeIndex {
        self.leaf_type
    }

    pub fn segments(&self) -> &[LinkedWritablePathSegment] {
        &self.segments
    }

    pub const fn selector_count(&self) -> u32 {
        self.selector_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedWritablePathError {
    NonDenseSelector {
        segment_index: usize,
        expected: u32,
        actual: u32,
    },
    SelectorCountOverflow,
}

impl fmt::Display for LinkedWritablePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonDenseSelector {
                segment_index,
                expected,
                actual,
            } => write!(
                formatter,
                "writable path segment {segment_index} has selector ordinal {actual}; expected {expected}"
            ),
            Self::SelectorCountOverflow => {
                formatter.write_str("writable path selector count exceeds u32")
            }
        }
    }
}

impl std::error::Error for LinkedWritablePathError {}
