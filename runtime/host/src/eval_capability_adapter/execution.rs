use super::*;

pub(super) struct RuntimeExecutionControl(pub(super) skiff_runtime_request::OwnedExecutionControl);

impl capability_contract::ExecutionControlApi for RuntimeExecutionControl {
    fn owned(&self) -> capability_contract::OwnedExecutionControl {
        capability_contract::OwnedExecutionControl::new(RuntimeOwnedExecutionControl(
            self.0.clone(),
        ))
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.0.borrow().cancel_flag()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.borrow().cancellation_token()
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.0.borrow().check_cancelled()
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.0.borrow().add_instruction_units(units)
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.0.borrow().poll_execution_budget()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: capability_contract::StreamRuntime,
    ) -> capability_contract::FileSourceStreamContext<'static> {
        capability_contract::FileSourceStreamContext::from_api(
            RuntimeOwnedFileSourceStreamContext {
                stream_runtime: concrete_stream_runtime(&stream_runtime).clone(),
                execution: self.0.clone(),
            },
        )
    }
}

struct RuntimeOwnedExecutionControl(skiff_runtime_request::OwnedExecutionControl);

impl capability_contract::OwnedExecutionControlApi for RuntimeOwnedExecutionControl {
    fn borrow(&self) -> capability_contract::ExecutionControl<'_> {
        capability_contract::ExecutionControl::new(RuntimeExecutionControl(self.0.clone()))
    }

    fn cancelled(&self) -> &AtomicBool {
        self.0.cancelled()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.cancellation_token()
    }
}
