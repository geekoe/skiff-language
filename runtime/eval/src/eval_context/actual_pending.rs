use std::future::Future;

use skiff_runtime_linked_program::{
    CallIr, ExprRefIr, LinkedActorMethodDispatchPlan, ServiceDependencySymbolRef,
};
use skiff_runtime_model::runtime_value::{
    CallbackCapabilityCarrier, RuntimeValue, RuntimeValueCarrier,
};
use skiff_runtime_native::dispatch::PreparedNativeCall;

use super::*;
use crate::{
    actor_executor::ActorExecutionFrame, capabilities::ExecutionControl,
    program_execution::ProgramExecutionContext, service_dispatch::PreparedOutboundServiceCall,
};

mod activation;

#[cfg(test)]
pub(super) mod tests;

pub(super) async fn await_operation<F>(
    context: &ProgramExecutionContext<'_>,
    frame: Option<ActorExecutionFrame>,
    heap: &mut RequestHeap,
    execution: &ExecutionControl<'_>,
    future: F,
) -> Result<F::Output>
where
    F: Future,
{
    super::checkpoint::actual_pending_checkpoint(context)?;
    let output = match frame {
        Some(frame) => frame.await_if_pending(heap, execution, future).await?,
        None => future.await,
    };
    super::checkpoint::actual_pending_checkpoint(context)?;
    Ok(output)
}

impl EvalContext<'_> {
    pub(super) async fn await_actual_pending<F>(&mut self, future: F) -> Result<F::Output>
    where
        F: Future,
    {
        let frame = self.context.actor_execution_frame().cloned();
        let execution = self.execution.clone();
        await_operation(&self.context, frame, self.heap, &execution, future).await
    }

    pub(super) async fn exec_emit(&mut self, value: ExprRefIr) -> Result<Flow> {
        let value = self.eval_program_expr_ref(value).await?;
        let sink = self
            .env
            .stream_sink
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Decode("emit used outside a stream output context".to_string())
            })?
            .clone();
        let cancellation = self.execution.cancellation_token();
        if let Some(item) = sink.project_runtime_item(value.value().clone(), self.heap)? {
            self.await_actual_pending(sink.send_internal_with_cancellation(
                item,
                &[],
                [cancellation],
            ))
            .await??;
            return Ok(Flow::Continue);
        }
        if !super::super::assembly_execution::is_canonical_boundary_stream_sink(&sink) {
            let mut item_heap = self.context.request_heap();
            let value =
                deep_clone_runtime_value_carrier_between_heaps(self.heap, &mut item_heap, &value)?;
            let cell = item_heap.alloc_local_carrier_cell(value)?;
            let item = StreamInternalItem::new(RuntimeValue::Heap(cell), item_heap);
            self.await_actual_pending(sink.send_internal_with_cancellation(
                item,
                &[],
                [cancellation],
            ))
            .await??;
            return Ok(Flow::Continue);
        }
        let value = runtime_to_wire_required_plan(
            &value,
            self.env.current_stream_item_type.as_ref(),
            "stream emit item",
            self.heap,
        )?;
        self.await_actual_pending(sink.send_with_cancellation(value, &[], [cancellation]))
            .await??;
        Ok(Flow::Continue)
    }

    pub(super) async fn eval_remote_interface_call(
        &mut self,
        dependency_ref: &str,
        operation_abi_id: &str,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        let outbound_context = self.context.outbound_context();
        let stream_runtime = self.context.stream_runtime();
        let prepared = super::super::service_dispatch::prepare_outbound_service_operation(
            self.interpreter,
            &outbound_context,
            &stream_runtime,
            self.heap,
            self.env,
            self.addr,
            dependency_ref,
            operation_abi_id,
            args.iter()
                .cloned()
                .map(RuntimeValueCarrier::into_value)
                .collect(),
        )?;
        self.finish_outbound_call(prepared).await.map(Into::into)
    }

    pub(super) async fn eval_callback_interface_call(
        &mut self,
        call: &CallIr,
        carrier: &CallbackCapabilityCarrier,
        method_abi_id: &str,
        slot: u32,
        args: &[RuntimeValueCarrier],
    ) -> Result<RuntimeValueCarrier> {
        self.context.runtime_assembly_target()?;
        let prepared = super::super::assembly_execution::prepare_callback_capability_call(
            self,
            call,
            carrier,
            method_abi_id,
            slot,
            args.iter()
                .cloned()
                .map(RuntimeValueCarrier::into_value)
                .collect(),
        )?;
        let interpreter = self.interpreter.clone_for_stream_producer();
        let wait = Box::pin(prepared.wait(&interpreter));
        let completed = self.await_actual_pending(wait).await?;
        completed.finalize(self.heap).map(Into::into)
    }

    pub(super) async fn eval_actor_dispatch(
        &mut self,
        plan: &LinkedActorMethodDispatchPlan,
        values: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        let prepared = crate::actor_dispatch::prepare_actor_method(self, plan, values)?;
        let completed = self.await_actual_pending(prepared.into_wait()).await?;
        completed.finalize(self.heap)
    }

    pub(super) async fn eval_legacy_service_dependency(
        &mut self,
        call: &CallIr,
        symbol: &ServiceDependencySymbolRef,
        values: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        self.ensure_legacy_service_path_allowed("service dependency dispatch")?;
        let outbound_context = self.context.outbound_context();
        let stream_runtime = self.context.stream_runtime();
        let prepared = super::super::service_dispatch::prepare_outbound_service(
            self.interpreter,
            &outbound_context,
            &stream_runtime,
            self.heap,
            self.env,
            self.addr,
            call,
            symbol,
            values
                .into_iter()
                .map(RuntimeValueCarrier::into_value)
                .collect(),
        )?;
        self.finish_outbound_call(prepared).await.map(Into::into)
    }

    async fn finish_outbound_call(
        &mut self,
        prepared: PreparedOutboundServiceCall,
    ) -> Result<RuntimeValue> {
        match prepared {
            PreparedOutboundServiceCall::Ready(value) => Ok(value),
            PreparedOutboundServiceCall::ExternalWait(operation) => {
                let completed = self.await_actual_pending(operation.into_wait()).await?;
                completed.finalize(self.heap, self.env)
            }
        }
    }

    pub(super) async fn eval_native_prepared_call(
        &mut self,
        call: &CallIr,
        target: &NativeTarget,
        values: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        let native_dispatch = NativeDispatch::new();
        let invocation = resolve_runtime_execution_native_invocation(
            self.interpreter,
            &self.projection,
            self.addr,
            self.env,
            call,
            target,
        )?;
        let return_plan = invocation.return_plan()?.clone();
        let native_capability_context = project_runtime_execution_native_capability_context(
            &self.context,
            self.projection.clone(),
            self.env.stream_capability_context(),
            invocation.required_context(),
        );
        let prepared = native_dispatch
            .prepare_resolved_native_call(
                native_capability_context,
                invocation,
                values
                    .into_iter()
                    .map(RuntimeValueCarrier::into_value)
                    .collect(),
                self.heap,
            )
            .map_err(RuntimeError::from)?;
        let value = match prepared {
            PreparedNativeCall::Ready(value) => value,
            PreparedNativeCall::ExternalWait(operation) => {
                let (wait, finalize) = operation.into_parts();
                let outcome = self.await_actual_pending(wait).await??;
                finalize
                    .finalize(outcome, self.heap)
                    .map_err(RuntimeError::from)?
            }
        };
        runtime_carrier_for_plan(value, &return_plan, "native return", self.heap)
    }
}
