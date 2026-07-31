use super::{
    prepared::run_prepared_native_call, unsupported_native_target, PreparedExternalNativeOperation,
    PreparedNativeCall, RuntimeNativeInvocation,
};
use crate::capability::NativeActorCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{encode_base64, RequestHeap, RuntimeValue};
use sha2::{Digest, Sha256};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorGetOrCreateControlRequest, ActorKeyControlMetadata,
    ActorRemoveControlRequest, ActorReplaceControlRequest,
};

const ACTOR_VALUE_ENCODING_VERSION: &str = "skiff-canonical-v1";

pub(super) struct ActorNativeDispatch;

impl ActorNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.actor.getOrCreate" | "std.actor.replace" | "std.actor.find" | "std.actor.remove"
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn prepare<'a, ActorContext>(
        actor_context: ActorContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'a>>
    where
        ActorContext: NativeActorCapability + Send + 'a,
    {
        let binding_key = invocation.binding_key().to_string();
        let arg_count = invocation.arg_count()?;
        if args.len() != arg_count {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{diagnostic_target} expected {} argument(s), got {}",
                arg_count,
                args.len()
            )));
        }
        let native_boundary = invocation.native_boundary()?;
        let actor_metadata = invocation.actor_metadata()?;
        let actor_type_identity = actor_metadata.actor_type_identity().to_string();
        let actor_id_type_identity = actor_metadata.actor_id_type_identity().to_string();
        let actor_abi_identity = actor_metadata.actor_abi_identity().to_string();
        let actor_implementation_identity =
            actor_metadata.actor_implementation_identity().to_string();

        let actor_id =
            native_boundary.to_wire_arg(0, &args[0], &format!("{diagnostic_target} id"), heap)?;
        let (canonical_actor_id_key_bytes, actor_id_hash) = actor_id_key(&actor_id)?;
        let actor_key = ActorKeyControlMetadata {
            service_id: actor_context.service_id().to_string(),
            actor_type_identity: actor_type_identity.clone(),
            actor_id_type_identity: actor_id_type_identity.clone(),
            actor_id_encoding_version: ACTOR_VALUE_ENCODING_VERSION.to_string(),
            canonical_actor_id_key_bytes_base64: encode_base64(&canonical_actor_id_key_bytes),
            actor_id_hash: Some(actor_id_hash.clone()),
        };
        let activation_identity =
            actor_context
                .activation_identity()
                .cloned()
                .ok_or_else(|| {
                    RuntimeError::Unsupported(format!(
                        "{diagnostic_target} requires a current pinned ActivationContext"
                    ))
                })?;

        let operation = match binding_key.as_str() {
            "std.actor.getOrCreate" => {
                let bootstrap = native_boundary.to_wire_arg(
                    1,
                    &args[1],
                    &format!("{diagnostic_target} bootstrap"),
                    heap,
                )?;
                let bootstrap_payload =
                    canonical_json_bytes(&bootstrap).map_err(RuntimeError::from)?;
                PreparedExternalNativeOperation::new(
                    async move {
                        actor_context
                            .get_or_create_actor(
                                ActorGetOrCreateControlRequest {
                                    rpc_id: String::new(),
                                    runtime_id: String::new(),
                                    activation_identity,
                                    actor_key,
                                    actor_abi_identity,
                                    actor_implementation_identity: actor_implementation_identity
                                        .clone(),
                                    bootstrap_encoding_version: ACTOR_VALUE_ENCODING_VERSION
                                        .to_string(),
                                },
                                bootstrap_payload,
                            )
                            .await
                            .map(ActorRegistryOutput::ActorRef)
                    },
                    move |output, heap| {
                        finalize_actor_registry_output(
                            &invocation,
                            &diagnostic_target,
                            output,
                            heap,
                        )
                    },
                )
            }
            "std.actor.replace" => {
                let bootstrap = native_boundary.to_wire_arg(
                    1,
                    &args[1],
                    &format!("{diagnostic_target} bootstrap"),
                    heap,
                )?;
                let bootstrap_payload =
                    canonical_json_bytes(&bootstrap).map_err(RuntimeError::from)?;
                PreparedExternalNativeOperation::new(
                    async move {
                        actor_context
                            .replace_actor(
                                ActorReplaceControlRequest {
                                    rpc_id: String::new(),
                                    runtime_id: String::new(),
                                    activation_identity,
                                    actor_key,
                                    actor_abi_identity,
                                    actor_implementation_identity,
                                    bootstrap_encoding_version: ACTOR_VALUE_ENCODING_VERSION
                                        .to_string(),
                                },
                                bootstrap_payload,
                            )
                            .await
                            .map(ActorRegistryOutput::ActorRef)
                    },
                    move |output, heap| {
                        finalize_actor_registry_output(
                            &invocation,
                            &diagnostic_target,
                            output,
                            heap,
                        )
                    },
                )
            }
            "std.actor.find" => PreparedExternalNativeOperation::new(
                async move {
                    actor_context
                        .find_actor(ActorFindControlRequest {
                            rpc_id: String::new(),
                            runtime_id: String::new(),
                            activation_identity,
                            actor_key,
                        })
                        .await
                        .map(ActorRegistryOutput::OptionalActorRef)
                },
                move |output, heap| {
                    finalize_actor_registry_output(&invocation, &diagnostic_target, output, heap)
                },
            ),
            "std.actor.remove" => PreparedExternalNativeOperation::new(
                async move {
                    actor_context
                        .remove_actor(ActorRemoveControlRequest {
                            rpc_id: String::new(),
                            runtime_id: String::new(),
                            activation_identity,
                            actor_key,
                        })
                        .await
                        .map(ActorRegistryOutput::Removed)
                },
                move |output, heap| {
                    finalize_actor_registry_output(&invocation, &diagnostic_target, output, heap)
                },
            ),
            _ => return Err(unsupported_native_target(&binding_key)),
        };
        Ok(PreparedNativeCall::ExternalWait(operation))
    }

    #[allow(dead_code)]
    pub(super) async fn dispatch<ActorContext>(
        actor_context: ActorContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        ActorContext: NativeActorCapability + Send,
    {
        let prepared = Self::prepare(actor_context, invocation, diagnostic_target, args, heap)?;
        run_prepared_native_call(prepared, heap).await
    }
}

