use skiff_artifact_model::{bytecode::EncodeError, StructuralValidationError};
use skiff_compiler_lowering::{mir::MirContractError, FrozenConstantLookupError};
use thiserror::Error;

/// A fail-closed bytecode emission failure.
///
/// No variant carries a partial artifact. The public emitter returns a
/// `BytecodeArtifact` only after canonical opcode validation, complete input
/// coverage, structural validation and identity assignment all succeed.
#[derive(Debug, Error)]
pub enum BytecodeEmissionError {
    #[error(
        "bytecode emitter opcode fingerprint mismatch: supplied `{supplied}`, canonical `{canonical}`"
    )]
    OpcodeFingerprintMismatch { supplied: String, canonical: String },

    #[error("bytecode emitter received duplicate MIR module `{module_path}`")]
    DuplicateMirModule { module_path: String },

    #[error("bytecode emitter received duplicate function key `{function_key}`")]
    DuplicateFunctionKey { function_key: String },

    #[error("bytecode emitter received duplicate frozen-constant bundle for `{module_path}`")]
    DuplicateConstantBundle { module_path: String },

    #[error("bytecode emitter has no frozen-constant bundle for MIR module `{module_path}`")]
    MissingConstantBundle { module_path: String },

    #[error("bytecode emitter received frozen-constant bundle for unknown module `{module_path}`")]
    UnexpectedConstantBundle { module_path: String },

    #[error("bytecode emitter has no frozen graph for constant `{symbol}`")]
    MissingConstantGraph { symbol: String },

    #[error("bytecode emitter received an unowned frozen graph for constant `{symbol}`")]
    UnexpectedConstantGraph { symbol: String },

    #[error("bytecode emitter received an empty frozen graph for constant `{symbol}`")]
    EmptyConstantGraph { symbol: String },

    #[error(
        "bytecode emitter function symbol `{symbol}` is not an exact member of module `{module_path}`"
    )]
    InvalidFunctionSymbol { module_path: String, symbol: String },

    #[error(
        "bytecode emitter parameter `{parameter}` in function `{function_key}` declares slot {slot} with a type different from the slot type"
    )]
    ParameterSlotTypeMismatch {
        function_key: String,
        parameter: String,
        slot: u32,
    },

    #[error(
        "bytecode emitter function `{function_key}` binds more than one parameter to slot {slot}"
    )]
    DuplicateParameterSlot { function_key: String, slot: u32 },

    #[error(
        "bytecode emitter function `{function_key}` has a stale or incomplete MIR liveness table"
    )]
    LivenessMismatch { function_key: String },

    #[error(
        "bytecode emitter function `{function_key}` statement side table diverges at flattened statement {position}"
    )]
    StatementTableMismatch {
        function_key: String,
        position: usize,
    },

    #[error(
        "bytecode emitter type at {location} references missing local type {type_index} in module `{module_path}` (type count {type_count})"
    )]
    MissingLocalType {
        module_path: String,
        location: String,
        type_index: u32,
        type_count: usize,
    },

    #[error("bytecode emitter constant graph `{symbol}` is invalid: {message}")]
    InvalidConstantGraph { symbol: String, message: String },

    #[error(
        "bytecode emitter constant graph `{symbol}` record node {node_index} has {child_count} children but shape {shape_index} declares {field_count} fields"
    )]
    ConstantShapeArityMismatch {
        symbol: String,
        node_index: u32,
        shape_index: u32,
        child_count: usize,
        field_count: u32,
    },

    #[error(
        "bytecode emitter cannot encode constant graph `{symbol}` node {node_index} ({construct}): {reason}"
    )]
    UnsupportedConstantNode {
        symbol: String,
        node_index: u32,
        construct: &'static str,
        reason: &'static str,
    },

    #[error(
        "bytecode emitter constant graph `{symbol}` references unknown behavior function `{function_key}`"
    )]
    UnknownBehaviorFunction {
        symbol: String,
        function_key: String,
    },

    #[error("bytecode emitter canonical serialization failed at {context}: {message}")]
    CanonicalSerialization { context: String, message: String },

    #[error("bytecode emitter has no explicit value-transfer plans for function `{function_key}`")]
    MissingValueTransferPlans { function_key: String },

    #[error(
        "bytecode emitter received value-transfer plans for unknown function `{function_key}`"
    )]
    UnexpectedValueTransferPlans { function_key: String },

    #[error(
        "bytecode emitter function `{function_key}` has {slot_count} slots but {plan_count} slot transfer plans"
    )]
    SlotPlanCountMismatch {
        function_key: String,
        slot_count: usize,
        plan_count: usize,
    },

    #[error(
        "bytecode emitter function `{function_key}` has {result_count} results but {plan_count} result transfer plans"
    )]
    ResultPlanCountMismatch {
        function_key: String,
        result_count: usize,
        plan_count: usize,
    },

    #[error(
        "bytecode emission for inout call in function `{function_key}` expression {expression} is pending the writable-region ISA"
    )]
    InOutEmissionPending {
        function_key: String,
        expression: u32,
    },

    #[error(
        "bytecode emitter does not support {construct} in function `{function_key}`{location}"
    )]
    UnsupportedConstruct {
        function_key: String,
        construct: &'static str,
        location: String,
    },

    #[error("bytecode emitter arithmetic overflow while {context}")]
    ArithmeticOverflow { context: &'static str },

    #[error("bytecode emitter limit `{limit}` exceeded at {location}: {actual} > {max}")]
    LimitExceeded {
        limit: &'static str,
        location: String,
        actual: u64,
        max: u64,
    },

    #[error("bytecode emitter MIR contract failed: {0}")]
    MirContract(#[from] MirContractError),

    #[error("bytecode emitter frozen-constant lookup failed: {0}")]
    FrozenConstantLookup(#[from] FrozenConstantLookupError),

    #[error("bytecode instruction encoding failed: {0}")]
    Encoding(#[from] EncodeError),

    #[error("emitted bytecode failed structural validation: {0}")]
    StructuralValidation(#[from] StructuralValidationError),

    #[error("emitted bytecode identity assignment failed: {0}")]
    ArtifactIdentity(#[from] skiff_artifact_identity::ArtifactIdentityError),
}

impl BytecodeEmissionError {
    pub(crate) fn unsupported_function(
        function_key: impl Into<String>,
        construct: &'static str,
    ) -> Self {
        Self::UnsupportedConstruct {
            function_key: function_key.into(),
            construct,
            location: String::new(),
        }
    }

    pub(crate) fn unsupported_statement(
        function_key: impl Into<String>,
        statement: u32,
        construct: &'static str,
    ) -> Self {
        Self::UnsupportedConstruct {
            function_key: function_key.into(),
            construct,
            location: format!(" at statement {statement}"),
        }
    }

    pub(crate) fn unsupported_expression(
        function_key: impl Into<String>,
        expression: u32,
        construct: &'static str,
    ) -> Self {
        Self::UnsupportedConstruct {
            function_key: function_key.into(),
            construct,
            location: format!(" at expression {expression}"),
        }
    }
}
