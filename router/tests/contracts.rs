use skiff_runtime_request_contract::RuntimeClientSessionControl;
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, FrameDirection, PayloadPresenceRule,
    RuntimeFrameFamily, RuntimeFrameSink, RuntimeFrameSinkRegistration, RUNTIME_FRAME_FAMILY_RULES,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_consumer_compiles_shared_envelope_and_connection_identity() {
        let session = RuntimeClientSessionControl {
            id: "client-1".to_string(),
        };
        let frame = encode_binary_frame(&session, &[]).expect("shared envelope must encode");
        let (decoded, payload): (RuntimeClientSessionControl, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("shared envelope must decode");
        assert_eq!(decoded, session);
        assert!(payload.is_empty());
    }

    #[test]
    fn frame_family_registry_is_closed_and_rule_consistent() {
        assert_eq!(
            RuntimeFrameFamily::ALL,
            [
                RuntimeFrameFamily::Session,
                RuntimeFrameFamily::Request,
                RuntimeFrameFamily::Connection,
                RuntimeFrameFamily::Actor,
                RuntimeFrameFamily::Task,
            ]
        );
        assert_eq!(RUNTIME_FRAME_FAMILY_RULES.len(), 5);
        for rule in RUNTIME_FRAME_FAMILY_RULES {
            assert_eq!(rule.family.direction(), rule.direction);
            assert_eq!(rule.family.payload_presence(), rule.payload_presence);
            assert!(!rule.family.wire_type_prefix().is_empty());
        }
    }

    #[test]
    fn sink_registration_contract_is_stable() {
        struct SessionSink;

        impl RuntimeFrameSink for SessionSink {
            fn registration(&self) -> RuntimeFrameSinkRegistration {
                RuntimeFrameSinkRegistration::new(
                    RuntimeFrameFamily::Session,
                    FrameDirection::Either,
                    PayloadPresenceRule::Empty,
                )
            }
        }

        let sink = SessionSink;
        assert_eq!(sink.registration().family, RuntimeFrameFamily::Session);
    }
}
