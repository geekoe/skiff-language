use std::future::Future;

use skiff_artifact_model::{BoundaryStreamContract, InstructionSourceSite};
use skiff_runtime_linked_program::{ActivationRelativeServiceCall, CallIr};
use skiff_runtime_model::runtime_value::RuntimeValue;

use super::{
    prepare_provider_unary, prepared_unary, start_provider_stream, CanonicalServiceErrorChannel,
};
use crate::{
    assembly_execution::service_error_channel::ServiceErrorImportContext,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    program_execution::{ExecutionCheckpoint, ExecutionCheckpointKind},
};

#[cfg(test)]
static ACTIVATION_RELATIVE_WAIT_GATE: std::sync::LazyLock<
    std::sync::Mutex<Option<ActivationRelativeWaitGateState>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
struct ActivationRelativeWaitGateState {
    request_generation: u64,
    started: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

#[cfg(test)]
pub(crate) struct ActivationRelativeWaitGate {
    started: tokio::sync::oneshot::Receiver<()>,
    release: tokio::sync::oneshot::Sender<()>,
}

#[cfg(test)]
impl ActivationRelativeWaitGate {
    pub(crate) fn has_started(&mut self) -> bool {
        self.started.try_recv().is_ok()
    }

    pub(crate) fn release(self) {
        self.release
            .send(())
            .expect("activation-relative wait gate receiver remains installed");
    }
}

pub(crate) struct PreparedActivationRelativeServiceCall {
    operation: PreparedActivationRelativeServiceOperation,
    request_generation: u64,
    remote_service_id: String,
    remote_operation_id: String,
    call_site: InstructionSourceSite,
}

enum PreparedActivationRelativeServiceOperation {
    Ready(Result<RuntimeValue>),
    Unary(prepared_unary::PreparedProviderUnary),
}

pub(crate) struct CompletedActivationRelativeServiceCall {
    completed: prepared_unary::CompletedProviderUnary,
    remote_service_id: String,
    remote_operation_id: String,
    call_site: InstructionSourceSite,
}

impl EvalContext<'_> {
    #[cfg(test)]
    pub(crate) fn install_activation_relative_wait_gate_for_test(
        request_generation: u64,
    ) -> ActivationRelativeWaitGate {
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let previous = ACTIVATION_RELATIVE_WAIT_GATE
            .lock()
            .expect("activation-relative wait gate mutex poisoned")
            .replace(ActivationRelativeWaitGateState {
                request_generation,
                started,
                release: release_rx,
            });
        assert!(
            previous.is_none(),
            "activation-relative wait gates are installed one at a time"
        );
        ActivationRelativeWaitGate {
            started: started_rx,
            release,
        }
    }

    pub(crate) fn prepare_activation_relative_service_call(
        &mut self,
        call: &CallIr,
        instruction: &ActivationRelativeServiceCall,
        args: Vec<RuntimeValue>,
    ) -> Result<PreparedActivationRelativeServiceCall> {
        self.context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::GeneratedChunk,
            0,
        ))?;
        let target = self
            .context
            .runtime_assembly_target()?
            .resolve_service_call(instruction)?;
        super::super::record_in_process_boundary_dispatch(
            super::super::InProcessBoundaryDispatchOrigin::InternalServiceCall,
            &target,
        );
        let request_generation = target.provider_request().generation();
        let remote_service_id = target.contract().service_id.clone();
        let remote_operation_id = target.descriptor().operation_id.as_str().to_string();
        let operation = match &target.descriptor().contract.stream {
            BoundaryStreamContract::Unsupported { reason } => {
                return Err(super::super::unsupported_stream_error(
                    &target.descriptor().operation_id,
                    reason,
                ));
            }
            BoundaryStreamContract::Unary => PreparedActivationRelativeServiceOperation::Unary(
                prepare_provider_unary(self, call, target, args)?,
            ),
            BoundaryStreamContract::ServerStream { .. } => {
                PreparedActivationRelativeServiceOperation::Ready(start_provider_stream(
                    self, call, target, args,
                ))
            }
        };
        Ok(PreparedActivationRelativeServiceCall {
            operation,
            request_generation,
            remote_service_id,
            remote_operation_id,
            call_site: call.site.clone(),
        })
    }
}

