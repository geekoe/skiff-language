use skiff_artifact_model::{ExprIr, InstructionSourceSite, StatementAttributionId};

use super::{direct_call_indices, function, lower_sources};
use crate::mir::{MirEmissionAnchor, MirFunction};

fn assert_direct_call_group(function: &MirFunction, expression_index: u32, tail: bool) -> usize {
    let events = function
        .source_event_plan
        .events()
        .expect("fully represented fixture has an available event plan");
    let mut group = events
        .iter()
        .filter(|event| {
            matches!(
                event.attribution_id,
                StatementAttributionId::Expression {
                    expression_index: candidate,
                    ..
                } if candidate == expression_index
            )
        })
        .collect::<Vec<_>>();
    group.sort_by_key(|event| match event.attribution_id {
        StatementAttributionId::Expression {
            occurrence_ordinal, ..
        } => occurrence_ordinal,
        _ => unreachable!("group contains only expression attributions"),
    });

    assert!(!group.is_empty(), "direct call has a source event group");
    let ExprIr::Call { call } = &function.expressions[expression_index as usize].expression else {
        unreachable!("direct call group references a call expression")
    };
    assert_eq!(&group[0].site, &call.site);
    for (expected_occurrence, event) in group.iter().enumerate() {
        let expected_occurrence = u32::try_from(expected_occurrence).unwrap();
        assert_eq!(
            event.attribution_id,
            StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal: expected_occurrence,
            }
        );
        assert!(matches!(&event.site, InstructionSourceSite::Source { .. }));
        if expected_occurrence == 0 {
            if tail {
                assert!(matches!(
                    event.anchor,
                    MirEmissionAnchor::TailLocalCallCandidate {
                        expression_index: candidate,
                        occurrence_ordinal: 0,
                        ..
                    } if candidate == expression_index
                ));
            } else {
                assert_eq!(
                    event.anchor,
                    MirEmissionAnchor::LocalCall {
                        expression_index,
                        occurrence_ordinal: 0,
                    }
                );
            }
        } else {
            assert_eq!(
                event.anchor,
                MirEmissionAnchor::Expression {
                    expression_index,
                    occurrence_ordinal: expected_occurrence,
                }
            );
        }
    }
    assert_eq!(
        group
            .iter()
            .filter(|event| {
                matches!(
                    event.anchor,
                    MirEmissionAnchor::LocalCall { .. }
                        | MirEmissionAnchor::TailLocalCallCandidate { .. }
                )
            })
            .count(),
        1,
        "one final direct call has exactly one specialized local-call anchor"
    );
    group.len()
}

#[test]
fn nested_local_call_callee_keys_stay_in_their_own_dense_groups() {
    let module = "internal.call_groups";
    let lowered = lower_sources(&[(
        "internal/call_groups.skiff",
        module,
        r#"
          function callee(value: number) -> number {
            return value
          }

          function caller(value: number) -> number {
            return callee(callee(value))
          }
        "#,
    )]);
    let caller = function(&lowered, module, "caller");
    let calls = direct_call_indices(caller);
    assert_eq!(calls.len(), 2);
    assert_eq!(assert_direct_call_group(caller, calls[0], false), 2);
    assert_eq!(assert_direct_call_group(caller, calls[1], true), 2);
}

#[test]
fn generic_publication_path_collapses_to_the_final_call_group() {
    let lowered = lower_sources(&[
        (
            "internal/worker.skiff",
            "internal.worker",
            r#"
              function identity<T>(value: T) -> T {
                return value
              }
            "#,
        ),
        (
            "internal/runner.skiff",
            "internal.runner",
            r#"
              function run() -> string {
                return root.internal.worker.identity<string>("ok")
              }
            "#,
        ),
    ]);
    let run = function(&lowered, "internal.runner", "run");
    let calls = direct_call_indices(run);
    let [call] = calls.as_slice() else {
        panic!("runner has one direct publication call")
    };
    assert!(assert_direct_call_group(run, *call, true) > 2);
}

#[test]
fn receiver_callee_field_collapses_without_absorbing_the_receiver_value() {
    let module = "internal.receiver_group";
    let lowered = lower_sources(&[(
        "internal/receiver_group.skiff",
        module,
        r#"
          type Box<T> { value: T }

          impl Box<T> {
            function unwrap() -> T {
              return self.value
            }
          }

          function caller(box: Box<string>) -> string {
            return box.unwrap()
          }
        "#,
    )]);
    let caller = function(&lowered, module, "caller");
    let calls = direct_call_indices(caller);
    let [call] = calls.as_slice() else {
        panic!("caller has one direct receiver call")
    };
    assert_eq!(assert_direct_call_group(caller, *call, true), 2);

    let receiver_index = caller
        .expressions
        .iter()
        .find(|expression| matches!(&expression.expression, ExprIr::LoadSlot { .. }))
        .expect("receiver lowers to its own expression")
        .index;
    let receiver_events = caller
        .source_event_plan
        .events()
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(
                event.attribution_id,
                StatementAttributionId::Expression {
                    expression_index,
                    occurrence_ordinal: 0,
                } if expression_index == receiver_index
            )
        })
        .count();
    assert_eq!(receiver_events, 1);
}
