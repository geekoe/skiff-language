use crate::StatementAttributionClass;

/// Budget/profiling charge kind distilled from authenticated source events or
/// immutable execution contracts. This enum is not persisted in statement
/// rows in bytecode schema v6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementChargeKind {
    FunctionEntry,
    Statement,
    Expression,
    LocalCall,
    TailHop,
    LoopCheck,
    GeneratedChunk,
}

impl StatementChargeKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::FunctionEntry => "functionEntry",
            Self::Statement => "statement",
            Self::Expression => "expression",
            Self::LocalCall => "localCall",
            Self::TailHop => "tailHop",
            Self::LoopCheck => "loopCheck",
            Self::GeneratedChunk => "generatedChunk",
        }
    }
}

/// Canonical default charge for each authenticated source-event class.
/// Opcode-derived [`StatementContract::RequiredEvent`] rules may reclassify
/// exactly one matching event at that PC; they do not create another row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttributionChargeContract {
    pub statement: StatementChargeKind,
    pub expression: StatementChargeKind,
    pub generated: StatementChargeKind,
}

impl AttributionChargeContract {
    const fn charge_kind(self, attribution: StatementAttributionClass) -> StatementChargeKind {
        match attribution {
            StatementAttributionClass::Statement => self.statement,
            StatementAttributionClass::Expression => self.expression,
            StatementAttributionClass::Generated => self.generated,
        }
    }
}

pub const ATTRIBUTION_CHARGE_CONTRACT: AttributionChargeContract = AttributionChargeContract {
    statement: StatementChargeKind::Statement,
    expression: StatementChargeKind::Expression,
    generated: StatementChargeKind::GeneratedChunk,
};

/// Looks up the canonical default charge for one authenticated source event.
pub const fn default_statement_charge_kind_for_attribution(
    attribution: StatementAttributionClass,
) -> StatementChargeKind {
    ATTRIBUTION_CHARGE_CONTRACT.charge_kind(attribution)
}

/// Top-level frame rule. Function-entry charging is derived exactly once when
/// a frame is invoked and is never represented by a statement row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameEntryStatementContract {
    pub charge_kind: StatementChargeKind,
}

pub const FRAME_ENTRY_STATEMENT_CONTRACT: FrameEntryStatementContract =
    FrameEntryStatementContract {
        charge_kind: StatementChargeKind::FunctionEntry,
    };

/// Per-opcode statement charging rule. `RequiredEvent` selects exactly one
/// source event of the declared class at the instruction PC; the selected row
/// is reclassified to this charge kind and no additional row is synthesized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementContract {
    None,
    RequiredEvent {
        charge_kind: StatementChargeKind,
        attribution: StatementAttributionClass,
    },
}