enum ActorRegistryOutput {
    ActorRef(crate::runtime_value_facade::ActorRef),
    OptionalActorRef(Option<crate::runtime_value_facade::ActorRef>),
    Removed(bool),
}

fn finalize_actor_registry_output(
    invocation: &RuntimeNativeInvocation,
    diagnostic_target: &str,
    output: ActorRegistryOutput,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let output = match output {
        ActorRegistryOutput::ActorRef(actor_ref) => RuntimeValue::ActorRef(actor_ref),
        ActorRegistryOutput::OptionalActorRef(actor_ref) => actor_ref
            .map(RuntimeValue::ActorRef)
            .unwrap_or(RuntimeValue::Null),
        ActorRegistryOutput::Removed(removed) => RuntimeValue::Bool(removed),
    };
    match output {
        RuntimeValue::ActorRef(_) | RuntimeValue::Null => Ok(output),
        RuntimeValue::Bool(_) => invocation.native_boundary()?.coerce_return(
            &output,
            &format!("{diagnostic_target} response"),
            heap,
        ),
        _ => Err(RuntimeError::InvalidArtifact(format!(
            "{diagnostic_target} produced an invalid actor registry result"
        ))),
    }
}

fn actor_id_key(actor_id: &serde_json::Value) -> Result<(Vec<u8>, String)> {
    let canonical_actor_id_key_bytes =
        canonical_json_bytes(actor_id).map_err(RuntimeError::from)?;
    let actor_id_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(&canonical_actor_id_key_bytes))
    );
    Ok((canonical_actor_id_key_bytes, actor_id_hash))
}

#[cfg(test)]
mod tests;
