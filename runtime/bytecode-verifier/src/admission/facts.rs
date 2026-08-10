use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

/// Private P1 facts derived from the exact hydration/candidate pair.
#[derive(Debug)]
pub(crate) struct AdmissionFacts {
    statements: ExactStatementBinding,
}

impl AdmissionFacts {
    pub(super) const fn new(statements: ExactStatementBinding) -> Self {
        Self { statements }
    }

    pub(crate) const fn statement_binding(&self) -> &ExactStatementBinding {
        &self.statements
    }
}

/// Exact admitted statement placements, indexed by dense linked function.
///
/// Construction remains inside P1. P2 consumes these facts instead of
/// treating candidate statement rows as their own authority.
#[derive(Debug)]
pub(crate) struct ExactStatementBinding {
    functions: Box<[ExactFunctionStatementBinding]>,
}

impl ExactStatementBinding {
    pub(super) fn new(functions: Box<[ExactFunctionStatementBinding]>) -> Self {
        Self { functions }
    }

    pub(crate) fn functions(&self) -> &[ExactFunctionStatementBinding] {
        &self.functions
    }

    pub(crate) fn function(
        &self,
        function: FunctionIndex,
    ) -> Option<&ExactFunctionStatementBinding> {
        self.functions
            .get(function.get() as usize)
            .filter(|binding| binding.function == function)
    }
}

/// P1-bound statement rows for one exact function specialization.
#[derive(Debug)]
pub(crate) struct ExactFunctionStatementBinding {
    function: FunctionIndex,
    entries: Box<[ExactStatementEntry]>,
}

impl ExactFunctionStatementBinding {
    pub(super) fn new(function: FunctionIndex, entries: Box<[ExactStatementEntry]>) -> Self {
        Self { function, entries }
    }

    pub(crate) fn entries(&self) -> &[ExactStatementEntry] {
        &self.entries
    }
}

/// One statement row copied from admitted authority only after every linked
/// field exact-matches it.
#[derive(Debug)]
pub(crate) struct ExactStatementEntry {
    instruction: InstructionIndex,
    sequence_ordinal: u32,
    attribution_id: StatementAttributionId,
    site: InstructionSourceSite,
}

impl ExactStatementEntry {
    pub(super) fn new(
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

    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub(crate) const fn sequence_ordinal(&self) -> u32 {
        self.sequence_ordinal
    }

    pub(crate) const fn attribution_id(&self) -> StatementAttributionId {
        self.attribution_id
    }

    pub(crate) const fn site(&self) -> &InstructionSourceSite {
        &self.site
    }
}
