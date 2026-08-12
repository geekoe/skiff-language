use skiff_artifact_model::{
    InstructionSourceSite, StatementAttributionId, StatementChargeKind,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use super::{charge_event_range, charge_frame_entry_kind, SourceEventView};
use crate::{
    frame::VmFrame, VmBudget, VmBudgetClosed, VmBudgetTerminal, VmSemanticCharge,
    VmSemanticChargeKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum BudgetAction {
    Poll,
    Entry {
        function: FunctionIndex,
        instruction: InstructionIndex,
    },
    EntryRejected,
    Event {
        function: FunctionIndex,
        instruction: InstructionIndex,
        sequence: u32,
        attribution_id: StatementAttributionId,
        site: InstructionSourceSite,
        kind: StatementChargeKind,
    },
    EventRejected {
        sequence: u32,
        kind: StatementChargeKind,
    },
}

#[derive(Default)]
struct RecordingBudget {
    actions: Vec<BudgetAction>,
    fail_poll: bool,
    fail_charge_at: Option<usize>,
    charge_calls: usize,
}

impl VmBudget for RecordingBudget {
    fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
        unreachable!("statement coordinator does not dispatch")
    }

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
        self.actions.push(BudgetAction::Poll);
        if self.fail_poll {
            Err(VmBudgetClosed::AlreadySettled(VmBudgetTerminal::Cancelled))
        } else {
            Ok(())
        }
    }

    fn charge_semantic(&mut self, charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
        let call = self.charge_calls;
        self.charge_calls += 1;
        if self.fail_charge_at == Some(call) {
            self.actions.push(match charge.kind() {
                VmSemanticChargeKind::FunctionEntry => BudgetAction::EntryRejected,
                VmSemanticChargeKind::SourceEvent {
                    sequence_ordinal,
                    charge_kind,
                    ..
                } => BudgetAction::EventRejected {
                    sequence: sequence_ordinal,
                    kind: charge_kind,
                },
            });
            return Err(VmBudgetClosed::AccountingFailure);
        }
        let action = match charge.kind() {
            VmSemanticChargeKind::FunctionEntry => BudgetAction::Entry {
                function: charge.function(),
                instruction: charge.instruction(),
            },
            VmSemanticChargeKind::SourceEvent {
                sequence_ordinal,
                attribution_id,
                site,
                charge_kind,
            } => BudgetAction::Event {
                function: charge.function(),
                instruction: charge.instruction(),
                sequence: sequence_ordinal,
                attribution_id,
                site: site.clone(),
                kind: charge_kind,
            },
        };
        self.actions.push(action);
        Ok(())
    }
}

#[test]
fn frame_entry_is_rowless_and_charged_once() {
    let mut frame = frame();
    let mut budget = RecordingBudget::default();

    charge_frame_entry_kind(&mut frame, StatementChargeKind::FunctionEntry, &mut budget).unwrap();
    charge_frame_entry_kind(&mut frame, StatementChargeKind::FunctionEntry, &mut budget).unwrap();

    assert_eq!(
        budget.actions,
        [BudgetAction::Entry {
            function: FunctionIndex::new(0),
            instruction: InstructionIndex::new(0),
        }]
    );
    assert!(!frame.function_entry_pending());
}

#[test]
fn frame_entry_failure_keeps_entry_pending() {
    let mut frame = frame();
    let mut budget = RecordingBudget {
        fail_charge_at: Some(0),
        ..RecordingBudget::default()
    };

    let error =
        charge_frame_entry_kind(&mut frame, StatementChargeKind::FunctionEntry, &mut budget)
            .unwrap_err();

    assert_eq!(error, VmBudgetClosed::AccountingFailure.into());
    assert!(frame.function_entry_pending());
    assert_eq!(budget.actions, [BudgetAction::EntryRejected]);
}

#[test]
fn frame_entry_rejects_a_source_event_charge_kind() {
    let mut frame = frame();
    let mut budget = RecordingBudget::default();

    let error = charge_frame_entry_kind(&mut frame, StatementChargeKind::Statement, &mut budget)
        .unwrap_err();

    assert!(matches!(
        error,
        crate::VmError::VerifiedEntryInvariant {
            invariant: crate::VmVerifiedInvariant::StatementScheduleFrameEntryKind,
        }
    ));
    assert!(frame.function_entry_pending());
    assert!(budget.actions.is_empty());
}

