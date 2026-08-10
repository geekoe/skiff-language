mod collector;
mod model;
mod statements;
mod validate;

#[cfg(test)]
mod tests;

pub(crate) use collector::{ExpressionEventKind, MirSourceEventCollector};
pub use model::{
    MirControlFlowEdge, MirEmissionAnchor, MirSourceEvent, MirSourceEventPlan,
    MirSourceEventPlanError, MirSourceEventUnavailableReason, MirStatementPlacement,
};

pub(crate) use validate::finalize_mir_source_event_plan;
