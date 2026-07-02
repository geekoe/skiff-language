use super::{unsupported_native_target, RuntimeNativeInvocation};
use crate::capability::NativeActorCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{encode_base64, ActorRef, RequestHeap, RuntimeValue};
use sha2::{Digest, Sha256};
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorKeyControlMetadata, ActorPutControlRequest,
    ActorRemoveControlRequest,
};

const ACTOR_ID_ENCODING_VERSION: &str = "runtime-json-v1";

pub(super) struct ActorNativeDispatch;

impl ActorNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "actor.put"
                | "actor.get"
                | "actor.find"
                | "actor.remove"
                | "std.actor.put"
                | "std.actor.get"
                | "std.actor.find"
                | "std.actor.remove"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch<ActorContext>(
        actor_context: &ActorContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        ActorContext: NativeActorCapability,
    {
        let binding_key = invocation.binding_key();
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

        let actor_id =
            native_boundary.to_wire_arg(0, &args[0], &format!("{diagnostic_target} id"), heap)?;
        let (canonical_actor_id_key_bytes, actor_id_hash) = actor_id_key(&actor_id)?;
        let actor_key = ActorKeyControlMetadata {
            service_id: actor_context.service_id().to_string(),
            actor_type_identity: actor_type_identity.clone(),
            actor_id_type_identity: actor_id_type_identity.clone(),
            actor_id_encoding_version: ACTOR_ID_ENCODING_VERSION.to_string(),
            canonical_actor_id_key_bytes_base64: encode_base64(&canonical_actor_id_key_bytes),
            actor_id_hash: Some(actor_id_hash.clone()),
        };

        let output = match binding_key {
            "actor.put" | "std.actor.put" => {
                let object = native_boundary.to_wire_arg(
                    1,
                    &args[1],
                    &format!("{diagnostic_target} object"),
                    heap,
                )?;
                let object_payload = serde_json::to_vec(&object).map_err(RuntimeError::from)?;
                let actor_ref = actor_context
                    .put_actor(
                        ActorPutControlRequest {
                            rpc_id: String::new(),
                            runtime_id: String::new(),
                            actor_key,
                            object_schema_identity: actor_type_identity,
                            object_encoding_version: ACTOR_ID_ENCODING_VERSION.to_string(),
                        },
                        object_payload,
                    )
                    .await?;
                RuntimeValue::ActorRef(actor_ref)
            }
            "actor.get" | "std.actor.get" => {
                let _ = diagnostic_target;
                let actor_ref = ActorRef::new(
                    actor_context.service_id().to_string(),
                    actor_type_identity,
                    actor_id_type_identity,
                    ACTOR_ID_ENCODING_VERSION,
                    canonical_actor_id_key_bytes,
                    actor_id_hash,
                    None,
                );
                RuntimeValue::ActorRef(actor_ref)
            }
            "actor.find" | "std.actor.find" => {
                let actor_ref = actor_context
                    .find_actor(ActorFindControlRequest {
                        rpc_id: String::new(),
                        runtime_id: String::new(),
                        actor_key,
                    })
                    .await?;
                actor_ref
                    .map(RuntimeValue::ActorRef)
                    .unwrap_or(RuntimeValue::Null)
            }
            "actor.remove" | "std.actor.remove" => {
                let removed = actor_context
                    .remove_actor(ActorRemoveControlRequest {
                        rpc_id: String::new(),
                        runtime_id: String::new(),
                        actor_key,
                    })
                    .await?;
                RuntimeValue::Bool(removed)
            }
            _ => return Err(unsupported_native_target(binding_key)),
        };
        native_boundary.coerce_return(&output, &format!("{diagnostic_target} response"), heap)
    }
}

fn actor_id_key(actor_id: &serde_json::Value) -> Result<(Vec<u8>, String)> {
    let canonical_actor_id_key_bytes = serde_json::to_vec(actor_id).map_err(RuntimeError::from)?;
    let actor_id_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(&canonical_actor_id_key_bytes))
    );
    Ok((canonical_actor_id_key_bytes, actor_id_hash))
}
