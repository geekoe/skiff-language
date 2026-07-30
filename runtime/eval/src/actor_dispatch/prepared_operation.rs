use std::future::Future;

use serde_json::Value;
use skiff_runtime_boundary::{json::RuntimeBoundaryCodec, plan::BoundaryUse};
use skiff_runtime_capability_context::{
    ActorInvocationOutcome, ActorInvocationRequest, CapabilityResult, OwnedActorCapabilityContext,
    OwnedExecutionControl,
};
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValueCarrier, type_plan::RuntimeTypePlan,
};

use super::{actor_cancellation_error, actor_error};
use crate::{
    error::{Result, RuntimeError},
    runtime_ops::runtime_carrier_for_plan,
};

pub(crate) struct PreparedActorMethodInvocation {
    context: OwnedActorCapabilityContext,
    request: ActorInvocationRequest,
    return_plan: RuntimeTypePlan,
    method_name: String,
    timeout_ms: u64,
    execution_control: OwnedExecutionControl,
}

pub(crate) struct ActorMethodInvocationCompletion {
    outcome: CapabilityResult<ActorInvocationOutcome>,
    return_plan: RuntimeTypePlan,
    method_name: String,
    timeout_ms: u64,
}

impl PreparedActorMethodInvocation {
    pub(super) fn new(
        context: OwnedActorCapabilityContext,
        request: ActorInvocationRequest,
        return_plan: RuntimeTypePlan,
        method_name: String,
        timeout_ms: u64,
        execution_control: OwnedExecutionControl,
    ) -> Self {
        Self {
            context,
            request,
            return_plan,
            method_name,
            timeout_ms,
            execution_control,
        }
    }

    pub(crate) fn into_wait(
        self,
    ) -> impl Future<Output = ActorMethodInvocationCompletion> + Send + 'static {
        async move {
            let Self {
                context,
                request,
                return_plan,
                method_name,
                timeout_ms,
                execution_control,
            } = self;
            let outcome = context.invoke_actor(request, execution_control).await;
            ActorMethodInvocationCompletion {
                outcome,
                return_plan,
                method_name,
                timeout_ms,
            }
        }
    }
}

impl ActorMethodInvocationCompletion {
    pub(crate) fn finalize(self, heap: &mut RequestHeap) -> Result<RuntimeValueCarrier> {
        let Self {
            outcome,
            return_plan,
            method_name,
            timeout_ms,
        } = self;
        match outcome {
            Ok(ActorInvocationOutcome::Returned(payload)) => {
                let checkpoint = heap.checkpoint();
                let result = finalize_returned(payload, &return_plan, &method_name, heap);
                if result.is_err() {
                    heap.rollback_to_checkpoint(checkpoint);
                }
                result
            }
            Ok(ActorInvocationOutcome::Cancelled(cancellation)) => {
                Err(actor_cancellation_error(cancellation, timeout_ms))
            }
            Ok(ActorInvocationOutcome::ActorError(error)) => Err(actor_error(error)),
            Err(error) => Err(RuntimeError::Opaque(Box::new(error))),
        }
    }
}

fn finalize_returned(
    payload: Vec<u8>,
    return_plan: &RuntimeTypePlan,
    method_name: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let wire: Value =
        serde_json::from_slice(&payload).map_err(|error| RuntimeError::DecodeTarget {
            target: "actor.method.return".to_string(),
            message: error.to_string(),
        })?;
    let value = RuntimeBoundaryCodec::new(
        return_plan,
        BoundaryUse::NativeReturn,
        format!("Actor method {method_name} return"),
    )
    .from_wire_json(&wire, heap)
    .map_err(RuntimeError::from)?;
    runtime_carrier_for_plan(value, return_plan, "Actor method return", heap)
}
