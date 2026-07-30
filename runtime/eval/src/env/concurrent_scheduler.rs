use std::{future::Future, pin::Pin};

use skiff_runtime_capability_context::OwnedExecutionControl;
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_carrier_between_heaps, RequestHeap},
    runtime_value::RuntimeValueCarrier,
};

use crate::{
    error::{Result, RuntimeError},
    program_execution::{ExecutionCheckpoint, ExecutionCheckpointKind, ProgramExecutionContext},
};

mod batch;

use super::{
    concurrent_plan::{ConcurrentPlan, ConcurrentPlanKind, LaneEvaluation, ProjectedLane},
    lane_state::{ConcurrentBaseline, LaneCompletion, LaneExecutionState, LaneExport},
    Env,
};
use batch::{
    cancel_evaluated, cancel_ready, cancel_running, evaluate_ready_lane, poll_ready_batch,
    RunningLane,
};

pub(crate) type ConcurrentLaneFuture<'a> =
    Pin<Box<dyn Future<Output = LaneCompletion> + Send + 'a>>;

pub(crate) trait ConcurrentLaneExecutor<'a> {
    fn start_lane(
        &'a self,
        lane: ProjectedLane,
        state: LaneExecutionState,
    ) -> ConcurrentLaneFuture<'a>;
}

pub(crate) trait ConcurrentOuterExecution {
    fn owned_execution_control(&self) -> OwnedExecutionControl;
    fn concurrent_checkpoint(&self, kind: ExecutionCheckpointKind) -> Result<()>;
}

#[derive(Debug)]
pub(crate) enum ConcurrentSchedulerResult {
    Statement,
    Value(RuntimeValueCarrier),
}

pub(crate) async fn run_concurrent_scheduler<'a, O, E>(
    plan: &ConcurrentPlan,
    parent_env: &Env,
    parent_heap: &mut RequestHeap,
    outer: &O,
    executor: &'a E,
) -> Result<ConcurrentSchedulerResult>
where
    O: ConcurrentOuterExecution,
    E: ConcurrentLaneExecutor<'a>,
{
    validate_projected_plan(plan)?;
    let baseline = ConcurrentBaseline::freeze(parent_env, parent_heap);
    let parent_control = outer.owned_execution_control();
    let parent_scope = parent_control.execution_scope().map_err(|error| {
        invalid_scheduler(format!("current execution scope is unavailable: {error}"))
    })?;
    let mut lanes = plan
        .lanes()
        .iter()
        .cloned()
        .map(LaneRecord::pending)
        .collect::<Vec<_>>();
    let mut running = Vec::new();

    loop {
        launch_ready_lanes(
            &mut lanes,
            &mut running,
            &baseline,
            &parent_control,
            &parent_scope,
            outer,
            executor,
        )?;

        if running.is_empty() {
            if lanes.iter().all(LaneRecord::is_normal) {
                return finish_plan(plan, &mut lanes, parent_heap);
            }
            return Err(invalid_scheduler(
                "DAG stalled with unfinished lanes and no running future",
            ));
        }

        let mut ready = poll_ready_batch(&mut running).await;
        ready.sort_by_key(|lane| lane.source_order);

        let mut outer_winner = None;
        for lane in &ready {
            if let Err(error) = outer.concurrent_checkpoint(ExecutionCheckpointKind::LaneEnd) {
                outer_winner = Some(error);
                break;
            }
            debug_assert_eq!(
                lanes[lane.source_order].lane.source_order(),
                lane.source_order
            );
        }
        if let Some(error) = outer_winner {
            cancel_ready(&mut ready);
            cancel_running(&mut running);
            return Err(error);
        }
        if ready.iter().any(|lane| !lane.has_lane_result()) {
            cancel_ready(&mut ready);
            cancel_running(&mut running);
            return Err(invalid_scheduler(
                "lane scope terminated without an outer owner-aware terminal",
            ));
        }

        let mut evaluated = ready
            .drain(..)
            .map(|lane| evaluate_ready_lane(lane, &lanes))
            .collect::<Vec<_>>();
        if let Some(winner_index) = evaluated.iter().position(|lane| lane.error.is_some()) {
            let winner_order = evaluated[winner_index].source_order;
            debug_assert!(evaluated
                .iter()
                .filter_map(|lane| lane.error.as_ref().map(|_| lane.source_order))
                .all(|source_order| source_order >= winner_order));
            cancel_evaluated(&mut evaluated);
            cancel_running(&mut running);
            let winner = evaluated.swap_remove(winner_index);
            return Err(winner.materialize_error(parent_heap));
        }

        for lane in &mut evaluated {
            if !lane.complete_lease() {
                cancel_evaluated(&mut evaluated);
                cancel_running(&mut running);
                return outer
                    .concurrent_checkpoint(ExecutionCheckpointKind::LaneEnd)
                    .and_then(|()| {
                        Err(invalid_scheduler(
                            "lane lease rejected normal completion without an outer terminal",
                        ))
                    });
            }
        }

        for lane in evaluated {
            lanes[lane.source_order].state = LaneRecordState::Normal(lane.success);
        }
    }
}

