/// Trusted ceilings applied while independently verifying one linked image.
///
/// Every ceiling is explicit. There is intentionally no `Default`
/// implementation: the deployment owner must select a complete verification
/// policy rather than silently inheriting permissive process-wide limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationLimits {
    /// Maximum number of concrete functions in the image.
    pub max_functions: u64,
    /// Maximum sum of semantic instructions across all functions.
    pub max_total_instructions: u64,
    /// Maximum semantic instructions in one function.
    pub max_instructions_per_function: u64,
    /// Maximum frame slots declared by one function.
    pub max_frame_slots_per_function: u64,
    /// Maximum independently computed operand-stack depth.
    pub max_operand_depth: u64,
    /// Maximum call, branch, handler and fallthrough edges in one function.
    pub max_control_flow_edges_per_function: u64,
    /// Maximum exception-region rows in one function.
    pub max_exception_regions_per_function: u64,
    /// Maximum switch targets summed across one function.
    pub max_switch_targets_per_function: u64,
    /// Maximum statement and source-map rows summed across one function.
    pub max_debug_entries_per_function: u64,
    /// Maximum rows in each image-local target or data table.
    pub max_image_table_entries: u64,
    /// Maximum call, callback, record or collection arity.
    pub max_arity: u64,
    /// Maximum captures in one synthetic callback.
    pub max_callback_captures_per_callback: u64,
    /// Maximum nesting depth of one concrete linked type.
    pub max_type_nesting_depth: u64,
    /// Maximum edges in the complete linked constant graph.
    pub max_constant_graph_edges: u64,
}
