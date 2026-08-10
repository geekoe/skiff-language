use skiff_artifact_model::ResumeErrorMode;
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedValueTransferPlan, ResumeSiteIndex, TypeIndex,
};

/// P1-owned exact resume descriptor rows in dense linked index order.
#[derive(Debug)]
pub(crate) struct ExactResumeBinding {
    rows: Box<[ExactResumeEntry]>,
}

impl ExactResumeBinding {
    pub(in crate::admission) fn new(rows: Box<[ExactResumeEntry]>) -> Self {
        Self { rows }
    }

    pub(crate) fn rows(&self) -> &[ExactResumeEntry] {
        &self.rows
    }

    pub(crate) fn row(&self, index: ResumeSiteIndex) -> Option<&ExactResumeEntry> {
        self.rows
            .get(index.get() as usize)
            .filter(|row| row.index == index)
    }
}

/// One descriptor copied only after artifact, specialization, and linked row
/// coordinates exact-match at P1.
#[derive(Debug)]
pub(crate) struct ExactResumeEntry {
    index: ResumeSiteIndex,
    artifact_descriptor_index: u32,
    function: FunctionIndex,
    site: InstructionIndex,
    resume: InstructionIndex,
    expected_stack_height_before_result: u32,
    result_types: Box<[TypeIndex]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    error_mode: ResumeErrorMode,
}

impl ExactResumeEntry {
    pub(in crate::admission) fn from_linked(
        row: &skiff_runtime_linked_bytecode::LinkedResumeSite,
        artifact_descriptor_index: u32,
    ) -> Self {
        Self {
            index: row.index(),
            artifact_descriptor_index,
            function: row.function(),
            site: row.site(),
            resume: row.resume(),
            expected_stack_height_before_result: row.expected_stack_height_before_result(),
            result_types: row.result_types().into(),
            result_plans: row.result_plans().into(),
            error_mode: row.error_mode(),
        }
    }

    pub(crate) const fn index(&self) -> ResumeSiteIndex {
        self.index
    }

    pub(crate) const fn artifact_descriptor_index(&self) -> u32 {
        self.artifact_descriptor_index
    }

    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) const fn site(&self) -> InstructionIndex {
        self.site
    }

    pub(crate) const fn resume(&self) -> InstructionIndex {
        self.resume
    }

    pub(crate) const fn expected_stack_height_before_result(&self) -> u32 {
        self.expected_stack_height_before_result
    }

    pub(crate) fn result_types(&self) -> &[TypeIndex] {
        &self.result_types
    }

    pub(crate) fn result_plans(&self) -> &[LinkedValueTransferPlan] {
        &self.result_plans
    }

    pub(crate) const fn error_mode(&self) -> ResumeErrorMode {
        self.error_mode
    }
}
