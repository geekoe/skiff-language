use skiff_artifact_model::{StatementAttributionId, StatementChargeKind};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use crate::{
    VerificationError, VerificationLimit, VerifiedStatementEvent, VerifiedStatementSchedule,
};

use super::fixtures::{
    generous_limits, loader_backed_local_call, LocalCallCandidateCorruption, TARGET_FUNCTION_INDEX,
};

#[test]
fn exact_rows_become_dense_reclassified_schedule_without_double_charge() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let schedule = crate::verifier::prove_statement_schedule_for_test(
        &hydrated,
        &candidate,
        &generous_limits(),
    )
    .expect("exact admitted statement rows must produce a schedule");

    assert_eq!(schedule.function_count(), 2);
    assert_eq!(schedule.total_event_count(), 3);
    assert_eq!(schedule.instruction_count(FunctionIndex::new(0)), Some(3));
    assert_eq!(schedule.instruction_count(TARGET_FUNCTION_INDEX), Some(1));
    assert_eq!(
        schedule.frame_entry_charge_kind(FunctionIndex::new(0)),
        Some(StatementChargeKind::FunctionEntry)
    );
    assert_eq!(
        schedule.frame_entry_charge_kind(TARGET_FUNCTION_INDEX),
        Some(StatementChargeKind::FunctionEntry)
    );

    let call = schedule
        .events_at(FunctionIndex::new(0), InstructionIndex::new(0))
        .unwrap();
    assert_eq!(call.len(), 2);
    assert_eq!(call[0].sequence_ordinal(), 0);
    assert_eq!(call[0].charge_kind(), StatementChargeKind::Statement);
    assert!(matches!(
        call[0].attribution_id(),
        StatementAttributionId::Statement { .. }
    ));
    assert_eq!(call[1].sequence_ordinal(), 1);
    assert_eq!(call[1].charge_kind(), StatementChargeKind::LocalCall);
    assert!(matches!(
        call[1].attribution_id(),
        StatementAttributionId::Expression { .. }
    ));

    let budget = schedule
        .events_at(FunctionIndex::new(0), InstructionIndex::new(1))
        .unwrap();
    assert_eq!(budget.len(), 1);
    assert_eq!(budget[0].charge_kind(), StatementChargeKind::LoopCheck);
    assert!(matches!(
        budget[0].attribution_id(),
        StatementAttributionId::Generated { ordinal: 0 }
    ));
    assert!(schedule
        .events_at(FunctionIndex::new(0), InstructionIndex::new(2))
        .unwrap()
        .is_empty());
    assert!(schedule
        .events_at(TARGET_FUNCTION_INDEX, InstructionIndex::new(0))
        .unwrap()
        .is_empty());
    assert_eq!(
        schedule
            .events_for_function(FunctionIndex::new(0))
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn statement_and_source_limits_fail_before_schedule_allocation() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let cases = [
        (VerificationLimit::StatementEventsPerPc, 1_u64),
        (VerificationLimit::StatementEventsPerFunction, 2),
        (VerificationLimit::TotalStatementEvents, 2),
        (VerificationLimit::SourceMapEntriesPerFunction, 1),
    ];
    for (expected, max) in cases {
        let mut limits = generous_limits();
        match expected {
            VerificationLimit::StatementEventsPerPc => {
                limits.max_statement_events_per_pc = max;
            }
            VerificationLimit::StatementEventsPerFunction => {
                limits.max_statement_events_per_function = max;
            }
            VerificationLimit::TotalStatementEvents => {
                limits.max_total_statement_events = max;
            }
            VerificationLimit::SourceMapEntriesPerFunction => {
                limits.max_source_map_entries_per_function = max;
            }
            _ => unreachable!(),
        }
        let error =
            crate::verifier::prove_statement_schedule_for_test(&hydrated, &candidate, &limits)
                .expect_err("an attribution ceiling below exact input must fail closed");
        assert!(matches!(
            error,
            VerificationError::LimitExceeded { limit, actual, max: actual_max, .. }
                if limit == expected && actual > actual_max && actual_max == max
        ));
    }
}

#[test]
fn schedule_surface_is_read_only_index_and_range_access() {
    let events_at: for<'a> fn(
        &'a VerifiedStatementSchedule,
        FunctionIndex,
        InstructionIndex,
    ) -> Option<&'a [VerifiedStatementEvent]> = VerifiedStatementSchedule::events_at;
    let all_events: for<'a> fn(
        &'a VerifiedStatementSchedule,
        FunctionIndex,
    ) -> Option<&'a [VerifiedStatementEvent]> = VerifiedStatementSchedule::events_for_function;

    let _ = (events_at, all_events);
}
