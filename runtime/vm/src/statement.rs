use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId, StatementChargeKind};
use skiff_runtime_linker::{ExecutionStatementEvent, ExecutionStatementSchedule};

use crate::{frame::VmFrame, VmBudget, VmError, VmSemanticCharge, VmVerifiedInvariant};

pub(crate) fn charge_frame_entry(
    schedule: &ExecutionStatementSchedule,
    frame: &mut VmFrame,
    budget: &mut dyn VmBudget,
) -> Result<(), VmError> {
    if !frame.function_entry_pending() {
        return Ok(());
    }
    let function = frame.function();
    let kind =
        schedule
            .frame_entry_charge_kind(function)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::StatementScheduleFunctionMissing { function },
            })?;
    charge_frame_entry_kind(frame, kind, budget)
}

pub(crate) fn charge_instruction_events(
    schedule: &ExecutionStatementSchedule,
    frame: &mut VmFrame,
    budget: &mut dyn VmBudget,
) -> Result<(), VmError> {
    if !frame.statement_events_pending() {
        return Ok(());
    }
    let function = frame.function();
    let instruction = frame.instruction();
    let events =
        schedule
            .events_at(function, instruction)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::StatementScheduleInstructionMissing {
                    function,
                    instruction,
                },
            })?;
    charge_event_range(frame, events, budget)
}

fn charge_frame_entry_kind(
    frame: &mut VmFrame,
    kind: StatementChargeKind,
    budget: &mut dyn VmBudget,
) -> Result<(), VmError> {
    if !frame.function_entry_pending() {
        return Ok(());
    }
    if kind != StatementChargeKind::FunctionEntry {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::StatementScheduleFrameEntryKind,
        });
    }
    budget.charge_semantic(VmSemanticCharge::function_entry(
        frame.function(),
        frame.instruction(),
    ))?;
    frame.mark_function_entry_charged();
    Ok(())
}

fn charge_event_range<R>(
    frame: &mut VmFrame,
    events: &R,
    budget: &mut dyn VmBudget,
) -> Result<(), VmError>
where
    R: SourceEventRange + ?Sized,
{
    if !frame.statement_events_pending() {
        return Ok(());
    }
    if frame.next_statement_event_index() > events.event_count() {
        return Err(VmError::VerifiedEntryInvariant {
            invariant: VmVerifiedInvariant::StatementScheduleEventCursor,
        });
    }
    while frame.next_statement_event_index() < events.event_count() {
        let event = events.view_at(frame.next_statement_event_index()).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::StatementScheduleEventCursor,
            },
        )?;
        if event.charge_kind == StatementChargeKind::FunctionEntry {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::StatementScheduleEventKind,
            });
        }
        if event.charge_kind == StatementChargeKind::LoopCheck {
            budget.poll_interrupt()?;
        }
        budget.charge_semantic(VmSemanticCharge::source_event(
            frame.function(),
            frame.instruction(),
            event.sequence_ordinal,
            event.attribution_id,
            event.site,
            event.charge_kind,
        ))?;
        if !frame.advance_statement_event() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::StatementScheduleEventCursor,
            });
        }
    }
    frame.mark_statement_events_complete();
    Ok(())
}

trait SourceEventRange {
    fn event_count(&self) -> usize;

    fn view_at(&self, index: usize) -> Option<SourceEventView<'_>>;
}

impl SourceEventRange for [ExecutionStatementEvent] {
    fn event_count(&self) -> usize {
        <[ExecutionStatementEvent]>::len(self)
    }

    fn view_at(&self, index: usize) -> Option<SourceEventView<'_>> {
        self.get(index).map(SourceEventView::from)
    }
}

#[derive(Clone, Copy)]
struct SourceEventView<'a> {
    sequence_ordinal: u32,
    attribution_id: StatementAttributionId,
    site: &'a InstructionSourceSite,
    charge_kind: StatementChargeKind,
}

impl<'a> From<&'a ExecutionStatementEvent> for SourceEventView<'a> {
    fn from(event: &'a ExecutionStatementEvent) -> Self {
        Self {
            sequence_ordinal: event.sequence_ordinal(),
            attribution_id: event.attribution_id(),
            site: event.site(),
            charge_kind: event.charge_kind(),
        }
    }
}

#[cfg(test)]
impl<'a> SourceEventRange for [SourceEventView<'a>] {
    fn event_count(&self) -> usize {
        <[SourceEventView<'a>]>::len(self)
    }

    fn view_at(&self, index: usize) -> Option<SourceEventView<'_>> {
        self.get(index).copied()
    }
}

#[cfg(test)]
mod tests;
