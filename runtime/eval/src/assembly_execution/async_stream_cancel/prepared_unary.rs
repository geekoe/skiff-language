use std::{collections::BTreeMap, future::Future};

use skiff_artifact_model::PackageBuildId;
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_capability_context::OwnedExecutionControl;
use skiff_runtime_linked_program::{CallIr, ExecutableAddr, LinkedTypeRef};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::{
    await_provider_unary, export_provider_failure, provider_execution_context,
    validate_supported_callback_contract, CallbackNativeCapabilityHooks,
    CanonicalServiceBoundaryPlan, ProviderUnaryWaitTerminal,
};
use crate::{
    env::Env,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    program_execution::OwnedProgramExecutionContext,
    Interpreter, RuntimeAssemblyServiceCallTarget,
};

/// Synchronously prepared activation-relative unary call.
///
/// Every field is owned. In particular, this state contains neither a caller
/// `RequestHeap`/`Env` borrow nor the caller's `EvalContext`/Actor frame. The
/// caller can therefore hand [`Self::wait`] to the Actor actual-Pending seam
/// while retaining independent access to its own synchronous segment.
pub(crate) struct PreparedProviderUnary {
    interpreter: Interpreter,
    provider_context: OwnedProgramExecutionContext,
    provider_heap: RequestHeap,
    provider_invocation_env: Env,
    caller_addr: ExecutableAddr,
    provider_addr: ExecutableAddr,
    type_args: BTreeMap<String, LinkedTypeRef>,
    provider_args: Vec<RuntimeValue>,
    execution: OwnedExecutionControl,
    request: ProviderUnaryRequestOwner,
    target: RuntimeAssemblyServiceCallTarget,
    caller_package_build_id: PackageBuildId,
    parameter_count: usize,
}

/// Owned provider heap plus the raw provider outcome.
///
/// The caller heap remains untouched until [`Self::finalize`] is invoked after
/// the Actor continuation has resumed.
pub(crate) struct CompletedProviderUnary {
    interpreter: Interpreter,
    provider_context: OwnedProgramExecutionContext,
    provider_heap: RequestHeap,
    target: RuntimeAssemblyServiceCallTarget,
    caller_package_build_id: PackageBuildId,
    parameter_count: usize,
    outcome: Result<RuntimeValue>,
}

struct ProviderUnaryRequestOwner {
    request: RequestActivationContext,
    cancel_on_drop: bool,
}

impl ProviderUnaryRequestOwner {
    fn new(request: RequestActivationContext) -> Self {
        Self {
            request,
            cancel_on_drop: true,
        }
    }

    fn request(&self) -> &RequestActivationContext {
        &self.request
    }

    fn complete_wait(&mut self) {
        self.cancel_on_drop = false;
    }
}

impl Drop for ProviderUnaryRequestOwner {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.request.cancel();
        }
    }
}

pub(crate) fn prepare_provider_unary(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<PreparedProviderUnary> {
    validate_supported_callback_contract(
        &target.descriptor().operation_id,
        &target.descriptor().contract.callbacks,
    )?;
    let parameter_count = args.len();
    let boundary = CanonicalServiceBoundaryPlan::new(
        target.descriptor(),
        target.schema_records().as_ref(),
        parameter_count,
    )?;
    let caller_package_build_id = context
        .context
        .runtime_assembly_target()?
        .activation_context()
        .implementation_package_build_id()
        .clone();
    let mut provider_heap = boundary.fresh_provider_heap(context.context.request_heap_limits());
    let caller_hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &caller_hooks)?;
    let provider_context = provider_execution_context(&context.context, &target)?;

    Ok(PreparedProviderUnary {
        interpreter: context.interpreter.clone_for_stream_producer(),
        provider_context: OwnedProgramExecutionContext::capture(&provider_context),
        provider_heap,
        provider_invocation_env: detached_provider_invocation_env(context.env),
        caller_addr: context.addr.clone(),
        provider_addr: target.executable_addr().clone(),
        type_args: call.type_args.clone(),
        provider_args,
        execution: context.execution.owned(),
        request: ProviderUnaryRequestOwner::new(target.provider_request().clone()),
        target,
        caller_package_build_id,
        parameter_count,
    })
}

