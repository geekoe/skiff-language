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

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0.borrow().deadline()
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        Ok(self.0.borrow().execution_scope().clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        runtime_owned_execution_control(self.0.borrow().derive_scope(local_deadline, site))
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
                stream_runtime,
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

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0.deadline()
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        Ok(self.0.execution_scope().clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        runtime_owned_execution_control(self.0.derive_scope(local_deadline, site))
    }
}

fn runtime_owned_execution_control(
    execution: std::result::Result<
        skiff_runtime_request::OwnedExecutionControl,
        capability_contract::ExecutionScopeDeriveError,
    >,
) -> std::result::Result<
    capability_contract::OwnedExecutionControl,
    capability_contract::ExecutionScopeAccessError,
> {
    let execution = execution.map_err(capability_contract::ExecutionScopeAccessError::from)?;
    Ok(capability_contract::OwnedExecutionControl::new(
        RuntimeOwnedExecutionControl(execution),
    ))
}

#[cfg(test)]
mod tests;
