use std::fmt;

use skiff_artifact_model::{
    InstructionSourceSite, StatementAttributionClass, StatementAttributionId,
};

/// Exact statement placement understood by the current emitter seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirStatementPlacement {
    BeforeStatement,
}

/// One exact edge in a function-owned MIR CFG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirControlFlowEdge {
    from_block: u32,
    to_block: u32,
}

impl MirControlFlowEdge {
    pub const fn from_block(self) -> u32 {
        self.from_block
    }

    pub const fn to_block(self) -> u32 {
        self.to_block
    }

    pub(crate) const fn new(from_block: u32, to_block: u32) -> Self {
        Self {
            from_block,
            to_block,
        }
    }
}

/// A finite, typed instruction-emission site. Every source index is a final
/// File IR/MIR table index; source preorder never crosses this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirEmissionAnchor {
    Statement {
        statement_index: u32,
        occurrence_ordinal: u32,
        placement: MirStatementPlacement,
    },
    Expression {
        expression_index: u32,
        occurrence_ordinal: u32,
    },
    LocalCall {
        expression_index: u32,
        occurrence_ordinal: u32,
    },
    TailLocalCallCandidate {
        statement_index: u32,
        expression_index: u32,
        occurrence_ordinal: u32,
    },
    BudgetCheckpoint {
        loop_statement_index: u32,
        edge: MirControlFlowEdge,
    },
    GeneratedStatement {
        statement_index: u32,
        placement: MirStatementPlacement,
    },
}

/// One checked MIR-to-emitter source event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirSourceEvent {
    pub attribution_id: StatementAttributionId,
    pub site: InstructionSourceSite,
    pub anchor: MirEmissionAnchor,
}

/// Why an executable deliberately exposes no event list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirSourceEventUnavailableReason {
    SourceFactsNotProvided,
    SourceOwnerNotProvided,
    SourceEventNotRepresentable { class: StatementAttributionClass },
    MirAnchorNotReachable { class: StatementAttributionClass },
}

/// Per-executable event plan. Available contents can only be constructed by
/// lowering's checked collector/finalizer. Unavailable is distinct from an
/// available zero-event function and must never be interpreted as empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirSourceEventPlan {
    contents: MirSourceEventPlanContents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MirSourceEventPlanContents {
    Available(Vec<MirSourceEvent>),
    Unavailable(MirSourceEventUnavailableReason),
}

impl MirSourceEventPlan {
    pub fn events(&self) -> Option<&[MirSourceEvent]> {
        match &self.contents {
            MirSourceEventPlanContents::Available(events) => Some(events),
            MirSourceEventPlanContents::Unavailable(_) => None,
        }
    }

    pub fn unavailable_reason(&self) -> Option<MirSourceEventUnavailableReason> {
        match &self.contents {
            MirSourceEventPlanContents::Available(_) => None,
            MirSourceEventPlanContents::Unavailable(reason) => Some(*reason),
        }
    }

    /// Safe public construction for fixtures that intentionally model an
    /// unavailable producer. It cannot create emitter-consumable contents.
    pub const fn unavailable(reason: MirSourceEventUnavailableReason) -> Self {
        Self {
            contents: MirSourceEventPlanContents::Unavailable(reason),
        }
    }

    pub(crate) fn checked_available(
        events: Vec<MirSourceEvent>,
    ) -> Result<Self, MirSourceEventPlanError> {
        super::validate::validate_canonical_events(&events)?;
        Ok(Self {
            contents: MirSourceEventPlanContents::Available(events),
        })
    }

    pub(crate) fn into_events(self) -> Option<Vec<MirSourceEvent>> {
        match self.contents {
            MirSourceEventPlanContents::Available(events) => Some(events),
            MirSourceEventPlanContents::Unavailable(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirSourceEventPlanError {
    message: String,
}

impl MirSourceEventPlanError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MirSourceEventPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MirSourceEventPlanError {}