impl ConcurrentOuterExecution for ProgramExecutionContext<'_> {
    fn owned_execution_control(&self) -> OwnedExecutionControl {
        self.execution().owned()
    }

    fn concurrent_checkpoint(&self, kind: ExecutionCheckpointKind) -> Result<()> {
        self.checkpoint(ExecutionCheckpoint::new(kind, 1))
    }
}

struct LaneRecord {
    lane: ProjectedLane,
    state: LaneRecordState,
}

enum LaneRecordState {
    Pending,
    Running,
    Normal(LaneSuccess),
}

struct LaneSuccess {
    export: Option<LaneExport>,
    tail: Option<TailValue>,
}

struct TailValue {
    source_heap: RequestHeap,
    carrier: RuntimeValueCarrier,
}

impl LaneRecord {
    fn pending(lane: ProjectedLane) -> Self {
        Self {
            lane,
            state: LaneRecordState::Pending,
        }
    }

    fn is_normal(&self) -> bool {
        matches!(self.state, LaneRecordState::Normal(_))
    }
}

fn launch_ready_lanes<'a, O, E>(
    lanes: &mut [LaneRecord],
    running: &mut Vec<RunningLane<'a>>,
    baseline: &ConcurrentBaseline,
    parent_control: &OwnedExecutionControl,
    parent_scope: &skiff_runtime_capability_context::ExecutionScope,
    outer: &O,
    executor: &'a E,
) -> Result<()>
where
    O: ConcurrentOuterExecution,
    E: ConcurrentLaneExecutor<'a>,
{
    let ready_orders = lanes
        .iter()
        .filter(|record| matches!(record.state, LaneRecordState::Pending))
        .filter(|record| {
            record
                .lane
                .dependencies()
                .iter()
                .all(|dependency| lanes[*dependency].is_normal())
        })
        .map(|record| record.lane.source_order())
        .collect::<Vec<_>>();

    for source_order in ready_orders {
        let checkpoint = match lanes[source_order].lane.evaluation() {
            LaneEvaluation::Tail { .. } => ExecutionCheckpointKind::TailStart,
            _ => ExecutionCheckpointKind::LaneStart,
        };
        outer.concurrent_checkpoint(checkpoint)?;

        let (lease, completion) = parent_scope.acquire_lease();
        let child_scope = lease.child_execution_scope();
        let lane_cancellation = lease.child_cancellation_token();
        let waiter = Box::pin(lease.wait());
        let mut state = baseline.lane_state(parent_control.clone(), child_scope, lane_cancellation);
        for dependency in lanes[source_order].lane.dependencies() {
            let LaneRecordState::Normal(success) = &lanes[*dependency].state else {
                return Err(invalid_scheduler(format!(
                    "lane {source_order} started before dependency {dependency} completed"
                )));
            };
            if let Some(export) = &success.export {
                if export.source_order != *dependency {
                    return Err(invalid_scheduler(format!(
                        "lane {source_order} dependency {dependency} resolved to export owner {}",
                        export.source_order
                    )));
                }
                state.import_export(export)?;
            }
        }

        let lane = lanes[source_order].lane.clone();
        let future = executor.start_lane(lane, state);
        lanes[source_order].state = LaneRecordState::Running;
        running.push(RunningLane {
            source_order,
            waiter: Some(waiter),
            completion: Some(completion),
            future: Some(future),
        });
    }
    Ok(())
}

