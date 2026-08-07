//! W-composition seam tests: per-session outbound writer registry, inbound
//! sink injection and the preserved `Unimplemented` fail-closed behavior.

use std::sync::{Arc, Mutex};

use skiff_router::config::RouterConfig;
use skiff_router::session::demux::{
    DemuxEvent, DemuxOutcome, InboundFrameSink, InboundSinkSet, RuntimeFrameDemux,
};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::layer::{SessionFrameWriter, SessionLayer, SessionLayerOptions};
use skiff_router::session::TerminalKind;
use skiff_runtime_transport::protocol::{
    encode_binary_frame, RequestCancelFrameHeader, ResponseEndFrameHeader,
    ResponseEndFrameMetadata, RuntimeFrameFamily, RuntimeHttpResponseFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

#[derive(Debug, Clone)]
struct RecordingWriter {
    inner: Arc<Mutex<Vec<Vec<u8>>>>,
    fail: Arc<Mutex<bool>>,
}

impl RecordingWriter {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::new(Mutex::new(false)),
        }
    }

    fn recorded(&self) -> Vec<Vec<u8>> {
        self.inner.lock().unwrap().clone()
    }

    fn fail_next(&self) {
        *self.fail.lock().unwrap() = true;
    }
}

impl SessionFrameWriter for RecordingWriter {
    fn enqueue(&self, bytes: Vec<u8>) -> Result<(), String> {
        if *self.fail.lock().unwrap() {
            return Err("injected queue full".to_string());
        }
        self.inner.lock().unwrap().push(bytes);
        Ok(())
    }
}

