//! M-task-repair demux direction tests (C-model-task §3.0).
//!
//! The task family is mixed-direction: `task.submit.request` arrives
//! Runtime->Router and is dispatched to the installed task sink (absent a
//! sink the exact session terminates); `task.submit.response/error` are
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
    decode_task_submit_request_frame, encode_binary_frame, encode_task_submit_request_frame,
    RuntimeFrameFamily, TaskSubmitRequestFrame,
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
        .join("task-wire")
        .join("frames.json");
    let root: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
            .expect("frames.json parses");
    let hex = root["frames"][name]["frameHex"]
        .as_str()
        .unwrap_or_else(|| panic!("frame {name} missing"));
    hex_bytes(hex)
}

/// Canonical task sink probe: decodes through the shared codec (the real
/// `TaskSubmitRouter` boundary) and records the decoded frame. Any decode
/// failure (legacy shape, closed-enum violation) rejects with
/// `MalformedFrame`, terminating the exact session.
#[derive(Debug)]
struct TaskSinkProbe {
    seen: Mutex<Vec<TaskSubmitRequestFrame>>,
}

impl InboundFrameSink for TaskSinkProbe {
    fn family(&self) -> RuntimeFrameFamily {
        RuntimeFrameFamily::Task
    }

    fn handle(&self, _session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        let (header, payload) =
            decode_task_submit_request_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
        self.seen
            .lock()
            .expect("sink lock")
            .push(TaskSubmitRequestFrame { header, payload });
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
    fn task_repair_direction_submit_requests_are_accepted_from_runtime_and_dispatched() {
        let demux = RuntimeFrameDemux;
        for name in [
            "task.submit.request.function",
            "task.submit.request.actorMethod",
        ] {
            let bytes = corpus_frame(name);
            let sink = Arc::new(TaskSinkProbe {
                seen: Mutex::new(Vec::new()),
            });
            let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
            let sinks = InboundSinkSet {
                task: Some(sink_trait.clone()),
                ..Default::default()
            };
            let outcome = demux.classify_with_sinks(&bytes, &sinks);
            match outcome {
                DemuxOutcome::Handled(DemuxEvent::Sink { family, raw }) => {
                    assert_eq!(family, RuntimeFrameFamily::Task, "{name}: family");
                    assert_eq!(raw, bytes, "{name}: raw dispatch bytes preserved");
                    // The session task invokes the installed sink with the raw
                    // frame; the sink owns the canonical codec.
                    sink_trait
                        .handle(&session(), &raw)
                        .expect("canonical task.submit.request must be accepted");
                }
                other => panic!("{name}: expected Sink dispatch, got {other:?}"),
            }
            let seen = sink.seen.lock().expect("sink lock");
            assert_eq!(seen.len(), 1, "{name}: sink must see the inbound frame");
            assert_eq!(
                encode_task_submit_request_frame(&seen[0].header, &seen[0].payload)
                    .expect("reconstruct"),
                bytes,
                "{name}: decoded dispatch frame must reconstruct the exact wire bytes"
            );
        }
    }

    #[test]
    fn task_repair_direction_submit_without_installed_sink_terminates_strictly() {
        let bytes = corpus_frame("task.submit.request.function");
        let demux = RuntimeFrameDemux;
        assert_eq!(
            demux.classify(&bytes),
            DemuxOutcome::Handled(DemuxEvent::Unimplemented {
                family: RuntimeFrameFamily::Task
            }),
            "task.submit.request without an installed sink must route to the \
             unimplemented sink; the session task terminates it with \
             UnimplementedFamily (authority design §6.1)"
        );
    }

    #[test]
    fn task_repair_direction_response_and_error_from_runtime_are_direction_violations() {
        let demux = RuntimeFrameDemux;
        let sink = Arc::new(TaskSinkProbe {
            seen: Mutex::new(Vec::new()),
        });
        let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
        let sinks = InboundSinkSet {
            task: Some(sink_trait),
            ..Default::default()
        };
        for name in ["task.submit.response", "task.submit.error.parentNotFound"] {
            let bytes = corpus_frame(name);
            assert_eq!(
                demux.classify(&bytes),
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                "{name}: outbound-only task frame must be a direction violation \
                 without a sink"
            );
            assert_eq!(
                demux.classify_with_sinks(&bytes, &sinks),
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                "{name}: outbound-only task frame must fail closed even with a \
                 task sink installed"
            );
        }
        assert!(sink.seen.lock().expect("sink lock").is_empty());
    }

    #[test]
    fn task_repair_direction_legacy_request_has_no_compatible_reader() {
        let bytes = corpus_frame("task.submit.request.legacy-no-caller-kind");
        let sink = Arc::new(TaskSinkProbe {
            seen: Mutex::new(Vec::new()),
        });
        let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
        let sinks = InboundSinkSet {
            task: Some(sink_trait.clone()),
            ..Default::default()
        };
        // Direction-valid (RuntimeToRouter), so the demux dispatches; the
        // canonical codec owned by the sink rejects the legacy shape with no
        // compatible reader.
        match RuntimeFrameDemux.classify_with_sinks(&bytes, &sinks) {
            DemuxOutcome::Handled(DemuxEvent::Sink { family, raw }) => {
                assert_eq!(family, RuntimeFrameFamily::Task);
                assert_eq!(raw, bytes);
            }
            other => panic!("legacy frame must dispatch to the task sink, got {other:?}"),
        }
        assert_eq!(
            sink_trait.handle(&session(), &bytes),
            Err(TerminalKind::MalformedFrame),
            "legacy old-shape task.submit.request (no callerKind) must be rejected \
             by the canonical codec and terminate the exact session"
        );
    }

    #[test]
    fn task_repair_direction_unknown_task_frame_is_malformed() {
        let unknown = encode_binary_frame(&serde_json::json!({ "type": "task.unknown" }), &[])
            .expect("frame encodes");
        assert_eq!(
            RuntimeFrameDemux.classify(&unknown),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
            "unknown task frame type must not dispatch"
        );
    }

    #[test]
    fn task_repair_direction_status_and_cancel_frames_are_declared_and_fail_closed_before_d2() {
        let demux = RuntimeFrameDemux;
        // RuntimeToRouter control requests are declared by the D1 direction
        // table. Without an installed handler they route to the task sink
        // owner; the D1 sink only accepts task.submit.request, so the exact
        // session fails closed (D2 installs the real handler).
        for name in ["task.status.request", "task.cancel.request"] {
            let bytes = corpus_frame(name);
            assert_eq!(
                demux.classify(&bytes),
                DemuxOutcome::Handled(DemuxEvent::Unimplemented {
                    family: RuntimeFrameFamily::Task
                }),
                "{name}: RuntimeToRouter task control request must reach the task lane"
            );
            let sink = Arc::new(TaskSinkProbe {
                seen: Mutex::new(Vec::new()),
            });
            let sink_trait: Arc<dyn InboundFrameSink> = sink.clone();
            let sinks = InboundSinkSet {
                task: Some(sink_trait.clone()),
                ..Default::default()
            };
            match demux.classify_with_sinks(&bytes, &sinks) {
                DemuxOutcome::Handled(DemuxEvent::Sink { family, raw }) => {
                    assert_eq!(family, RuntimeFrameFamily::Task);
                    assert_eq!(raw, bytes);
                    assert_eq!(
                        sink_trait.handle(&session(), &raw),
                        Err(TerminalKind::MalformedFrame),
                        "{name}: D1 sink has no status/cancel handler (D2)"
                    );
                }
                other => panic!("{name}: expected Sink dispatch, got {other:?}"),
            }
        }

        // RouterToRuntime responses remain direction violations when sent by
        // the Runtime, exactly like task.submit.response/error.
        for name in ["task.status.response.scheduled", "task.cancel.response.canceled"] {
            let bytes = corpus_frame(name);
            assert_eq!(
                demux.classify(&bytes),
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                "{name}: outbound-only task frame must be a direction violation"
            );
        }
    }
}
