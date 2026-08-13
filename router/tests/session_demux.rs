//! Closed-family demux classification tests (authority design §5.5,
//! C-session §6, C-model-registration §2.2).

use serde_json::json;
use skiff_router::session::demux::{DemuxEvent, DemuxOutcome, RuntimeFrameDemux};
use skiff_router::session::TerminalKind;
use skiff_runtime_transport::protocol::{encode_binary_frame, RuntimeFrameFamily};

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn corpus_frame(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("registration-handshake")
        .join("frames.json");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
            .expect("frames.json parses");
    let hex = root["frames"][name]["frameHex"]
        .as_str()
        .unwrap_or_else(|| panic!("frame {name} missing"));
    hex_decode(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_capabilities_and_health_are_handled() {
        let demux = RuntimeFrameDemux;
        match demux.classify(&corpus_frame("capabilities.runtime-a")) {
            DemuxOutcome::Handled(DemuxEvent::Capabilities(header)) => {
                assert_eq!(header.runtime_id, "runtime-a");
            }
            other => panic!("expected capabilities, got {other:?}"),
        }
        match demux.classify(&corpus_frame("health.empty")) {
            DemuxOutcome::Handled(DemuxEvent::Health(header)) => {
                assert_eq!(header.runtime_id, "runtime-a");
            }
            other => panic!("expected health, got {other:?}"),
        }
    }
    #[test]
    fn session_legacy_register_is_rejected_by_demux() {
        // The legacy `runtime.register` tuple frame is retired (M4):
        // capabilities-only registration; its bytes fail closed as a
        // malformed frame. The corpus no longer carries the retired bytes,
        // so they are rebuilt inline from the legacy header shape.
        let bytes = encode_binary_frame(
            &json!({
                "schemaVersion": "skiff-runtime-frame-v4",
                "type": "runtime.register",
                "runtimeId": "runtime-a",
                "generation": 42,
            }),
            &[],
        )
        .expect("legacy register frame encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&bytes),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
    }

    #[test]
    fn session_outbound_only_frames_are_direction_violations() {
        assert_eq!(
            RuntimeFrameDemux.classify(&corpus_frame("bootstrap.prod.42")),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
        assert_eq!(
            RuntimeFrameDemux.classify(&corpus_frame("registered.runtime-a")),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
    }

    #[test]
    fn session_known_families_without_installed_sink_terminate_exact_session() {
        let demux = RuntimeFrameDemux;
        for family in [
            RuntimeFrameFamily::Request,
            RuntimeFrameFamily::Connection,
            RuntimeFrameFamily::Actor,
        ] {
            let bytes = encode_binary_frame(
            &json!({ "type": format!("{}.start", family.wire_type_prefix().trim_end_matches('.')) }),
            &[],
        )
        .expect("frame encodes");
            assert_eq!(
                demux.classify(&bytes),
                DemuxOutcome::Handled(DemuxEvent::Unimplemented { family }),
                "family {family:?} must route to the unimplemented sink"
            );
        }
    }

    #[test]
    fn session_retired_activation_family_fails_closed_as_malformed() {
        // The `assembly.activation` frame family is retired (M4): its legacy
        // bytes have no installed family prefix and terminate the exact
        // session as a malformed frame.
        let bytes = encode_binary_frame(
            &json!({
                "type": "assembly.activation.prepare",
                "activationId": "activation-1",
            }),
            &[],
        )
        .expect("legacy activation frame encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&bytes),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
    }

    #[test]
    fn session_payload_presence_violation_is_malformed() {
        let header = json!({
            "schemaVersion": "skiff-runtime-frame-v4",
            "type": "runtime.capabilities",
            "runtimeId": "runtime-a",
            "capabilities": {
                "dispatchModes": ["unary", "serverStream"],
                "packageTestDispatch": true,
                "requestCancel": true
            }
        });
        let bytes = encode_binary_frame(&header, b"payload").expect("frame encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&bytes),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
    }

    #[test]
    fn session_unknown_family_and_garbage_are_malformed() {
        let unknown =
            encode_binary_frame(&json!({ "type": "telemetry.batch" }), &[]).expect("encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&unknown),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
        assert_eq!(
            RuntimeFrameDemux.classify(b"not a skiff frame"),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        );
    }
}
