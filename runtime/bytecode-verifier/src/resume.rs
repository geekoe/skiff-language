use skiff_artifact_model::{InstructionSourceSite, ResumeErrorMode};
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, FunctionIndex, InstructionIndex, LinkedValueTransferPlan, ResumeSiteIndex,
    TypeIndex,
};

/// Immutable verifier certificate for all admitted pending/resume sites.
#[derive(Debug)]
pub struct VerifiedResumeSites {
    rows: Box<[VerifiedResumeSite]>,
}

impl VerifiedResumeSites {
    pub(crate) fn new(rows: Box<[VerifiedResumeSite]>) -> Self {
        Self { rows }
    }

    pub fn rows(&self) -> &[VerifiedResumeSite] {
        &self.rows
    }

    pub fn get(&self, index: ResumeSiteIndex) -> Option<&VerifiedResumeSite> {
        self.rows
            .get(index.get() as usize)
            .filter(|row| row.index == index)
    }
}

/// Exact successful-item and error-route proof for one resume descriptor.
///
/// The first slice deliberately certifies no natural-end outcome: the ISA has
/// not assigned that outcome a control-flow or stack contract yet.
#[derive(Debug)]
pub struct VerifiedResumeSite {
    index: ResumeSiteIndex,
    function: FunctionIndex,
    site: InstructionIndex,
    resume: InstructionIndex,
    expected_stack_height_before_result: u32,
    result_type: TypeIndex,
    result_plan: LinkedValueTransferPlan,
    error_mode: ResumeErrorMode,
    original_site: InstructionSourceSite,
    kind: VerifiedResumeKind,
}

pub(crate) struct VerifiedResumeSiteParts {
    pub(crate) index: ResumeSiteIndex,
    pub(crate) function: FunctionIndex,
    pub(crate) site: InstructionIndex,
    pub(crate) resume: InstructionIndex,
    pub(crate) expected_stack_height_before_result: u32,
    pub(crate) result_type: TypeIndex,
    pub(crate) result_plan: LinkedValueTransferPlan,
    pub(crate) error_mode: ResumeErrorMode,
    pub(crate) original_site: InstructionSourceSite,
    pub(crate) kind: VerifiedResumeKind,
}

impl VerifiedResumeSite {
    pub(crate) fn from_parts(parts: VerifiedResumeSiteParts) -> Self {
        Self {
            index: parts.index,
            function: parts.function,
            site: parts.site,
            resume: parts.resume,
            expected_stack_height_before_result: parts.expected_stack_height_before_result,
            result_type: parts.result_type,
            result_plan: parts.result_plan,
            error_mode: parts.error_mode,
            original_site: parts.original_site,
            kind: parts.kind,
        }
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

    pub const fn expected_stack_height_before_result(&self) -> u32 {
        self.expected_stack_height_before_result
    }

    pub const fn result_type(&self) -> TypeIndex {
        self.result_type
    }

    pub const fn result_plan(&self) -> &LinkedValueTransferPlan {
        &self.result_plan
    }

    pub const fn error_mode(&self) -> ResumeErrorMode {
        self.error_mode
    }

    pub const fn original_site(&self) -> &InstructionSourceSite {
        &self.original_site
    }

    pub const fn kind(&self) -> &VerifiedResumeKind {
        &self.kind
    }
}

/// Pending semantics whose stack and slot transfers are fully certified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedResumeKind {
    StreamRead {
        endpoint_slot: FrameSlotIndex,
        item_type: TypeIndex,
    },
    StreamBackpressure,
    ServiceBoundary,
    ActorBoundary,
    InterfaceBoundary,
    CallbackBoundary,
    HostEffect,
}
