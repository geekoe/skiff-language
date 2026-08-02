use std::{collections::BTreeMap, sync::Arc};

#[cfg(test)]
use std::{future::Future, pin::Pin};

use skiff_artifact_model::{BoundaryValueOwner, ContractTypeRef};
use skiff_runtime_activation::CallbackCapabilityError;
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_linked_program::{CallIr, ExecutableAddr, LinkedTypeRef};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{CallbackCapabilityCarrier, InterfaceReceiverCallAbi, RuntimeValue},
};
use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

use crate::{
    env::Env,
    error::{rematerialize_runtime_error_between_heaps, Result, RuntimeError},
    eval_context::EvalContext,
    heap_access::HeapAccess,
    program_execution::OwnedProgramExecutionContext,
    Interpreter,
};

use super::{callback_capability_error, materialize_callback_value};

#[cfg(test)]
type CallbackOwnerFuture<'heap> =
    Pin<Box<dyn Future<Output = Result<RuntimeValue>> + Send + 'heap>>;

/// Invocation-scoped owner authority. The guard is owned rather than borrowed
/// from the adapter, so a callback wait never keeps the caller context alive.
struct CallbackOwnerWait {
    owner_heap: tokio::sync::OwnedMutexGuard<RequestHeap>,
}

impl CallbackOwnerWait {
    fn new(owner_heap: tokio::sync::OwnedMutexGuard<RequestHeap>) -> Self {
        Self { owner_heap }
    }

    #[cfg(test)]
    async fn run<F>(mut self, invoke: F) -> CallbackOwnerWaitOutcome
    where
        F: for<'heap> FnOnce(&'heap mut RequestHeap) -> CallbackOwnerFuture<'heap> + Send,
    {
        let result = invoke(&mut self.owner_heap).await;
        CallbackOwnerWaitOutcome {
            owner_heap: self.owner_heap,
            result,
        }
    }
}

struct CallbackOwnerWaitOutcome {
    owner_heap: tokio::sync::OwnedMutexGuard<RequestHeap>,
    result: Result<RuntimeValue>,
}

/// Callback work after all caller-owned values have been detached into the
/// callback owner's heap. This value may live across a real Pending poll
/// without borrowing caller heap, caller env, or the caller Actor frame.
pub(crate) struct PreparedCallbackInvocation {
    owner: CallbackOwnerWait,
    owner_context: OwnedProgramExecutionContext,
    owner_call_env: Env,
    caller_addr: ExecutableAddr,
    executable: ExecutableAddr,
    type_args: BTreeMap<String, LinkedTypeRef>,
    receiver: RuntimeValue,
    args: Vec<RuntimeValue>,
    return_type: ContractTypeRef,
    package_schema_records: PackageSchemaRecords,
}

impl PreparedCallbackInvocation {
    /// Recursively executes the owner-local callback exactly once. The
    /// returned outcome retains the owner guard until finalize imports the
    /// result or propagates the method terminal.
    pub(crate) async fn wait(self, interpreter: &Interpreter) -> CompletedCallbackInvocation {
        let Self {
            owner,
            owner_context,
            owner_call_env,
            caller_addr,
            executable,
            type_args,
            receiver,
            args,
            return_type,
            package_schema_records,
        } = self;
        let CallbackOwnerWait { mut owner_heap } = owner;
        let context = owner_context.borrow();
        let result = if context.actor_execution_frame().is_some() {
            Err(RuntimeError::InvalidArtifact(
                "owned callback execution captured the caller Actor frame".to_string(),
            ))
        } else {
            let mut owner_access = HeapAccess::Exclusive(&mut owner_heap);
            interpreter
                .call_program_executable_with_self(
                    context,
                    &mut owner_access,
                    &owner_call_env,
                    &caller_addr,
                    &executable,
                    &type_args,
                    receiver,
                    args,
                )
                .await
        };
        let owner = CallbackOwnerWaitOutcome { owner_heap, result };
        CompletedCallbackInvocation {
            owner,
            return_type,
            package_schema_records,
        }
    }
}

/// Owner-local terminal awaiting caller-heap import. Consuming finalize makes
/// a second import impossible and releases the owner guard on every path.
pub(crate) struct CompletedCallbackInvocation {
    owner: CallbackOwnerWaitOutcome,
    return_type: ContractTypeRef,
    package_schema_records: PackageSchemaRecords,
}

impl CompletedCallbackInvocation {
    pub(crate) fn finalize(self, caller_heap: &mut RequestHeap) -> Result<RuntimeValue> {
        let CallbackOwnerWaitOutcome { owner_heap, result } = self.owner;
        let owner_result = match result {
            Ok(value) => value,
            Err(error) => {
                let error =
                    rematerialize_runtime_error_between_heaps(error, &owner_heap, caller_heap)
                        .unwrap_or_else(|error| error);
                drop(owner_heap);
                return Err(error);
            }
        };
        let result = materialize_callback_value(
            &self.return_type,
            &self.package_schema_records,
            &owner_result,
            &owner_heap,
            caller_heap,
            BoundaryValueOwner::Provider,
        );
        drop(owner_heap);
        result
    }
}

