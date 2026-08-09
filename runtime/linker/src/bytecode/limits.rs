use crate::bytecode::{
    BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation, BytecodeLinkObligation,
};

/// Trusted ceilings applied while linking one exact consumer deployment.
///
/// Every ceiling is explicit. There is intentionally no `Default`
/// implementation: the deployment owner must select a complete link policy
/// instead of silently inheriting permissive process-wide limits.
///
/// ```compile_fail
/// use skiff_runtime_linker::LinkLimits;
///
/// let _ = LinkLimits::default();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkLimits {
    /// Maximum packages in the exact consumer package closure.
    pub max_packages: u64,
    /// Maximum canonical roots admitted to the specialization worklist.
    pub max_root_specializations: u64,
    /// Maximum distinct concrete specialization keys in the linked image.
    pub max_specializations: u64,
    /// Maximum artifact code words copied into one concrete specialization.
    pub max_code_words_per_function: u64,
    /// Maximum artifact code words across all concrete specializations.
    pub max_total_code_words: u64,
    /// Maximum relocations processed for one concrete specialization.
    pub max_relocations_per_function: u64,
    /// Maximum relocations processed across all concrete specializations.
    pub max_total_relocations: u64,
    /// Maximum rows in any one image-local target or data table.
    pub max_image_table_entries: u64,
    /// Maximum rows summed across all image-local target and data tables.
    pub max_total_image_table_entries: u64,
    /// Maximum rows copied into function-local exception, switch, statement
    /// and source tables across all specializations.
    pub max_total_function_table_entries: u64,
    /// Maximum nesting depth of a fully concrete linked type.
    pub max_type_nesting_depth: u64,
    /// Maximum nodes in the deployment-wide constant initialization plan.
    pub max_constant_graph_nodes: u64,
    /// Maximum edges in the deployment-wide constant initialization plan.
    pub max_constant_graph_edges: u64,
}

/// Checked deployment-link counters shared by closure discovery and concrete
/// candidate construction.
///
/// The tracker owns no artifact facts. Callers account for work immediately
/// before reserving or copying it, so a failed counter update cannot leave a
/// partially publishable candidate behind.
#[derive(Debug)]
pub(super) struct LinkLimitTracker<'a> {
    limits: &'a LinkLimits,
    total_code_words: u64,
    total_relocations: u64,
    total_image_table_entries: u64,
    total_function_table_entries: u64,
}

#[allow(dead_code)]
impl<'a> LinkLimitTracker<'a> {
    pub(super) const fn new(limits: &'a LinkLimits) -> Self {
        Self {
            limits,
            total_code_words: 0,
            total_relocations: 0,
            total_image_table_entries: 0,
            total_function_table_entries: 0,
        }
    }

    pub(super) fn check_packages(
        &self,
        actual: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::Packages,
            actual,
            self.limits.max_packages,
            location,
        )
    }

    pub(super) fn check_root_specializations(
        &self,
        actual: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::RootSpecializations,
            actual,
            self.limits.max_root_specializations,
            location,
        )
    }

    pub(super) fn check_specializations(
        &self,
        actual: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::Specializations,
            actual,
            self.limits.max_specializations,
            location,
        )
    }

    pub(super) fn add_function(
        &mut self,
        code_words: u64,
        relocations: u64,
        function_table_entries: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::CodeWordsPerFunction,
            code_words,
            self.limits.max_code_words_per_function,
            location.clone(),
        )?;
        check_limit(
            BytecodeLinkLimit::RelocationsPerFunction,
            relocations,
            self.limits.max_relocations_per_function,
            location.clone(),
        )?;
        self.total_code_words = checked_total(
            self.total_code_words,
            code_words,
            BytecodeLinkObligation::ConcreteSpecialization,
            location.clone(),
            "summing specialization code words",
        )?;
        check_limit(
            BytecodeLinkLimit::TotalCodeWords,
            self.total_code_words,
            self.limits.max_total_code_words,
            location.clone(),
        )?;
        self.total_relocations = checked_total(
            self.total_relocations,
            relocations,
            BytecodeLinkObligation::RelocationResolution,
            location.clone(),
            "summing specialization relocations",
        )?;
        check_limit(
            BytecodeLinkLimit::TotalRelocations,
            self.total_relocations,
            self.limits.max_total_relocations,
            location.clone(),
        )?;
        self.total_function_table_entries = checked_total(
            self.total_function_table_entries,
            function_table_entries,
            BytecodeLinkObligation::SourceAndStatementTables,
            location.clone(),
            "summing function-local table rows",
        )?;
        check_limit(
            BytecodeLinkLimit::TotalFunctionTableEntries,
            self.total_function_table_entries,
            self.limits.max_total_function_table_entries,
            location,
        )
    }

    pub(super) fn add_image_table(
        &mut self,
        entries: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::ImageTableEntries,
            entries,
            self.limits.max_image_table_entries,
            location.clone(),
        )?;
        self.total_image_table_entries = checked_total(
            self.total_image_table_entries,
            entries,
            BytecodeLinkObligation::ConcreteTargetTables,
            location.clone(),
            "summing image-local table rows",
        )?;
        check_limit(
            BytecodeLinkLimit::TotalImageTableEntries,
            self.total_image_table_entries,
            self.limits.max_total_image_table_entries,
            location,
        )
    }

    pub(super) fn check_type_depth(
        &self,
        depth: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::TypeNestingDepth,
            depth,
            self.limits.max_type_nesting_depth,
            location,
        )
    }

    pub(super) fn check_constant_graph(
        &self,
        nodes: u64,
        edges: u64,
        location: BytecodeLinkLocation,
    ) -> Result<(), BytecodeLinkError> {
        check_limit(
            BytecodeLinkLimit::ConstantGraphNodes,
            nodes,
            self.limits.max_constant_graph_nodes,
            location.clone(),
        )?;
        check_limit(
            BytecodeLinkLimit::ConstantGraphEdges,
            edges,
            self.limits.max_constant_graph_edges,
            location,
        )
    }
}

fn check_limit(
    limit: BytecodeLinkLimit,
    actual: u64,
    max: u64,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if actual > max {
        return Err(BytecodeLinkError::LimitExceeded {
            limit,
            actual,
            max,
            location,
        });
    }
    Ok(())
}

fn checked_total(
    current: u64,
    additional: u64,
    obligation: BytecodeLinkObligation,
    location: BytecodeLinkLocation,
    context: &'static str,
) -> Result<u64, BytecodeLinkError> {
    current
        .checked_add(additional)
        .ok_or_else(|| BytecodeLinkError::UnsatisfiedObligation {
            obligation,
            location,
            detail: format!("arithmetic overflow while {context}"),
        })
}
