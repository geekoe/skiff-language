use skiff_runtime_linked_program::{ActivationRelativeServiceCall, CallIr};
use skiff_runtime_model::runtime_value::{RuntimeValue, RuntimeValueCarrier};

use super::*;

impl EvalContext<'_> {
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
            if let Some(result) = self.interpreter.runtime_test_effects.dispatch_service(
                &effect_target,
                &values,
                Some(&self.interpreter.stream_runtime),
                self.heap,
            ) {
                return match result? {
                    ServiceTestEffectDispatch::Complete(value) => Ok(value),
                    ServiceTestEffectDispatch::Throw(throw) => {
                        self.materialize_service_test_throw(call, instruction, throw)
                    }
                };
            }
        }

        // Frozen R1 handoff: R4 replaces this pre-suspend route atomically with
        // provider unary actual-Pending while preserving serverStream setup.
        let frame = self.context.actor_execution_frame().cloned();
        if let Some(frame) = &frame {
            frame.suspend(self.heap)?;
        }
        let result = super::super::super::assembly_execution::dispatch_service_call(
            self,
            call,
            instruction,
            values
                .into_iter()
                .map(RuntimeValueCarrier::into_value)
                .collect::<Vec<RuntimeValue>>(),
        )
        .await;
        if let Some(frame) = frame {
            frame.resume(self.heap, &self.execution).await?;
        }
        result.map(Into::into)
    }
}
