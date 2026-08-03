//! M-spawn-repair demux direction tests (C-model-spawn §3.0).
//!
//! The spawn family is mixed-direction: `spawn.submit.request` arrives
//! Runtime->Router and is dispatched to the installed spawn sink (absent a
//! sink the exact session terminates); `spawn.submit.response/error` are
//! Router->Runtime frames and are direction violations if the Runtime sends
//! them. The demux owns framing/direction/family dispatch; the lane sink owns
//! the canonical codec, so legacy/invalid requests reach the sink and are
//! rejected there with no compatible reader (exact session terminates).

use std::sync::Arc;
use std::sync::Mutex;

use serde_json::Value;
use skiff_router::session::demux::{DemuxEvent, DemuxOutcome, InboundFrameSink, InboundSinkSet};
use skiff_router::session::{RuntimeSessionEpoch, TerminalKind};
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_request_frame, encode_binary_frame, encode_spawn_submit_request_frame,
    RuntimeFrameFamily, SpawnSubmitRequestFrame,
};

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn corpus_frame(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("spawn-wire")
        .join("frames.json");
    let root: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
            .expect("frames.json parses");
    let hex = root["frames"][name]["frameHex"]
        .as_str()
        .unwrap_or_else(|| panic!("frame {name} missing"));
    hex_bytes(hex)
}

/// Canonical spawn sink probe: decodes through the shared codec (the real
/// `SpawnSubmitRouter` boundary) and records the decoded frame. Any decode
/// failure (legacy shape, closed-enum violation) rejects with
/// `MalformedFrame`, terminating the exact session.
#[derive(Debug)]
struct SpawnSinkProbe {
    seen: Mutex<Vec<SpawnSubmitRequestFrame>>,
}

impl InboundFrameSink for SpawnSinkProbe {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Spawn
    }

    fn handle(&self, _session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        let (header, payload) =
            decode_spawn_submit_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
        self.seen
            .lock()
            .expect("sink lock")
            .push(SpawnSubmitRequestFrame { header, payload });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_router::session::demux::RuntimeFrameDemux;

    fn session() -> RuntimeSessionEpoch {
        RuntimeSessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        }
    }

    #[test]
    fn spawn_repair_direction_submit_requests_are_accepted_from_runtime_and_dispatched() {
        let demux = RuntimeFrameDemux;
        for name in [
            "spawn.submit.request.function",
            "spawn.submit.request.actorMethod",
        ] {
            let bytes = corpus_frame(name);
            let sink = Arc::new(SpawnSinkProbe {
                seen: Mutex::new(Vec::new()),
            });
            let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
            let sinks = InboundSinkSet {
                spawn: Some(sink_trait.clone()),
                ..Default::default()
            };
            let outcome = demux.classify_with_sinks(&bytes, &sinks);
            match outcome {
                DemuxOutcome::Handled(DemuxEvent::Sink { family, raw }) => {
                    assert_eq!(family, RuntimeFrameFamily::Spawn, "{name}: family");
                    assert_eq!(raw, bytes, "{name}: raw dispatch bytes preserved");
                    // The session task invokes the installed sink with the raw
                    // frame; the sink owns the canonical codec.
                    sink_trait
                        .handle(&session(), &raw)
                        .expect("canonical spawn.submit.request must be accepted");
                }
                other => panic!("{name}: expected Sink dispatch, got {other:?}"),
            }
            let seen = sink.seen.lock().expect("sink lock");
            assert_eq!(seen.len(), 1, "{name}: sink must see the inbound frame");
            assert_eq!(
                encode_spawn_submit_request_frame(&seen[0].header, &seen[0].payload)
                    .expect("reconstruct"),
                bytes,
                "{name}: decoded dispatch frame must reconstruct the exact wire bytes"
            );
        }
    }

    #[test]
    fn spawn_repair_direction_submit_without_installed_sink_terminates_strictly() {
        let bytes = corpus_frame("spawn.submit.request.function");
        let demux = RuntimeFrameDemux;
        assert_eq!(
            demux.classify(&bytes),
            DemuxOutcome::Handled(DemuxEvent::Unimplemented {
                family: RuntimeFrameFamily::Spawn
            }),
            "spawn.submit.request without an installed sink must route to the \
             unimplemented sink; the session task terminates it with \
             UnimplementedFamily (authority design §6.1)"
        );
    }

    #[test]
    fn spawn_repair_direction_response_and_error_from_runtime_are_direction_violations() {
        let demux = RuntimeFrameDemux;
        let sink = Arc::new(SpawnSinkProbe {
            seen: Mutex::new(Vec::new()),
        });
        let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
        let sinks = InboundSinkSet {
            spawn: Some(sink_trait),
            ..Default::default()
        };
        for name in ["spawn.submit.response", "spawn.submit.error.parentNotFound"] {
            let bytes = corpus_frame(name);
            assert_eq!(
                demux.classify(&bytes),
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                "{name}: outbound-only spawn frame must be a direction violation \
                 without a sink"
            );
            assert_eq!(
                demux.classify_with_sinks(&bytes, &sinks),
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                "{name}: outbound-only spawn frame must fail closed even with a \
                 spawn sink installed"
            );
        }
        assert!(sink.seen.lock().expect("sink lock").is_empty());
    }

    #[test]
    fn spawn_repair_direction_legacy_request_has_no_compatible_reader() {
        let bytes = corpus_frame("spawn.submit.request.legacy-no-caller-kind");
        let sink = Arc::new(SpawnSinkProbe {
            seen: Mutex::new(Vec::new()),
        });
        let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
        let sinks = InboundSinkSet {
            spawn: Some(sink_trait.clone()),
            ..Default::default()
        };
        // Direction-valid (RuntimeToRouter), so the demux dispatches; the
        // canonical codec owned by the sink rejects the legacy shape with no
        // compatible reader.
        match RuntimeFrameDemux.classify_with_sinks(&bytes, &sinks) {
            DemuxOutcome::Handled(DemuxEvent::Sink { family, raw }) => {
                assert_eq!(family, RuntimeFrameFamily::Spawn);
                assert_eq!(raw, bytes);
            }
            other => panic!("legacy frame must dispatch to the spawn sink, got {other:?}"),
        }
        assert_eq!(
            sink_trait.handle(&session(), &bytes),
            Err(TerminalKind::MalformedFrame),
            "legacy old-shape spawn.submit.request (no callerKind) must be rejected \
             by the canonical codec and terminate the exact session"
        );
    }

    #[test]
    fn spawn_repair_direction_unknown_spawn_frame_is_malformed() {
        let unknown = encode_binary_frame(&serde_json::json!({ "type": "spawn.unknown" }), &[])
            .expect("frame encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&unknown),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
            "unknown spawn frame type must not dispatch"
        );
    }
}