#[test]
fn source_events_keep_verified_sequence_and_are_not_replayed() {
    let mut frame = frame();
    let site = synthetic_site();
    let events = [
        event(0, StatementChargeKind::Statement, &site),
        event(1, StatementChargeKind::LocalCall, &site),
        event(2, StatementChargeKind::TailHop, &site),
    ];
    let mut budget = RecordingBudget::default();
    let instruction = frame.instruction();

    charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap();
    charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap();

    assert_eq!(
        budget.actions,
        [
            BudgetAction::Event {
                function: FunctionIndex::new(0),
                instruction,
                sequence: 0,
                attribution_id: StatementAttributionId::Statement {
                    statement_index: 0,
                    occurrence_ordinal: 0,
                },
                site: site.clone(),
                kind: StatementChargeKind::Statement,
            },
            BudgetAction::Event {
                function: FunctionIndex::new(0),
                instruction,
                sequence: 1,
                attribution_id: StatementAttributionId::Expression {
                    expression_index: 1,
                    occurrence_ordinal: 0,
                },
                site: site.clone(),
                kind: StatementChargeKind::LocalCall,
            },
            BudgetAction::Event {
                function: FunctionIndex::new(0),
                instruction,
                sequence: 2,
                attribution_id: StatementAttributionId::Expression {
                    expression_index: 2,
                    occurrence_ordinal: 0,
                },
                site,
                kind: StatementChargeKind::TailHop,
            },
        ]
    );
    assert!(!frame.statement_events_pending());
}

#[test]
fn retry_starts_at_the_first_uncommitted_event() {
    let mut frame = frame();
    let site = synthetic_site();
    let instruction = frame.instruction();
    let events = [
        event(0, StatementChargeKind::Statement, &site),
        event(1, StatementChargeKind::LocalCall, &site),
        event(2, StatementChargeKind::TailHop, &site),
    ];
    let mut budget = RecordingBudget {
        fail_charge_at: Some(1),
        ..RecordingBudget::default()
    };

    let error = charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap_err();

    assert_eq!(error, VmBudgetClosed::AccountingFailure.into());
    assert_eq!(frame.instruction(), instruction);
    assert_eq!(frame.next_statement_event_index(), 1);
    assert!(frame.statement_events_pending());

    budget.fail_charge_at = None;
    charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap();

    assert!(matches!(
        budget.actions.as_slice(),
        [
            BudgetAction::Event { sequence: 0, .. },
            BudgetAction::EventRejected {
                sequence: 1,
                kind: StatementChargeKind::LocalCall,
            },
            BudgetAction::Event { sequence: 1, .. },
            BudgetAction::Event { sequence: 2, .. },
        ]
    ));
    assert_eq!(frame.next_statement_event_index(), 3);
    assert!(!frame.statement_events_pending());
}

#[test]
fn loop_check_polls_then_charges_exactly_once() {
    let mut frame = frame();
    let site = synthetic_site();
    let instruction = frame.instruction();
    let mut budget = RecordingBudget::default();

    charge_event_range(
        &mut frame,
        &[event(0, StatementChargeKind::LoopCheck, &site)][..],
        &mut budget,
    )
    .unwrap();

    assert_eq!(
        budget.actions,
        [
            BudgetAction::Poll,
            BudgetAction::Event {
                function: FunctionIndex::new(0),
                instruction,
                sequence: 0,
                attribution_id: StatementAttributionId::Generated { ordinal: 0 },
                site,
                kind: StatementChargeKind::LoopCheck,
            },
        ]
    );
}

