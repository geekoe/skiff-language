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