fn test_config() -> RouterConfig {
    RouterConfig {
        run_dir: None,
        artifacts_path: "/opt/skiff/artifacts".into(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1,
        http_max_response_bytes: 8_388_608,
        http_port: 0,
        manifests: vec![],
        profile: "dev".to_string(),
        release_mode: None,
        request_timeout_ms: 20_000,
        rewrite: vec![],
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

fn session() -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outbound_writer_registry_enforces_bounded_non_blocking_enqueue() {
        let config = test_config();
        let layer = Arc::new(
            SessionLayer::with_options(config, SessionLayerOptions::default())
                .expect("default session layer options"),
        );
        let session = session();
        let writer = Arc::new(RecordingWriter::new());

        // Missing writer fails closed.
        assert!(layer
            .write_session_frame(&session, b"missing".to_vec())
            .is_err());

        layer.register_frame_writer(&session, writer.clone());
        assert!(layer.has_frame_writer(&session));
        layer
            .write_session_frame(&session, b"one".to_vec())
            .expect("enqueue succeeds after registration");
        assert_eq!(writer.recorded(), vec![b"one".to_vec()]);

        writer.fail_next();
        assert!(layer
            .write_session_frame(&session, b"two".to_vec())
            .is_err());

        layer.unregister_frame_writer(&session);
        assert!(!layer.has_frame_writer(&session));
        assert!(layer
            .write_session_frame(&session, b"three".to_vec())
            .is_err());
    }

    type RecordedCall = (RuntimeSessionEpoch, Vec<u8>);

    #[derive(Debug, Clone, Default)]
    struct RecordingSink {
        calls: Arc<Mutex<Vec<RecordedCall>>>,
    }

    impl InboundFrameSink for RecordingSink {
        fn family(&self) -> RuntimeFrameFamily {
            RuntimeFrameFamily::Request
        }

        fn accepts_frame_type(&self, frame_type: &str) -> bool {
            frame_type.starts_with("response.") || frame_type == "request.cancel"
        }

        fn handle(
            &self,
            session: &RuntimeSessionEpoch,
            raw: &[u8],
        ) -> Result<(), skiff_router::session::TerminalKind> {
            self.calls
                .lock()
                .unwrap()
                .push((session.clone(), raw.to_vec()));
            Ok(())
        }
    }

    fn request_family_frame() -> Vec<u8> {
        let header = ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.end".to_string(),
            request_id: "req-1".to_string(),
            payload_present: true,
            metadata: ResponseEndFrameMetadata::Http(RuntimeHttpResponseFrameHeader {
                status: 200,
                headers: Vec::new(),
            }),
        };
        encode_binary_frame(&header, b"ok".as_slice()).expect("encode response.end")
    }

    fn request_cancel_frame() -> Vec<u8> {
        let header = RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: "req-1".to_string(),
            reason: "client_disconnect".to_string(),
        };
        encode_binary_frame(&header, &[]).expect("encode request.cancel")
    }

    /// Legacy `assembly.activation` bytes (retired in M4): a well-framed
    /// frame whose type has no installed family prefix.
    fn legacy_activation_prepared_bytes() -> Vec<u8> {
        encode_binary_frame(
            &serde_json::json!({
                "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
                "type": "assembly.activation.prepare",
                "activationId": "activation-1",
            }),
            &[],
        )
        .expect("legacy activation frame encodes")
    }

    #[test]
    fn demux_preserves_unimplemented_without_sinks_and_injects_installed_sinks() {
        let demux = RuntimeFrameDemux;
        let request_bytes = request_family_frame();
        let cancel_bytes = request_cancel_frame();

        // Empty sink set: identical to the W-session `classify` behavior.
        let empty = InboundSinkSet::default();
        assert_eq!(
            demux.classify(&request_bytes),
            demux.classify_with_sinks(&request_bytes, &empty)
        );
        // `response.*` is outside the closed family prefixes: without a sink it
        // stays fail-closed MalformedFrame.
        assert!(matches!(
            demux.classify_with_sinks(&request_bytes, &empty),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        ));
        // `request.cancel` matches the closed Request family: without a sink it
        // stays Unimplemented (exact W-session behavior).
        assert!(matches!(
            demux.classify_with_sinks(&cancel_bytes, &empty),
            DemuxOutcome::Handled(DemuxEvent::Unimplemented {
                family: RuntimeFrameFamily::Request
            })
        ));
        // The retired `assembly.activation` family has no wire prefix anymore:
        // its legacy bytes fail closed as a malformed frame (M4).
        assert!(matches!(
            demux.classify_with_sinks(&legacy_activation_prepared_bytes(), &empty),
            DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
        ));

        // Installed sinks receive the raw frame through `DemuxEvent::Sink`.
        let sink = Arc::new(RecordingSink::default());
        let set = InboundSinkSet {
            request: Some(Arc::clone(&sink) as Arc<dyn InboundFrameSink>),
            ..InboundSinkSet::default()
        };
        let DemuxOutcome::Handled(DemuxEvent::Sink {
            family: RuntimeFrameFamily::Request,
            raw,
        }) = demux.classify_with_sinks(&request_bytes, &set)
        else {
            panic!("request frame must route to the installed sink");
        };
        assert_eq!(raw, request_bytes);
        assert!(matches!(
            demux.classify_with_sinks(&cancel_bytes, &set),
            DemuxOutcome::Handled(DemuxEvent::Sink {
                family: RuntimeFrameFamily::Request,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn session_layer_installs_and_exposes_the_sink_bundle() {
        let config = test_config();
        let layer = Arc::new(
            SessionLayer::with_options(config, SessionLayerOptions::default())
                .expect("default session layer options"),
        );
        assert!(layer.inbound_sinks().is_empty());

        let sink = Arc::new(RecordingSink::default());
        let set = InboundSinkSet {
            request: Some(sink),
            ..InboundSinkSet::default()
        };
        layer.install_inbound_sinks(Arc::new(set));
        let installed = layer.inbound_sinks();
        assert!(installed.sink_for(RuntimeFrameFamily::Request).is_some());
        assert!(installed.sink_for(RuntimeFrameFamily::Connection).is_none());

        layer.install_inbound_sinks(Arc::new(InboundSinkSet::default()));
        assert!(layer.inbound_sinks().is_empty());
    }
}
