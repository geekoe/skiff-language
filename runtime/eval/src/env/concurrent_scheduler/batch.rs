use std::{
    future::{poll_fn, Future},
    pin::Pin,
    task::Poll,
};

use skiff_runtime_capability_context::{
    ExecutionScopeLeaseCompletion, ExecutionScopeLeaseTerminal,
};
use skiff_runtime_model::request_heap::RequestHeap;

use crate::error::{
    materialize_request_heap_owned_runtime_error, RequestHeapOwnedStreamError, RuntimeError,
};

use super::{invalid_scheduler, ConcurrentLaneFuture, LaneRecord, LaneSuccess, TailValue};
use crate::env::{concurrent_plan::LaneEvaluation, lane_state::LaneCompletion};

pub(super) type ScopeWaiter = Pin<Box<dyn Future<Output = ExecutionScopeLeaseTerminal> + Send>>;

pub(super) struct RunningLane<'a> {
    pub(super) source_order: usize,
    pub(super) waiter: Option<ScopeWaiter>,
    pub(super) completion: Option<ExecutionScopeLeaseCompletion>,
    pub(super) future: Option<ConcurrentLaneFuture<'a>>,
}

pub(super) struct ReadyLane<'a> {
    pub(super) source_order: usize,
    waiter: Option<ScopeWaiter>,
    completion: Option<ExecutionScopeLeaseCompletion>,
    future: Option<ConcurrentLaneFuture<'a>>,
    completion_value: Option<LaneCompletion>,
}

pub(super) struct EvaluatedLane {
    pub(super) source_order: usize,
    waiter: Option<ScopeWaiter>,
    completion: Option<ExecutionScopeLeaseCompletion>,
    pub(super) success: LaneSuccess,
    pub(super) error: Option<LaneFailure>,
}

pub(super) struct LaneFailure {
    error: RuntimeError,
    heap: RequestHeap,
}

pub(super) async fn poll_ready_batch<'a>(running: &mut Vec<RunningLane<'a>>) -> Vec<ReadyLane<'a>> {
    poll_fn(|context| {
        let mut ready_indexes = Vec::new();
        for (index, lane) in running.iter_mut().enumerate() {
            let future = lane
                .future
                .as_mut()
                .expect("running lane always owns its future");
            let completion_value = match future.as_mut().poll(context) {
                Poll::Ready(completion_value) => Some(completion_value),
                Poll::Pending => None,
            };
            let control_ready = lane
                .waiter
                .as_mut()
                .expect("running lane always owns its scope waiter")
                .as_mut()
                .poll(context)
                .is_ready();
            if completion_value.is_some() || control_ready {
                ready_indexes.push((index, completion_value));
            }
        }
        if ready_indexes.is_empty() {
            return Poll::Pending;
        }

        let mut batch = Vec::with_capacity(ready_indexes.len());
        for (index, completion_value) in ready_indexes.into_iter().rev() {
            let mut lane = running.remove(index);
            let future = lane.future.take();
            batch.push(ReadyLane {
                source_order: lane.source_order,
                waiter: lane.waiter.take(),
                completion: lane.completion.take(),
                future: completion_value
                    .is_none()
                    .then(|| future.expect("control-ready lane retains its pending lane future")),
                completion_value,
            });
        }
        Poll::Ready(batch)
    })
    .await
}

pub(super) fn evaluate_ready_lane(ready: ReadyLane<'_>, lanes: &[LaneRecord]) -> EvaluatedLane {
    let source_order = ready.source_order;
    let projected = &lanes[source_order].lane;
    let (state, outcome) = ready
        .completion_value
        .expect("evaluated lane always has a lane result")
        .into_parts();
    let mut evaluated = EvaluatedLane {
        source_order,
        waiter: ready.waiter,
        completion: ready.completion,
        success: LaneSuccess {
            export: None,
            tail: None,
        },
        error: None,
    };

    match (projected.evaluation(), outcome) {
        (_, Err(error)) => {
            evaluated.error = Some(LaneFailure {
                error,
                heap: state.into_heap(),
            });
        }
        (LaneEvaluation::Statement { .. }, Ok(None)) => match projected.export_slot() {
            Some(slot) => match state.into_export(source_order, slot) {
                Ok(export) => evaluated.success.export = Some(export),
                Err(error) => {
                    evaluated.error = Some(LaneFailure {
                        error,
                        heap: RequestHeap::default(),
                    });
                }
            },
            None => drop(state),
        },
        (LaneEvaluation::Serial { .. }, Ok(None)) => drop(state),
        (LaneEvaluation::Tail { .. }, Ok(Some(carrier))) => {
            let (source_heap, carrier) = state.into_heap_and_outcome(carrier);
            evaluated.success.tail = Some(TailValue {
                source_heap,
                carrier,
            });
        }
        (LaneEvaluation::Statement { .. } | LaneEvaluation::Serial { .. }, Ok(Some(_))) => {
            evaluated.error = Some(LaneFailure {
                error: invalid_scheduler(format!("non-tail lane {source_order} returned a value")),
                heap: state.into_heap(),
            });
        }
        (LaneEvaluation::Tail { .. }, Ok(None)) => {
            evaluated.error = Some(LaneFailure {
                error: invalid_scheduler(format!("tail lane {source_order} returned no value")),
                heap: state.into_heap(),
            });
        }
    }
    evaluated
}

impl ReadyLane<'_> {
    pub(super) fn has_lane_result(&self) -> bool {
        self.completion_value.is_some()
    }
}

pub(super) fn cancel_ready(ready: &mut [ReadyLane<'_>]) {
    for lane in ready {
        lane.waiter.take();
        lane.future.take();
    }
}

pub(super) fn cancel_running(running: &mut Vec<RunningLane<'_>>) {
    for lane in running.iter_mut() {
        lane.waiter.take();
    }
    running.clear();
}

pub(super) fn cancel_evaluated(evaluated: &mut [EvaluatedLane]) {
    for lane in evaluated {
        lane.waiter.take();
    }
}

impl EvaluatedLane {
    pub(super) fn complete_lease(&mut self) -> bool {
        let completed = self
            .completion
            .as_ref()
            .is_some_and(ExecutionScopeLeaseCompletion::complete);
        self.waiter.take();
        self.completion.take();
        completed
    }

    pub(super) fn materialize_error(mut self, parent_heap: &mut RequestHeap) -> RuntimeError {
        let failure = self
            .error
            .take()
            .expect("winner lane always contains an error");
        let owned = match RequestHeapOwnedStreamError::try_new(failure.error, failure.heap) {
            Ok(error) => RuntimeError::Opaque(Box::new(error)),
            Err(error) => error,
        };
        let checkpoint = parent_heap.checkpoint();
        match materialize_request_heap_owned_runtime_error(owned, parent_heap) {
            Ok(error) => error,
            Err(error) => {
                parent_heap.rollback_to_checkpoint(checkpoint);
                invalid_scheduler(format!("winner error materialization failed: {error}"))
            }
        }
    }
}
