use skiff_artifact_model::{ExprIr, StatementAttributionId};

use super::{direct_call_indices, function, lower_sources};
use crate::mir::{MirEmissionAnchor, MirFunction};

#[test]
fn returned_dispatch_is_never_a_local_or_tail_call_anchor() {
    let module = "internal.returned_dispatch_group";
    let lowered = lower_sources(&[(
        "internal/returned_dispatch_group.skiff",
        module,
        r#"
          function run() -> void {
            return
          }

          function start() -> std.task.TaskRef {
            return dispatch run()
          }
        "#,
    )]);
    assert_dispatch_group(function(&lowered, module, "start"));
}

#[test]
fn statement_dispatch_outer_expression_maps_to_the_final_call() {
    let module = "internal.statement_dispatch_group";
    let lowered = lower_sources(&[(
        "internal/statement_dispatch_group.skiff",
        module,
        r#"
          function run() -> void {
            return
          }

          function start() -> void {
            dispatch run()
          }
        "#,
    )]);
    assert_dispatch_group(function(&lowered, module, "start"));
}

fn assert_dispatch_group(function: &MirFunction) {
    let calls = direct_call_indices(function);
    let [call_index] = calls.as_slice() else {
        panic!("fixture has one direct dispatch target")
    };
    let ExprIr::Call { call } = &function.expressions[*call_index as usize].expression else {
        unreachable!("direct call index references a call")
    };
    assert!(crate::task_call::is_task_submit_call(call));

    let events = function
        .source_event_plan
        .events()
        .expect("dispatch fixture is fully represented");
    let mut group = events
        .iter()
        .filter_map(|event| match event.attribution_id {
            StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            } if expression_index == *call_index => Some((occurrence_ordinal, event)),
            _ => None,
        })
        .collect::<Vec<_>>();
    group.sort_by_key(|(occurrence, _)| *occurrence);
    assert!(group.len() >= 3);
    for (expected, (occurrence, event)) in group.iter().enumerate() {
        let expected = u32::try_from(expected).unwrap();
        assert_eq!(*occurrence, expected);
        assert_eq!(
            event.anchor,
            MirEmissionAnchor::Expression {
                expression_index: *call_index,
                occurrence_ordinal: expected,
            }
        );
    }
}
