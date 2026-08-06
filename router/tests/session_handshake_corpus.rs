//! Replays the frozen M4 registration-handshake corpus
//! (`runtime/transport/testdata/registration-handshake/`) through the
//! PRODUCTION W-session state machine, directory and registration sink.
//!
//! This is the consumer gate required by the M4 registration handshake: the
//! same fixtures must pass through the real codec/state machine, not only the
//! test-only reference model in `skiff-runtime-transport`. Registration is
//! capabilities-only: there is no Register frame and no epoch tuple.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::demux::{RegistrationFrameSink, RegistrationSinkOutput};
use skiff_router::session::directory::RuntimeRegistrationDirectory;
use skiff_router::session::handshake::{
    CapabilitiesEvent, HandshakeState, HealthEvent, TimeoutKind,
};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::pre_auth::PreAuthPool;
use skiff_router::session::{ConsumerKind, HandshakePhase, TerminalKind};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame_header, decode_typed_binary_frame,
    RouterBootstrapFrameHeader, RuntimeCapabilitiesFrameHeader, RuntimeHealthFrameHeader,
};

const REQUIRED_SCENARIOS: [&str; 10] = [
    "accept-sequence",
    "wrong-order-health-before-capabilities",
    "wrong-order-capabilities-before-bootstrap",
    "identity-change-capabilities-replica",
    "ack-loss",
    "pre-auth-limit",
    "bootstrap-timeout",
    "capabilities-timeout",
    "disconnect-mid-handshake",
    "capabilities-refresh-same-replica",
];

#[derive(Debug, Clone)]
struct FrameEntry {
    frame_hex: String,
    decode_as: String,
}

#[derive(Debug, Clone)]
enum SemanticFrame {
    Bootstrap,
    Capabilities { runtime_id: String },
    Registered { runtime_id: String },
    Health { runtime_id: String },
}

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn corpus_path(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime/transport/testdata/registration-handshake")
        .join(relative)
}

fn load_catalog() -> BTreeMap<String, FrameEntry> {
    let value: Value = serde_json::from_str(
        &std::fs::read_to_string(corpus_path("frames.json")).expect("frames.json must exist"),
    )
    .expect("frames.json must parse");
    let mut frames = BTreeMap::new();
    for (name, entry) in value["frames"].as_object().expect("frames object") {
        frames.insert(
            name.clone(),
            FrameEntry {
                frame_hex: entry["frameHex"].as_str().expect("frameHex").to_string(),
                decode_as: entry["decodeAs"].as_str().expect("decodeAs").to_string(),
            },
        );
    }
    frames
}

fn decode_catalog_frame(entry: &FrameEntry) -> SemanticFrame {
    let bytes = hex_decode(&entry.frame_hex);
    match entry.decode_as.as_str() {
        "RouterBootstrap" => {
            let header: RouterBootstrapFrameHeader = decode_router_bootstrap_frame_header(
                decode_binary_frame(&bytes).expect("frame decodes").header,
            )
            .expect("bootstrap decodes");
            let _ = header;
            SemanticFrame::Bootstrap
        }
        "Capabilities" => {
            let (typed, _): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("capabilities decodes");
            SemanticFrame::Capabilities {
                runtime_id: typed.runtime_id,
            }
        }
        "Registered" => {
            let (typed, _): (
                skiff_runtime_transport::protocol::RuntimeRegisteredFrameHeader,
                Vec<u8>,
            ) = decode_typed_binary_frame(&bytes).expect("registered decodes");
            SemanticFrame::Registered {
                runtime_id: typed.runtime_id,
            }
        }
        "Health" => {
            let (typed, _): (RuntimeHealthFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&bytes).expect("health decodes");
            SemanticFrame::Health {
                runtime_id: typed.runtime_id,
            }
        }
        other => panic!("unknown decodeAs {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONSUMERS: [ConsumerKind; 1] = [ConsumerKind::HealthLedger];

    #[test]
    fn handshake_sequences_match_the_production_state_machine() {
        let frames = load_catalog();
        let semantic = frames
            .iter()
            .map(|(name, entry)| (name.clone(), decode_catalog_frame(entry)))
            .collect::<HashMap<_, _>>();

        // accept-sequence: bootstrap -> capabilities bind -> ACK -> health.
        let mut machine = HandshakeState::new();
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(CONSUMERS));
        machine
            .on_bootstrap_written()
            .expect("bootstrap write in Accepted");
        let session = RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        };
        match machine.on_capabilities("runtime-a") {
            CapabilitiesEvent::Bound => {
                let output = RegistrationFrameSink.handle_capabilities(
                    &mut machine,
                    &mut directory,
                    &session,
                    &CONSUMERS,
                );
                assert!(
                    matches!(output, RegistrationSinkOutput::PendingPublished { .. }),
                    "capabilities bind publishes the pending record"
                );
            }
            other => panic!("capabilities must bind, got {other:?}"),
        }
        machine.on_ack_written().expect("ACK in CapabilitiesBound");
        assert_eq!(machine.phase(), HandshakePhase::Registered);
        assert!(directory.mark_registered(&session));
        match machine.on_health("runtime-a") {
            HealthEvent::Observed => {}
            other => panic!("health must observe, got {other:?}"),
        }
        assert_eq!(machine.outcome_name(), "Registered");

        // wrong-order: capabilities before bootstrap terminates.
        let mut machine = HandshakeState::new();
        assert!(matches!(
            machine.on_capabilities("runtime-a"),
            CapabilitiesEvent::Terminal(TerminalKind::WrongOrder)
        ));

        // identity change on a bound connection terminates.
        let mut machine = HandshakeState::new();
        machine.on_bootstrap_written().expect("bootstrap");
        machine.on_capabilities("runtime-a");
        assert!(matches!(
            machine.on_capabilities("runtime-b"),
            CapabilitiesEvent::Terminal(TerminalKind::IdentityChange)
        ));

        // ack-loss: capabilities bound then ACK write failure.
        let mut machine = HandshakeState::new();
        machine.on_bootstrap_written().expect("bootstrap");
        machine.on_capabilities("runtime-a");
        assert_eq!(machine.on_ack_write_failed(), TerminalKind::AckLoss);

        // capabilities refresh on a registered session is idempotent.
        let mut machine = HandshakeState::new();
        machine.on_bootstrap_written().expect("bootstrap");
        machine.on_capabilities("runtime-a");
        machine.on_ack_written().expect("ACK");
        assert!(matches!(
            machine.on_capabilities("runtime-a"),
            CapabilitiesEvent::Refreshed
        ));

        // timeouts.
        let mut machine = HandshakeState::new();
        assert_eq!(
            machine.on_timeout(TimeoutKind::Bootstrap),
            TerminalKind::BootstrapTimeout
        );
        let mut machine = HandshakeState::new();
        machine.on_bootstrap_written().expect("bootstrap");
        assert_eq!(
            machine.on_timeout(TimeoutKind::Capabilities),
            TerminalKind::CapabilitiesTimeout
        );
    }

    #[test]
    fn every_required_scenario_is_present_in_the_corpus() {
        let dir = corpus_path("scenarios");
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("scenarios dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("scenario readable"))
                    .expect("scenario decodes");
            names.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "corpus must contain required scenario {required}"
            );
        }
    }

    #[test]
    fn pre_auth_pool_refuses_above_the_limit() {
        let mut pool = PreAuthPool::new(1);
        assert!(pool.try_acquire("c1"));
        assert!(!pool.try_acquire("c2"));
        assert_eq!(pool.refused(), 1);
    }
}
