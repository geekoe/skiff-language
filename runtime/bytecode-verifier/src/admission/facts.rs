use std::collections::BTreeSet;

use skiff_artifact_model::{
    CallableEffectSummary, InstructionSourceSite, PackageCallableId, StatementAttributionId,
};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

mod resume;

pub(crate) use resume::{ExactResumeBinding, ExactResumeEntry};

/// Private P1 facts derived from the exact hydration/candidate pair.
#[derive(Debug)]
pub(crate) struct AdmissionFacts {
    statements: ExactStatementBinding,
    effects: ExactCanonicalEffectBinding,
    resumes: ExactResumeBinding,
}

impl AdmissionFacts {
    pub(super) const fn new(
        statements: ExactStatementBinding,
        effects: ExactCanonicalEffectBinding,
        resumes: ExactResumeBinding,
    ) -> Self {
        Self {
            statements,
            effects,
            resumes,
        }
    }

    pub(crate) const fn statement_binding(&self) -> &ExactStatementBinding {
        &self.statements
    }

    pub(crate) const fn effect_binding(&self) -> &ExactCanonicalEffectBinding {
        &self.effects
    }

    pub(crate) const fn resume_binding(&self) -> &ExactResumeBinding {
        &self.resumes
    }
}

/// Exact effect authority, indexed by dense linked function.
///
/// The summary is copied only from the canonical package callable after the
/// linked declaration exact-matches it. Local-ABI declarations remain
/// separate facts: in particular, their `may_suspend = false` value never
/// turns an unknown canonical summary into a NoPending proof.
#[derive(Debug)]
pub(crate) struct ExactCanonicalEffectBinding {
    functions: Box<[ExactFunctionEffectBinding]>,
}

impl ExactCanonicalEffectBinding {
    pub(super) fn new(functions: Box<[ExactFunctionEffectBinding]>) -> Self {
        Self { functions }
    }

    pub(crate) fn functions(&self) -> &[ExactFunctionEffectBinding] {
        &self.functions
    }

    #[cfg(test)]
    pub(crate) fn function(&self, function: FunctionIndex) -> Option<&ExactFunctionEffectBinding> {
        self.functions
            .get(function.get() as usize)
            .filter(|binding| binding.function == function)
    }

    /// Rechecks the opaque P1 effect frontier before the semantic gate uses it.
    ///
    /// This is intentionally independent of the linked candidate: it proves
    /// only the dense/self-consistent shape of the verifier-owned token and
    /// never turns an unknown summary into a NoPending proof.
    pub(crate) fn frontier_summary(
        &self,
    ) -> Result<ExactEffectFrontierSummary<'_>, ExactEffectFrontierViolation> {
        let mut unknown_summary_count = 0usize;
        let mut local_abi_declaration_count = 0usize;
        let mut first_canonical_callable = None;

        for (ordinal, binding) in self.functions.iter().enumerate() {
            let expected = u32::try_from(ordinal)
                .map(FunctionIndex::new)
                .map_err(|_| {
                    ExactEffectFrontierViolation::at_image(
                        "effect frontier function ordinal does not fit u32",
                    )
                })?;
            if binding.function != expected {
                return Err(ExactEffectFrontierViolation::at_function(
                    expected,
                    "effect frontier function rows are not dense",
                ));
            }
            if binding.canonical_callable.as_str().is_empty() {
                return Err(ExactEffectFrontierViolation::at_function(
                    expected,
                    "effect frontier canonical callable is empty",
                ));
            }
            if first_canonical_callable.is_none() {
                first_canonical_callable = Some(&binding.canonical_callable);
            }

            match &binding.summary {
                CallableEffectSummary::Unknown { .. } => unknown_summary_count += 1,
                CallableEffectSummary::Analyzed { effects } => {
                    if effects.may_pending != effects.may_pending() {
                        return Err(ExactEffectFrontierViolation::at_function(
                            expected,
                            "effect frontier analyzed mayPending disagrees with its categories",
                        ));
                    }
                    let mut categories = BTreeSet::new();
                    if effects
                        .pending_effect_categories
                        .iter()
                        .copied()
                        .any(|category| !categories.insert(category))
                    {
                        return Err(ExactEffectFrontierViolation::at_function(
                            expected,
                            "effect frontier pending categories contain a duplicate",
                        ));
                    }
                }
            }

            let mut alias_callables = BTreeSet::new();
            let mut declared_may_suspend = None;
            for declaration in &binding.local_abi_declarations {
                local_abi_declaration_count += 1;
                if !alias_callables.insert(declaration.callable.as_str()) {
                    return Err(ExactEffectFrontierViolation::at_function(
                        expected,
                        "effect frontier repeats a Local ABI callable declaration",
                    ));
                }
                match declared_may_suspend {
                    None => declared_may_suspend = Some(declaration.may_suspend),
                    Some(value) if value != declaration.may_suspend => {
                        return Err(ExactEffectFrontierViolation::at_function(
                            expected,
                            "effect frontier Local ABI aliases disagree on maySuspend",
                        ));
                    }
                    Some(_) => {}
                }
            }
            let Some(declared_may_suspend) = declared_may_suspend else {
                return Err(ExactEffectFrontierViolation::at_function(
                    expected,
                    "effect frontier has no Local ABI declaration",
                ));
            };
            if let CallableEffectSummary::Analyzed { effects } = &binding.summary {
                if declared_may_suspend != effects.may_pending {
                    return Err(ExactEffectFrontierViolation::at_function(
                        expected,
                        "effect frontier Local ABI maySuspend disagrees with analyzed mayPending",
                    ));
                }
            }
        }