impl PreparedActivationRelativeServiceCall {
    pub(crate) fn ready_result(
        self,
        context: &mut EvalContext<'_>,
    ) -> std::result::Result<Result<RuntimeValue>, Self> {
        let Self {
            operation,
            request_generation,
            remote_service_id,
            remote_operation_id,
            call_site,
        } = self;
        match operation {
            PreparedActivationRelativeServiceOperation::Ready(result) => {
                Ok(finish_activation_relative_service_result(
                    context,
                    &remote_service_id,
                    &remote_operation_id,
                    &call_site,
                    result,
                ))
            }
            PreparedActivationRelativeServiceOperation::Unary(prepared) => Err(Self {
                operation: PreparedActivationRelativeServiceOperation::Unary(prepared),
                request_generation,
                remote_service_id,
                remote_operation_id,
                call_site,
            }),
        }
    }

    pub(crate) fn wait(
        self,
    ) -> impl Future<Output = CompletedActivationRelativeServiceCall> + Send + 'static {
        async move {
            let PreparedActivationRelativeServiceCall {
                operation,
                request_generation,
                remote_service_id,
                remote_operation_id,
                call_site,
            } = self;
            #[cfg(not(test))]
            let _ = request_generation;
            #[cfg(test)]
            wait_activation_relative_gate_for_test(request_generation).await;
            let PreparedActivationRelativeServiceOperation::Unary(prepared) = operation else {
                unreachable!("synchronous activation-relative service calls are not awaited")
            };
            CompletedActivationRelativeServiceCall {
                completed: prepared.wait().await,
                remote_service_id,
                remote_operation_id,
                call_site,
            }
        }
    }
}

#[cfg(test)]
async fn wait_activation_relative_gate_for_test(request_generation: u64) {
    let gate = {
        let mut installed = ACTIVATION_RELATIVE_WAIT_GATE
            .lock()
            .expect("activation-relative wait gate mutex poisoned");
        match installed.as_ref() {
            Some(gate) if gate.request_generation == request_generation => installed.take(),
            Some(_) | None => None,
        }
    };
    if let Some(gate) = gate {
        let _ = gate.started.send(());
        let _ = gate.release.await;
    }
}

impl CompletedActivationRelativeServiceCall {
    pub(crate) fn finalize(self, context: &mut EvalContext<'_>) -> Result<RuntimeValue> {
        let result = self.completed.finalize(context.heap.heap_mut());
        finish_activation_relative_service_result(
            context,
            &self.remote_service_id,
            &self.remote_operation_id,
            &self.call_site,
            result,
        )
    }
}

fn finish_activation_relative_service_result(
    context: &mut EvalContext<'_>,
    remote_service_id: &str,
    remote_operation_id: &str,
    call_site: &InstructionSourceSite,
    result: Result<RuntimeValue>,
) -> Result<RuntimeValue> {
    let Err(RuntimeError::FixedServiceFailure(error)) = result else {
        return result;
    };
    let caller_stack_at_site = context.context.exception_stack_for_site(call_site.clone());
    let caller_target = context.context.runtime_assembly_target()?;
    let exception = CanonicalServiceErrorChannel::import_caller_failure(
        error,
        ServiceErrorImportContext {
            execution_image: caller_target.execution_image().as_ref(),
            type_view: caller_target.execution_projection().type_view(),
            caller_heap: context.heap.heap_mut(),
            caller_package_build_id: caller_target
                .activation_context()
                .implementation_package_build_id(),
            caller_executable_addr: context.addr,
            call_site,
            caller_stack_at_site: &caller_stack_at_site,
            remote_service_id,
            remote_operation_id,
        },
    )?;
    super::super::record_in_process_boundary_failure_import(
        caller_target.request_activation().generation(),
        caller_target.activation_context().activation_id().as_str(),
        &exception,
    );
    Err(RuntimeError::UserException(exception))
}