impl PreparedProviderUnary {
    /// Runs the provider exactly once while owning all provider-side state.
    pub(crate) fn wait(self) -> impl Future<Output = CompletedProviderUnary> + Send + 'static {
        async move {
            let Self {
                interpreter,
                provider_context,
                mut provider_heap,
                provider_invocation_env,
                caller_addr,
                provider_addr,
                type_args,
                provider_args,
                execution,
                mut request,
                target,
                caller_package_build_id,
                parameter_count,
            } = self;
            let terminal = {
                let provider_context = provider_context.borrow();
                let provider_future = interpreter.call_program_executable(
                    provider_context,
                    &mut provider_heap,
                    &provider_invocation_env,
                    &caller_addr,
                    &provider_addr,
                    &type_args,
                    provider_args,
                );
                await_provider_unary(&execution.borrow(), request.request(), provider_future).await
            };
            let outcome = match terminal {
                ProviderUnaryWaitTerminal::Provider(Err(error)) if error.is_cancelled() => {
                    request.request().cancel();
                    Err(error)
                }
                terminal => terminal.into_result(),
            };
            request.complete_wait();

            CompletedProviderUnary {
                interpreter,
                provider_context,
                provider_heap,
                target,
                caller_package_build_id,
                parameter_count,
                outcome,
            }
        }
    }

    #[cfg(test)]
    pub(super) fn provider_context_has_actor_frame(&self) -> bool {
        self.provider_context
            .borrow()
            .actor_execution_frame()
            .is_some()
    }

    #[cfg(test)]
    pub(super) fn complete_for_test(
        mut self,
        outcome: Result<RuntimeValue>,
    ) -> CompletedProviderUnary {
        self.request.complete_wait();
        CompletedProviderUnary {
            interpreter: self.interpreter,
            provider_context: self.provider_context,
            provider_heap: self.provider_heap,
            target: self.target,
            caller_package_build_id: self.caller_package_build_id,
            parameter_count: self.parameter_count,
            outcome,
        }
    }
}

impl CompletedProviderUnary {
    pub(crate) fn finalize(mut self, caller_heap: &mut RequestHeap) -> Result<RuntimeValue> {
        if self.outcome.as_ref().is_err_and(RuntimeError::is_cancelled) {
            return self.outcome;
        }

        // The target and schema are immutable owned records. Prepare already
        // validated this exact plan; rebuilding the borrowed view here keeps
        // the completed operation self-contained without lending target data
        // into the owned wait.
        let boundary = CanonicalServiceBoundaryPlan::new(
            self.target.descriptor(),
            self.target.schema_records().as_ref(),
            self.parameter_count,
        )?;
        let provider_context = self.provider_context.borrow();
        let outcome = match self.outcome {
            Ok(value) => Ok(value),
            Err(RuntimeError::FixedServiceFailure(error)) => {
                Err(RuntimeError::FixedServiceFailure(error))
            }
            Err(error) => Err(RuntimeError::FixedServiceFailure(export_provider_failure(
                &self.interpreter,
                &provider_context,
                &self.provider_heap,
                &self.caller_package_build_id,
                self.target
                    .provider_activation()
                    .implementation_package_build_id(),
                &self
                    .target
                    .provider_activation()
                    .identity()
                    .deployment
                    .service_id,
                self.target.descriptor().operation_id.as_str(),
                &error,
            )?)),
        };
        let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
        boundary.materialize_provider_result(outcome, &mut self.provider_heap, caller_heap, &hooks)
    }
}

pub(crate) async fn execute_provider_unary(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let prepared = prepare_provider_unary(context, call, target, args)?;
    let completed = prepared.wait().await;
    completed.finalize(context.heap)
}

fn detached_provider_invocation_env(caller: &Env) -> Env {
    // `call_program_executable` needs only these owned call-site capabilities
    // and type substitutions. Do not clone caller slots/self: they may contain
    // handles into the caller heap and are not provider state.
    let mut provider = Env::new();
    provider.stream_sink = caller.stream_sink.clone();
    provider.current_stream_item_type = caller.current_stream_item_type.clone();
    provider.response_stream_sink = caller.response_stream_sink.clone();
    provider.type_substitutions = caller.type_substitutions.clone();
    provider
}