pub(crate) fn prepare_interface_call(
    context: &mut EvalContext<'_, '_>,
    call: &CallIr,
    carrier: &CallbackCapabilityCarrier,
    method_abi_id: &str,
    slot: u32,
    args: Vec<RuntimeValue>,
) -> Result<PreparedCallbackInvocation> {
    let receiver_target = context.context.runtime_assembly_target()?;
    validate_callback_request_generation(
        receiver_target.request_activation().generation(),
        carrier,
    )?;
    let owner = receiver_target
        .activation_by_opaque_id(carrier.owner_activation_id())
        .ok_or_else(|| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    let payload = owner
        .callback_capabilities()
        .lookup(carrier)
        .map_err(callback_capability_error)?;
    let adapter = Arc::downcast::<InProcessCallbackAdapter>(payload)
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    let canonical_identity = serde_json::to_string(adapter.canonical_package_schema_type())
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    if canonical_identity != carrier.interface_or_adapter_contract() || !call.type_args.is_empty() {
        return Err(callback_capability_error(
            CallbackCapabilityError::CapabilityUnavailable,
        ));
    }
    let operation = adapter
        .operation(slot, method_abi_id)
        .cloned()
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    if args.len() != operation.parameters().len() {
        return Err(callback_capability_error(
            CallbackCapabilityError::CapabilityUnavailable,
        ));
    }

    let owner_request = receiver_target
        .request_activation()
        .switch_to(owner)
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    let owner_target = receiver_target
        .with_request_activation(owner_request)
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    let owner_context = context.context.clone().switch_activation_owner(
        owner_target,
        crate::program_execution::ActivationExecutionOperation::callback_method(method_abi_id),
    )?;
    let owner_context = OwnedProgramExecutionContext::capture(&owner_context);
    if owner_context.borrow().actor_execution_frame().is_some() {
        return Err(RuntimeError::InvalidArtifact(
            "owned callback context captured the caller Actor frame".to_string(),
        ));
    }
    let mut owner_heap = adapter
        .try_lock_owner_heap_owned()
        .map_err(|_| callback_capability_error(CallbackCapabilityError::CapabilityUnavailable))?;
    let owner_args = prepare_owner_arguments(
        operation.parameters(),
        adapter.package_schema_records(),
        &args,
        context.heap,
        &mut owner_heap,
    )?;
    match operation.receiver_call_abi() {
        InterfaceReceiverCallAbi::ExplicitSelfFirst => {}
    }

    Ok(PreparedCallbackInvocation {
        owner: CallbackOwnerWait::new(owner_heap),
        owner_context,
        owner_call_env: callback_owner_call_env(context.env),
        caller_addr: context.addr.clone(),
        executable: operation.executable().clone(),
        type_args: call.type_args.clone(),
        receiver: adapter.receiver().clone(),
        args: owner_args,
        return_type: operation.return_type().clone(),
        package_schema_records: adapter.package_schema_records().clone(),
    })
}

fn validate_callback_request_generation(
    expected_generation: u64,
    carrier: &CallbackCapabilityCarrier,
) -> Result<()> {
    if expected_generation == carrier.request_generation() {
        Ok(())
    } else {
        Err(callback_capability_error(
            CallbackCapabilityError::CapabilityUnavailable,
        ))
    }
}

fn callback_owner_call_env(caller: &Env) -> Env {
    let mut owner = Env::new();
    owner.stream_sink = caller.stream_sink.clone();
    owner.current_stream_item_type = caller.current_stream_item_type.clone();
    owner.response_stream_sink = caller.response_stream_sink.clone();
    owner.type_substitutions = caller.type_substitutions.clone();
    owner
}

fn prepare_owner_arguments(
    parameters: &[ContractTypeRef],
    schema: &PackageSchemaRecords,
    args: &[RuntimeValue],
    caller_heap: &RequestHeap,
    owner_heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let checkpoint = owner_heap.checkpoint();
    let prepared = parameters
        .iter()
        .zip(args)
        .map(|(ty, value)| {
            materialize_callback_value(
                ty,
                schema,
                value,
                caller_heap,
                owner_heap,
                BoundaryValueOwner::Caller,
            )
        })
        .collect::<Result<Vec<_>>>();
    match prepared {
        Ok(args) => Ok(args),
        Err(error) => {
            owner_heap.rollback_to_checkpoint(checkpoint);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests;
