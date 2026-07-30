use std::sync::atomic::{AtomicBool, Ordering};

use skiff_runtime_linked_program::LinkedConcurrentPlanIr;

use super::*;
use crate::{
    actor_executor::{ActorConcurrentContinuationBridge, ActorConcurrentContinuationLane},
    env::{
        project_concurrent_plan, run_concurrent_scheduler, ConcurrentLaneExecutor,
        ConcurrentLaneFuture, ConcurrentPlan, ConcurrentPlanKind, ConcurrentSchedulerResult,
        LaneCompletion, LaneEvaluation, LaneExecutionState, ProjectedLane,
    },
};

impl EvalContext<'_> {
    pub(super) async fn exec_concurrent_statement(
        &mut self,
        linked: &LinkedConcurrentPlanIr,
    ) -> Result<Flow> {
        let plan = project_concurrent_plan(linked, self.executable, ConcurrentPlanKind::Statement)?;
        match self.run_evaluator_concurrent_plan(&plan).await? {
            ConcurrentSchedulerResult::Statement => Ok(Flow::Continue),
            ConcurrentSchedulerResult::Value(_) => Err(invalid_evaluator_concurrent(
                "statement concurrent returned a value",
            )),
        }
    }

    pub(super) async fn eval_concurrent_value(
        &mut self,
        linked: &LinkedConcurrentPlanIr,
    ) -> Result<RuntimeValueCarrier> {
        let plan = project_concurrent_plan(linked, self.executable, ConcurrentPlanKind::Value)?;
        match self.run_evaluator_concurrent_plan(&plan).await? {
            ConcurrentSchedulerResult::Value(value) => Ok(value),
            ConcurrentSchedulerResult::Statement => Err(invalid_evaluator_concurrent(
                "value concurrent returned no tail value",
            )),
        }
    }

    async fn run_evaluator_concurrent_plan(
        &mut self,
        plan: &ConcurrentPlan,
    ) -> Result<ConcurrentSchedulerResult> {
        let actor_bridge = self
            .context
            .actor_execution_frame()
            .filter(|_| !plan.lanes().is_empty())
            .map(|frame| frame.begin_concurrent(self.heap, plan.lanes().len()))
            .transpose()?;
        let executor = EvaluatorConcurrentLaneExecutor::new(
            self.interpreter,
            self.context.clone(),
            self.addr,
            self.file,
            self.executable,
            actor_bridge,
            plan.lanes().len(),
        );

        let scheduler_result =
            run_concurrent_scheduler(plan, &*self.env, self.heap, &self.context, &executor).await;
        let close_result = executor.close_unclaimed_actor_lanes();
        let resume_result = executor
            .resume_actor_parent(self.heap, &self.execution)
            .await;

        // A parent resume performs the final Actor fence and outer terminal
        // check. It must win over accepting any scheduler result.
        resume_result?;
        close_result?;
        scheduler_result
    }
}

struct EvaluatorConcurrentLaneExecutor<'program> {
    interpreter: &'program Interpreter,
    parent_context: ProgramExecutionContext<'program>,
    addr: &'program ExecutableAddr,
    file: &'program LinkedFileUnit,
    executable: &'program LinkedExecutable,
    actor_bridge: Option<ActorConcurrentContinuationBridge>,
    actor_claims: Vec<AtomicBool>,
}

impl<'program> EvaluatorConcurrentLaneExecutor<'program> {
    fn new(
        interpreter: &'program Interpreter,
        parent_context: ProgramExecutionContext<'program>,
        addr: &'program ExecutableAddr,
        file: &'program LinkedFileUnit,
        executable: &'program LinkedExecutable,
        actor_bridge: Option<ActorConcurrentContinuationBridge>,
        lane_count: usize,
    ) -> Self {
        Self {
            interpreter,
            parent_context,
            addr,
            file,
            executable,
            actor_bridge,
            actor_claims: (0..lane_count).map(|_| AtomicBool::new(false)).collect(),
        }
    }

