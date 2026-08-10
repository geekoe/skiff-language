use std::fmt;

use skiff_runtime_linked_bytecode::{CandidateTable, FunctionIndex, InstructionIndex};

/// Bounded resource whose configured ceiling was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationLimit {
    Functions,
    TotalInstructions,
    InstructionsPerFunction,
    FrameSlotsPerFunction,
    OperandDepth,
    ControlFlowEdgesPerFunction,
    ExceptionRegionsPerFunction,
    SwitchTargetsPerFunction,
    StatementEventsPerPc,
    StatementEventsPerFunction,
    TotalStatementEvents,
    SourceMapEntriesPerFunction,
    ImageTableEntries,
    Arity,
    CallbackCapturesPerCallback,
    TypeNestingDepth,
    ValueLifecycleNodes,
    ValueLifecycleCanonicalBytes,
    ConstantGraphEdges,
}

impl VerificationLimit {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Functions => "functions",
            Self::TotalInstructions => "total instructions",
            Self::InstructionsPerFunction => "instructions per function",
            Self::FrameSlotsPerFunction => "frame slots per function",
            Self::OperandDepth => "operand depth",
            Self::ControlFlowEdgesPerFunction => "control-flow edges per function",
            Self::ExceptionRegionsPerFunction => "exception regions per function",
            Self::SwitchTargetsPerFunction => "switch targets per function",
            Self::StatementEventsPerPc => "statement events per PC",
            Self::StatementEventsPerFunction => "statement events per function",
            Self::TotalStatementEvents => "total statement events",
            Self::SourceMapEntriesPerFunction => "source-map entries per function",
            Self::ImageTableEntries => "image table entries",
            Self::Arity => "arity",
            Self::CallbackCapturesPerCallback => "callback captures per callback",
            Self::TypeNestingDepth => "type nesting depth",
            Self::ValueLifecycleNodes => "value lifecycle nodes",
            Self::ValueLifecycleCanonicalBytes => "value lifecycle canonical bytes",
            Self::ConstantGraphEdges => "constant graph edges",
        }
    }
}

/// Independent proof family owned by the semantic verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationObligation {
    ExactHydrationBinding,
    ExactTargetAndCallPlan,
    ControlFlow,
    StackAndSlotState,
    ConcreteTypeAndShape,
    ConcreteSpecialization,
    InterfaceSignature,
    ExceptionRegion,
    ResumeSite,
    EffectAndNoPending,
    TailCall,
    ValueTransferAndDrop,
    CallbackCaptureAndEscape,
    SourceAndStatementAttribution,
    ResourceAccounting,
    BudgetCheckpoint,
    FrozenConstantSafety,
}

impl VerificationObligation {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ExactHydrationBinding => "exact hydration binding",
            Self::ExactTargetAndCallPlan => "exact target and call plan",
            Self::ControlFlow => "control flow",
            Self::StackAndSlotState => "stack and slot state",
            Self::ConcreteTypeAndShape => "concrete type and shape",
            Self::ConcreteSpecialization => "concrete specialization",
            Self::InterfaceSignature => "interface signature",
            Self::ExceptionRegion => "exception region",
            Self::ResumeSite => "resume site",
            Self::EffectAndNoPending => "effect and NoPending reachability",
            Self::TailCall => "tail call",
            Self::ValueTransferAndDrop => "value transfer and drop",
            Self::CallbackCaptureAndEscape => "callback capture and escape",
            Self::SourceAndStatementAttribution => "source and statement attribution",
            Self::ResourceAccounting => "resource accounting",
            Self::BudgetCheckpoint => "budget checkpoint",
            Self::FrozenConstantSafety => "frozen constant safety",
        }
    }
}

/// Stable location attached to a verification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationLocation {
    Image,
    Table {
        table: CandidateTable,
        row: u32,
    },
    Function {
        function: FunctionIndex,
    },
    Instruction {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
}

impl fmt::Display for VerificationLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image => formatter.write_str("linked image"),
            Self::Table { table, row } => {
                write!(formatter, "{} table row {row}", table.name())
            }
            Self::Function { function } => {
                write!(formatter, "function {}", function.get())
            }
            Self::Instruction {
                function,
                instruction,
            } => write!(
                formatter,
                "function {} instruction {}",
                function.get(),
                instruction.get()
            ),
        }
    }
}

/// Structured, fail-closed result of independent semantic verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationError {
    LimitExceeded {
        limit: VerificationLimit,
        actual: u64,
        max: u64,
        location: VerificationLocation,
    },
    SemanticViolation {
        obligation: VerificationObligation,
        location: VerificationLocation,
        detail: String,
    },
    /// The crate has not independently established a required proof.
    ///
    /// This is never a soft warning and must never publish an image. It makes
    /// interface-first development fail closed until the corresponding proof
    /// implementation lands.
    ProofUnavailable {
        obligation: VerificationObligation,
        location: VerificationLocation,
    },
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded {
                limit,
                actual,
                max,
                location,
            } => write!(
                formatter,
                "bytecode verification limit {} exceeded at {location}: actual {actual} > max {max}",
                limit.name()
            ),
            Self::SemanticViolation {
                obligation,
                location,
                detail,
            } => write!(
                formatter,
                "bytecode {} verification failed at {location}: {detail}",
                obligation.name()
            ),
            Self::ProofUnavailable {
                obligation,
                location,
            } => write!(
                formatter,
                "bytecode {} proof is unavailable at {location}",
                obligation.name()
            ),
        }
    }
}

impl std::error::Error for VerificationError {}
