use skiff_artifact_model::{bytecode::EncodeError, StructuralValidationError};
use skiff_compiler_lowering::mir::MirContractError;
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
