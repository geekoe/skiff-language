use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    ExecutionScope, ExecutionScopeAccessError, OwnedExecutionControl,
};

use super::*;

pub(super) fn request_scope(
    cancellation: CancellationToken,
    deadline: Option<Instant>,
) -> ExecutionScope {
    ExecutionScope::request(cancellation, deadline)
}

pub(super) fn current_scope(
    control: &TestExecutionControl,
) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
    Ok(control.execution_scope.clone())
}

pub(super) fn derive_scope(
    control: &TestExecutionControl,
    local_deadline: Instant,
    site: InstructionSourceSite,
) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
    let execution_scope = control.execution_scope.derive(local_deadline, site)?;
    Ok(OwnedExecutionControl::new(TestExecutionControl {
        cancelled: Arc::clone(&control.cancelled),
        cancellation: control.cancellation.clone(),
        deadline: execution_scope
            .effective_deadline()
            .map(|deadline| deadline.at()),
        execution_scope,
    }))
}