    fn claim_actor_lane(
        &self,
        source_order: usize,
    ) -> Result<Option<ActorConcurrentContinuationLane>> {
        let Some(bridge) = &self.actor_bridge else {
            return Ok(None);
        };
        let claimed = self.actor_claims.get(source_order).ok_or_else(|| {
            invalid_evaluator_concurrent(format!(
                "Actor lane {source_order} is outside the projected plan"
            ))
        })?;
        if claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(invalid_evaluator_concurrent(format!(
                "Actor lane {source_order} was claimed more than once"
            )));
        }
        bridge.lane(source_order).map(Some)
    }

    fn close_unclaimed_actor_lanes(&self) -> Result<()> {
        let Some(bridge) = &self.actor_bridge else {
            return Ok(());
        };
        let mut first_error = None;
        for (source_order, claimed) in self.actor_claims.iter().enumerate() {
            if claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            match bridge.lane(source_order) {
                Ok(lane) => lane.abandon(),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn resume_actor_parent(
        &self,
        heap: &mut RequestHeap,
        execution: &ExecutionControl<'_>,
    ) -> Result<()> {
        match &self.actor_bridge {
            Some(bridge) => bridge.resume_parent(heap, execution).await,
            None => Ok(()),
        }
    }
}

impl<'executor, 'program: 'executor> ConcurrentLaneExecutor<'executor>
    for EvaluatorConcurrentLaneExecutor<'program>
{
    fn start_lane(
        &'executor self,
        lane: ProjectedLane,
        state: LaneExecutionState,
    ) -> ConcurrentLaneFuture<'executor> {
        let actor_lane = match self.claim_actor_lane(lane.source_order()) {
            Ok(actor_lane) => actor_lane,
            Err(error) => return Box::pin(async move { LaneCompletion::error(state, error) }),
        };
        let interpreter = self.interpreter;
        let parent_context = self.parent_context.clone();
        let addr = self.addr;
        let file = self.file;
        let executable = self.executable;
        Box::pin(async move {
            execute_evaluator_lane(
                interpreter,
                parent_context,
                addr,
                file,
                executable,
                lane,
                state,
                actor_lane,
            )
            .await
        })
    }
}

async fn execute_evaluator_lane<'program>(
    interpreter: &'program Interpreter,
    parent_context: ProgramExecutionContext<'program>,
    addr: &'program ExecutableAddr,
    file: &'program LinkedFileUnit,
    executable: &'program LinkedExecutable,
    lane: ProjectedLane,
    mut state: LaneExecutionState,
    mut actor_lane: Option<ActorConcurrentContinuationLane>,
) -> LaneCompletion {
    let mut lane_context = state.program_context(&parent_context);
    if let Some(actor) = actor_lane.as_ref() {
        let execution = lane_context.execution();
        if let Err(error) = actor.resume(state.heap_mut(), &execution).await {
            return LaneCompletion::error(state, error);
        }
        lane_context = lane_context.with_actor_execution_frame(actor.frame().clone());
    }

    let outcome = evaluate_projected_lane(
        interpreter,
        lane_context,
        addr,
        file,
        executable,
        &lane,
        &mut state,
    )
    .await;

    if outcome.is_ok() {
        if let Some(actor) = actor_lane.take() {
            if let Err(error) = actor.complete(state.heap().clone()) {
                return LaneCompletion::error(state, error);
            }
        }
    } else if let Some(actor) = actor_lane.take() {
        actor.abandon();
    }
    state.complete(outcome)
}

async fn evaluate_projected_lane<'program>(
    interpreter: &'program Interpreter,
    context: ProgramExecutionContext<'program>,
    addr: &'program ExecutableAddr,
    file: &'program LinkedFileUnit,
    executable: &'program LinkedExecutable,
    lane: &ProjectedLane,
    state: &mut LaneExecutionState,
) -> Result<Option<RuntimeValueCarrier>> {
    let evaluation = lane.evaluation().clone();
    let (env, heap) = state.env_and_heap_mut();
    let mut eval = EvalContext::new(interpreter, context, heap, env, addr, file, executable)?;
    match evaluation {
        LaneEvaluation::Statement { body } => {
            let block = program_block(executable, &body)?;
            let [statement_ref] = block.statements.as_slice() else {
                return Err(invalid_evaluator_concurrent(format!(
                    "statement lane {} no longer has one direct statement",
                    lane.source_order()
                )));
            };
            let statement = program_statement_ref(executable, statement_ref)?;
            let flow = eval.exec_program_statement(statement).await?;
            continue_lane(flow, lane.source_order()).map(|()| None)
        }
        LaneEvaluation::Serial { body } => {
            let flow = eval.exec_program_block(&body).await?;
            continue_lane(flow, lane.source_order()).map(|()| None)
        }
        LaneEvaluation::Tail { expression } => {
            eval.eval_program_expr_ref(expression).await.map(Some)
        }
    }
}

fn continue_lane(flow: Flow, source_order: usize) -> Result<()> {
    if matches!(flow, Flow::Continue) {
        return Ok(());
    }
    Err(invalid_evaluator_concurrent(format!(
        "lane {source_order} produced forbidden {} flow",
        flow_kind(&flow)
    )))
}

fn flow_kind(flow: &Flow) -> &'static str {
    match flow {
        Flow::Continue => "continue",
        Flow::Return(_) => "return",
        Flow::Break => "break",
        Flow::LoopContinue => "loop-continue",
        Flow::Parked => "parked",
        Flow::ContinueConsumer => "continue-consumer",
    }
}

fn invalid_evaluator_concurrent(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "invalid concurrent evaluator state: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests;
