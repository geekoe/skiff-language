use skiff_runtime_linked_program::{ActivationRelativeServiceCall, CallIr};
use skiff_runtime_model::runtime_value::{RuntimeValue, RuntimeValueCarrier};

use super::*;

impl EvalContext<'_> {
    #[async_recursion]
    #[cfg(any(test, feature = "legacy-eval"))]
    pub(in crate::eval_context) async fn eval_activation_relative_service_call(
        &mut self,
        call: &CallIr,
        instruction: &ActivationRelativeServiceCall,
        values: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        if self.interpreter.test_effects_enabled {
            let effect_target = TestEffectTarget::contract_operation(
                instruction.operation_id().clone(),
                instruction.expected_protocol_identity().clone(),
            );
            let stream_runtime = self.context.stream_runtime();
            if let Some(result) = self.interpreter.runtime_test_effects.dispatch_service(
                &effect_target,
                &values,
                Some(&stream_runtime),
                self.heap.heap_mut(),
            ) {
                return match result? {
                    ServiceTestEffectDispatch::Complete(value) => Ok(value),
                    ServiceTestEffectDispatch::Throw(throw) => {
                        self.materialize_service_test_throw(call, instruction, throw)
                    }
                };
            }
        }

        let operation = self.prepare_activation_relative_service_call(
            call,
            instruction,
            values
                .into_iter()
                .map(RuntimeValueCarrier::into_value)
                .collect::<Vec<RuntimeValue>>(),
        )?;
        let operation = match operation.ready_result(self) {
            Ok(result) => return result.map(Into::into),
            Err(operation) => operation,
        };
        let wait = Box::pin(operation.wait());
        let completed = self.await_actual_pending(wait).await?;
        completed.finalize(self).map(Into::into)
    }
}

#[cfg(test)]
mod tests;