        Ok(ExactEffectFrontierSummary {
            function_count: self.functions.len(),
            unknown_summary_count,
            local_abi_declaration_count,
            first_canonical_callable,
        })
    }
}

/// Small aggregate produced while defensively checking every opaque effect
/// authority row. Fields stay private to preserve the token boundary.
pub(crate) struct ExactEffectFrontierSummary<'a> {
    function_count: usize,
    unknown_summary_count: usize,
    local_abi_declaration_count: usize,
    first_canonical_callable: Option<&'a PackageCallableId>,
}

impl ExactEffectFrontierSummary<'_> {
    pub(crate) fn cross_proof_mismatch_detail(
        &self,
        control_flow_function_count: usize,
        exact_call_function_count: usize,
        statement_function_count: usize,
    ) -> Option<String> {
        if self.function_count == control_flow_function_count
            && self.function_count == exact_call_function_count
            && self.function_count == statement_function_count
        {
            return None;
        }
        Some(format!(
            "effect frontier shape disagrees across proof tokens: effects={}, controlFlow={}, exactCalls={}, statements={}, unknownSummaries={}, localAbiDeclarations={}, firstCanonical={}",
            self.function_count,
            control_flow_function_count,
            exact_call_function_count,
            statement_function_count,
            self.unknown_summary_count,
            self.local_abi_declaration_count,
            self.first_canonical_callable
                .map_or("<none>", PackageCallableId::as_str),
        ))
    }
}

pub(crate) struct ExactEffectFrontierViolation {
    function: Option<FunctionIndex>,
    detail: String,
}

impl ExactEffectFrontierViolation {
    fn at_image(detail: impl Into<String>) -> Self {
        Self {
            function: None,
            detail: detail.into(),
        }
    }

    fn at_function(function: FunctionIndex, detail: impl Into<String>) -> Self {
        Self {
            function: Some(function),
            detail: detail.into(),
        }
    }

    pub(crate) fn into_parts(self) -> (Option<FunctionIndex>, String) {
        (self.function, self.detail)
    }
}

/// Canonical semantic effect facts and all Local-ABI declarations that map
/// to one exact admitted function specialization.
#[derive(Debug)]
pub(crate) struct ExactFunctionEffectBinding {
    function: FunctionIndex,
    canonical_callable: PackageCallableId,
    summary: CallableEffectSummary,
    local_abi_declarations: Box<[ExactLocalAbiEffectDeclaration]>,
}

impl ExactFunctionEffectBinding {
    pub(super) fn new(
        function: FunctionIndex,
        canonical_callable: PackageCallableId,
        summary: CallableEffectSummary,
        local_abi_declarations: Box<[ExactLocalAbiEffectDeclaration]>,
    ) -> Self {
        Self {
            function,
            canonical_callable,
            summary,
            local_abi_declarations,
        }
    }

    pub(crate) const fn canonical_callable(&self) -> &PackageCallableId {
        &self.canonical_callable
    }

    pub(crate) const fn summary(&self) -> &CallableEffectSummary {
        &self.summary
    }

    pub(crate) fn local_abi_declarations(&self) -> &[ExactLocalAbiEffectDeclaration] {
        &self.local_abi_declarations
    }
}

/// One canonical Package Local ABI declaration that selects an exact
/// admitted function.
#[derive(Debug)]
pub(crate) struct ExactLocalAbiEffectDeclaration {
    callable: PackageCallableId,
    may_suspend: bool,
}

impl ExactLocalAbiEffectDeclaration {
    pub(super) const fn new(callable: PackageCallableId, may_suspend: bool) -> Self {
        Self {
            callable,
            may_suspend,
        }
    }

    pub(crate) const fn callable(&self) -> &PackageCallableId {
        &self.callable
    }

    pub(crate) const fn may_suspend(&self) -> bool {
        self.may_suspend
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
