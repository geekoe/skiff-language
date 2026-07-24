use skiff_runtime_transport::{
    actor_method::{decode_actor_method_frame, ActorMethodFrame},
    BinaryFrameError,
};

/// The wire checkpoint intentionally ends before instance admission and method
/// execution. Keeping this disposition distinct prevents an Actor invocation
/// from reaching the ordinary service request handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorMethodHandoff {
    DispatcherNotImplemented,
}

pub fn receive_actor_method_frame(bytes: &[u8]) -> Result<ActorMethodHandoff, BinaryFrameError> {
    match decode_actor_method_frame(bytes)? {
        ActorMethodFrame::Invoke(_, _) => Ok(ActorMethodHandoff::DispatcherNotImplemented),
        ActorMethodFrame::Cancel(_) => Ok(ActorMethodHandoff::DispatcherNotImplemented),
        ActorMethodFrame::Return(_, _) | ActorMethodFrame::Error(_) => {
            Ok(ActorMethodHandoff::DispatcherNotImplemented)
        }
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity,
    };
    use skiff_runtime_transport::{
        actor_method::{
            encode_actor_method_frame, ActorDeclarationOwnerFrameHeader,
            ActorLogicalRefFrameHeader, ActorMethodDeadlineFrameHeader, ActorMethodFrame,
            ActorMethodInvokeFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
            ACTOR_ARGUMENTS_ENCODING_V1,
        },
        protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn invoke_stops_at_dedicated_unimplemented_handoff() {
        let digest = "a".repeat(64);
        let frame = ActorMethodFrame::Invoke(
            ActorMethodInvokeFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
                envelope_type: "actor.method.invoke".into(),
                invocation_id: "inv:1".into(),
                actor_ref: ActorLogicalRefFrameHeader {
                    service_id: "svc".into(),
                    actor_type_identity: "actor".into(),
                    actor_id_type_identity: "id".into(),
                    actor_id_encoding_version: "v1".into(),
                    canonical_actor_id_key_bytes_base64: "AQ==".into(),
                    actor_id_hash: format!("sha256:{}", "d".repeat(64)),
                    epoch: 1,
                },
                declaration_owner: ActorDeclarationOwnerFrameHeader {
                    unit: ActorOwnerUnitFrameHeader::Service,
                    file: ActorOwnerFileFrameHeader::FileIrIdentity("file".into()),
                    actor_symbol: "Counter".into(),
                },
                actor_abi_identity: ActorAbiIdentity::new(format!(
                    "skiff-actor-abi-v1:sha256:{digest}"
                )),
                actor_implementation_identity: ActorImplementationIdentity::new(format!(
                    "skiff-actor-implementation-v1:sha256:{digest}"
                )),
                method_identity: ActorMethodIdentity::new(format!(
                    "skiff-actor-method-v1:sha256:{digest}"
                )),
                arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.into(),
                deadline: ActorMethodDeadlineFrameHeader {
                    timeout_ms: 10,
                    expires_at: "2026-07-25T00:00:00Z".into(),
                },
                cancellation_correlation: "cancel:1".into(),
            },
            vec![1],
        );
        let wire = encode_actor_method_frame(&frame).unwrap();
        assert_eq!(
            receive_actor_method_frame(&wire).unwrap(),
            ActorMethodHandoff::DispatcherNotImplemented
        );
    }
}
