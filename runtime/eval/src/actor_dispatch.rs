use serde_json::{json, Value};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{json::RuntimeBoundaryCodec, plan::BoundaryUse};
use skiff_runtime_capability_context::{
    ActorInvocationCancellation, ActorInvocationDeadline, ActorInvocationDeclarationOwner,
    ActorInvocationError, ActorInvocationIdentity, ActorInvocationOutcome,
    ActorInvocationOwnerFile, ActorInvocationOwnerUnit, ActorInvocationRequest,
};
use skiff_runtime_linked_program::{FileAddr, LinkedActorMethodDispatchPlan, UnitAddr};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlan, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::runtime_value::{RuntimeValue, RuntimeValueCarrier};

use crate::{
    actor_instance::resolve_actor_declaration,
    error::{Result, RuntimeError, RuntimeErrorPayload},
    eval_context::EvalContext,
    exceptions::annotate_runtime_type_plan,
    runtime_ops::runtime_carrier_for_plan,
};

pub(crate) async fn dispatch_actor_method(
    context: &mut EvalContext<'_>,
    plan: &LinkedActorMethodDispatchPlan,
    values: Vec<RuntimeValueCarrier>,
) -> Result<RuntimeValueCarrier> {
    let (receiver, arguments) = values.split_first().ok_or_else(|| {
        RuntimeError::InvalidArtifact("Actor method call is missing its receiver".to_string())
    })?;
    let RuntimeValue::ActorRef(actor_ref) = receiver.value() else {
        return Err(RuntimeError::InvalidArtifact(
            "Actor method receiver is not an Actor reference".to_string(),
        ));
    };
    let projection = context.execution_projection().clone();
    let declaration = resolve_actor_declaration(projection.type_view(), &plan.declaration_owner)?;
    let mut methods = declaration
        .public_methods
        .iter()
        .filter(|method| method.method_identity == plan.method_identity);
    let method = methods.next().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "Actor method {} is absent from declaration {}",
            plan.method_identity.as_str(),
            declaration.actor_name
        ))
    })?;
    if methods.next().is_some() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "Actor method {} is ambiguous in declaration {}",
            plan.method_identity.as_str(),
            declaration.actor_name
        )));
    }
    if arguments.len() != method.parameters.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "Actor method {} expects {} argument(s), got {}",
            method.name,
            method.parameters.len(),
            arguments.len()
        )));
    }

    let type_context = PlanContext::from_type_view(projection.type_view(), context.addr);
    let wire_arguments = arguments
        .iter()
        .zip(&method.parameters)
        .enumerate()
        .map(|(index, (value, parameter))| {
            let type_plan = RuntimeTypePlan::from_linked(&parameter.ty, &type_context)?;
            RuntimeBoundaryCodec::new(
                &type_plan,
                BoundaryUse::NativeArg,
                format!("Actor argument {index}"),
            )
            .to_wire_json(value.value(), context.heap)
            .map_err(RuntimeError::from)
        })
        .collect::<Result<Vec<Value>>>()?;
    let arguments_payload = canonical_json_bytes(&Value::Array(wire_arguments))
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    let timeout_ms = context
        .context
        .outbound_context()
        .effective_timeout_ms(None)
        .unwrap_or(30_000)
        .max(1);
    let invocation_id = format!("actor-method:{}", uuid::Uuid::new_v4());
    let cancellation_correlation = format!("{invocation_id}:cancel");
    let request = ActorInvocationRequest {
        actor_ref: actor_ref.clone(),
        declaration_owner: declaration_owner(&plan.declaration_owner),
        identity: ActorInvocationIdentity {
            invocation_id,
            expected_epoch: actor_ref.epoch().ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "Actor method receiver is missing its pinned epoch".to_string(),
                )
            })?,
            actor_abi_identity: plan.actor_abi_identity.clone(),
            requested_implementation_identity: plan.actor_implementation_identity.clone(),
            method_identity: plan.method_identity.clone(),
            cancellation_correlation,
        },
        deadline: ActorInvocationDeadline { timeout_ms },
        arguments_payload,
    };
    let outcome = context
        .context
        .actor_context()
        .invoke_actor(request)
        .await
        .map_err(|error| RuntimeError::Opaque(Box::new(error)))?;
    match outcome {
        ActorInvocationOutcome::Returned(payload) => {
            let wire: Value =
                serde_json::from_slice(&payload).map_err(|error| RuntimeError::DecodeTarget {
                    target: "actor.method.return".to_string(),
                    message: error.to_string(),
                })?;
            let mut return_plan = RuntimeTypePlan::from_linked(&method.return_type, &type_context)?;
            annotate_runtime_type_plan(
                &mut return_plan,
                &method.return_type,
                projection.type_view(),
            )?;
            let value = RuntimeBoundaryCodec::new(
                &return_plan,
                BoundaryUse::NativeReturn,
                format!("Actor method {} return", method.name),
            )
            .from_wire_json(&wire, context.heap)
            .map_err(RuntimeError::from)?;
            runtime_carrier_for_plan(value, &return_plan, "Actor method return", context.heap)
        }
        ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::Cancelled) => {
            Err(RuntimeError::Cancelled)
        }
        ActorInvocationOutcome::Cancelled(ActorInvocationCancellation::DeadlineExceeded) => {
            Err(RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
                code: "DeadlineExceeded".to_string(),
                message: "Actor method deadline exceeded".to_string(),
                status: None,
                details: None,
            }))
        }
        ActorInvocationOutcome::ActorError(error) => Err(actor_error(error)),
    }
}

fn declaration_owner(
    owner: &skiff_runtime_linked_program::LinkedActorDeclarationOwner,
) -> ActorInvocationDeclarationOwner {
    ActorInvocationDeclarationOwner {
        unit: match owner.unit {
            UnitAddr::Service => ActorInvocationOwnerUnit::Service,
            UnitAddr::Package(slot) => ActorInvocationOwnerUnit::Package(slot as u64),
        },
        file: match &owner.file {
            FileAddr::LoadedFileIndex(index) => {
                ActorInvocationOwnerFile::LoadedFileIndex(*index as u64)
            }
            FileAddr::FileIrIdentity(identity) => {
                ActorInvocationOwnerFile::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: owner.actor_symbol.clone(),
    }
}

fn actor_error(error: ActorInvocationError) -> RuntimeError {
    let (code, message, details) = match error {
        ActorInvocationError::ActorUpgrading { retry_after_ms } => (
            "ActorUpgradingError",
            "Actor is upgrading".to_string(),
            json!({ "retryAfterMs": retry_after_ms }),
        ),
        ActorInvocationError::ActorVersionRejected {
            requested,
            accepted,
        } => (
            "ActorVersionRejectedError",
            "Actor implementation version was rejected".to_string(),
            json!({
                "requestedImplementationIdentity": requested.as_str(),
                "acceptedImplementationIdentity": accepted.as_str(),
            }),
        ),
        ActorInvocationError::ActorIncarnationReplaced {
            requested_epoch,
            current_epoch,
        } => (
            "ActorIncarnationReplacedError",
            "Actor incarnation was replaced".to_string(),
            json!({
                "requestedEpoch": requested_epoch,
                "currentEpoch": current_epoch,
            }),
        ),
    };
    RuntimeError::RootRuntimePayload(RuntimeErrorPayload {
        code: code.to_string(),
        message,
        status: None,
        details: Some(details),
    })
}