fn finish_plan(
    plan: &ConcurrentPlan,
    lanes: &mut [LaneRecord],
    parent_heap: &mut RequestHeap,
) -> Result<ConcurrentSchedulerResult> {
    match plan.kind() {
        ConcurrentPlanKind::Statement => Ok(ConcurrentSchedulerResult::Statement),
        ConcurrentPlanKind::Value => {
            let Some(LaneRecord {
                state: LaneRecordState::Normal(success),
                ..
            }) = lanes.last_mut()
            else {
                return Err(invalid_scheduler(
                    "value concurrent completed without a normal tail lane",
                ));
            };
            let tail = success.tail.take().ok_or_else(|| {
                invalid_scheduler("value concurrent completed without a tail carrier")
            })?;
            let checkpoint = parent_heap.checkpoint();
            match deep_clone_runtime_value_carrier_between_heaps(
                &tail.source_heap,
                parent_heap,
                &tail.carrier,
            ) {
                Ok(value) => Ok(ConcurrentSchedulerResult::Value(value)),
                Err(error) => {
                    parent_heap.rollback_to_checkpoint(checkpoint);
                    Err(invalid_scheduler(format!(
                        "tail carrier clone into parent heap failed: {error}"
                    )))
                }
            }
        }
    }
}

fn validate_projected_plan(plan: &ConcurrentPlan) -> Result<()> {
    let mut tail_count = 0_usize;
    for (index, lane) in plan.lanes().iter().enumerate() {
        if lane.source_order() != index {
            return Err(invalid_scheduler(format!(
                "projected lane order is not contiguous at index {index}"
            )));
        }
        if lane
            .dependencies()
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
            || lane
                .dependencies()
                .iter()
                .any(|dependency| *dependency >= index)
        {
            return Err(invalid_scheduler(format!(
                "projected lane {index} dependencies are malformed"
            )));
        }
        match lane.evaluation() {
            LaneEvaluation::Statement { .. } => {}
            LaneEvaluation::Serial { .. } => {
                if lane.export_slot().is_some() {
                    return Err(invalid_scheduler(format!(
                        "projected serial lane {index} has an export slot"
                    )));
                }
            }
            LaneEvaluation::Tail { .. } => {
                tail_count += 1;
                if lane.export_slot().is_some() {
                    return Err(invalid_scheduler(format!(
                        "projected tail lane {index} has an export slot"
                    )));
                }
                if lane.dependencies().iter().copied().ne(0..index) {
                    return Err(invalid_scheduler(format!(
                        "projected tail lane {index} does not close over every prior lane"
                    )));
                }
            }
        }
    }

    let valid_tail_shape = match plan.kind() {
        ConcurrentPlanKind::Statement => tail_count == 0,
        ConcurrentPlanKind::Value => {
            tail_count == 1
                && plan
                    .lanes()
                    .last()
                    .is_some_and(|lane| matches!(lane.evaluation(), LaneEvaluation::Tail { .. }))
        }
    };
    if !valid_tail_shape {
        return Err(invalid_scheduler(
            "projected plan has an invalid tail shape",
        ));
    }
    Ok(())
}

pub(super) fn invalid_scheduler(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "invalid concurrent scheduler state: {}",
        message.into()
    ))
}
