use std::sync::{Arc, Mutex};

use skiff_runtime_model::request_heap::RequestHeap;

use super::super::{
    concurrent_scheduler::run_concurrent_scheduler, concurrent_scheduler_test_support::*,
    LaneCompletion,
};

#[tokio::test]
async fn concurrent_scheduler_lane_controls_share_budget_but_install_distinct_scopes() {
    let lane_cancel_flags = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let lane_cancel_flags = lane_cancel_flags.clone();
        move |lane, state| {
            let control = state.execution_control();
            control
                .borrow()
                .add_instruction_units((lane.source_order() + 1) as u64)
                .unwrap();
            lane_cancel_flags
                .lock()
                .unwrap()
                .push(Arc::as_ptr(&control.borrow().cancel_flag()) as usize);
            boxed_lane(async move { LaneCompletion::normal(state) })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
    ]);
    let outer = TestOuter::new();
    let parent_cancel_flag = outer.parent_cancel_flag_address();
    let mut parent_heap = RequestHeap::default();

    run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .unwrap();

    let mut lane_cancel_flags = lane_cancel_flags.lock().unwrap().clone();
    lane_cancel_flags.sort_unstable();
    lane_cancel_flags.dedup();
    assert_eq!(outer.instruction_units(), 3);
    assert_eq!(lane_cancel_flags.len(), 2);
    assert!(lane_cancel_flags
        .iter()
        .all(|address| *address != parent_cancel_flag));
    assert_clean_scope(&outer);
}