#[test]
fn failed_loop_charge_retries_poll_and_the_same_event() {
    let mut frame = frame();
    let site = synthetic_site();
    let events = [event(0, StatementChargeKind::LoopCheck, &site)];
    let mut budget = RecordingBudget {
        fail_charge_at: Some(0),
        ..RecordingBudget::default()
    };

    let error = charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap_err();

    assert_eq!(error, VmBudgetClosed::AccountingFailure.into());
    assert_eq!(frame.next_statement_event_index(), 0);
    assert!(frame.statement_events_pending());

    budget.fail_charge_at = None;
    charge_event_range(&mut frame, events.as_slice(), &mut budget).unwrap();

    assert!(matches!(
        budget.actions.as_slice(),
        [
            BudgetAction::Poll,
            BudgetAction::EventRejected {
                sequence: 0,
                kind: StatementChargeKind::LoopCheck,
            },
            BudgetAction::Poll,
            BudgetAction::Event {
                sequence: 0,
                kind: StatementChargeKind::LoopCheck,
                ..
            },
        ]
    ));
    assert_eq!(frame.next_statement_event_index(), 1);
    assert!(!frame.statement_events_pending());
}

#[test]
fn event_failure_does_not_commit_pc_or_event_progress() {
    let mut frame = frame();
    let site = synthetic_site();
    let instruction = frame.instruction();
    let mut budget = RecordingBudget {
        fail_charge_at: Some(0),
        ..RecordingBudget::default()
    };

    let error = charge_event_range(
        &mut frame,
        &[event(0, StatementChargeKind::LocalCall, &site)][..],
        &mut budget,
    )
    .unwrap_err();

    assert_eq!(error, VmBudgetClosed::AccountingFailure.into());
    assert_eq!(frame.instruction(), instruction);
    assert!(frame.statement_events_pending());
    assert_eq!(frame.next_statement_event_index(), 0);
    assert_eq!(
        budget.actions,
        [BudgetAction::EventRejected {
            sequence: 0,
            kind: StatementChargeKind::LocalCall,
        }]
    );
}

#[test]
fn loop_poll_failure_happens_before_charge_or_pc_commit() {
    let mut frame = frame();
    let site = synthetic_site();
    let instruction = frame.instruction();
    let mut budget = RecordingBudget {
        fail_poll: true,
        ..RecordingBudget::default()
    };

    let error = charge_event_range(
        &mut frame,
        &[event(0, StatementChargeKind::LoopCheck, &site)][..],
        &mut budget,
    )
    .unwrap_err();

    assert_eq!(
        error,
        VmBudgetClosed::AlreadySettled(VmBudgetTerminal::Cancelled).into()
    );
    assert_eq!(budget.actions, [BudgetAction::Poll]);
    assert_eq!(frame.instruction(), instruction);
    assert!(frame.statement_events_pending());
}

#[test]
fn function_entry_cannot_arrive_as_a_source_event() {
    let mut frame = frame();
    let site = synthetic_site();
    let mut budget = RecordingBudget::default();

    let error = charge_event_range(
        &mut frame,
        &[event(0, StatementChargeKind::FunctionEntry, &site)][..],
        &mut budget,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        crate::VmError::VerifiedEntryInvariant {
            invariant: crate::VmVerifiedInvariant::StatementScheduleEventKind,
        }
    ));
    assert!(budget.actions.is_empty());
    assert!(frame.statement_events_pending());
}

fn frame() -> VmFrame {
    VmFrame::root(FunctionIndex::new(0), 0, 0)
}

fn event(
    sequence_ordinal: u32,
    charge_kind: StatementChargeKind,
    site: &InstructionSourceSite,
) -> SourceEventView<'_> {
    let attribution_id = match charge_kind {
        StatementChargeKind::Statement => StatementAttributionId::Statement {
            statement_index: sequence_ordinal,
            occurrence_ordinal: 0,
        },
        StatementChargeKind::Expression
        | StatementChargeKind::LocalCall
        | StatementChargeKind::TailHop => StatementAttributionId::Expression {
            expression_index: sequence_ordinal,
            occurrence_ordinal: 0,
        },
        StatementChargeKind::LoopCheck
        | StatementChargeKind::GeneratedChunk
        | StatementChargeKind::FunctionEntry => StatementAttributionId::Generated {
            ordinal: sequence_ordinal,
        },
    };
    SourceEventView {
        sequence_ordinal,
        attribution_id,
        site,
        charge_kind,
    }
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}
