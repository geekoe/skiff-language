use futures_util::StreamExt;
use serde_json::json;
use tokio::{
    net::TcpListener,
    sync::mpsc,
    time::{timeout, Duration},
};

use super::*;

const TS: &str = "2026-05-06T12:00:00.000Z";

#[test]
fn redaction_masks_secret_keys_and_limits_strings() {
    let mut event = telemetry_event(TS, TelemetrySource::Runtime);
    event.level = Some(TelemetryLevel::Info);
    event.message = Some("x".repeat(3000));
    event.attrs = Some(Map::from_iter([
        ("authorization".to_string(), json!("Bearer token")),
        ("apiKey".to_string(), json!("sk-test")),
        (
            "nested".to_string(),
            json!({ "password": "pw", "ok": "visible" }),
        ),
        ("long".to_string(), json!("y".repeat(3000))),
    ]));

    let redacted = redact_event(event, DEFAULT_STRING_MAX_CHARS, DEFAULT_EVENT_MAX_BYTES);
    let attrs = redacted.attrs.expect("attrs should remain object");

    assert_eq!(attrs["authorization"], "[redacted]");
    assert_eq!(attrs["apiKey"], "[redacted]");
    assert_eq!(attrs["nested"]["password"], "[redacted]");
    assert_eq!(attrs["nested"]["ok"], "visible");
    assert_eq!(
        redacted
            .message
            .expect("message should remain")
            .chars()
            .count(),
        DEFAULT_STRING_MAX_CHARS
    );
    assert_eq!(
        attrs["long"]
            .as_str()
            .expect("long attr should remain")
            .chars()
            .count(),
        DEFAULT_STRING_MAX_CHARS
    );
}

#[test]
fn redaction_marks_oversized_attrs() {
    let mut event = telemetry_event(TS, TelemetrySource::Runtime);
    event.level = Some(TelemetryLevel::Info);
    event.attrs = Some(Map::from_iter([(
        "large".to_string(),
        json!("x".repeat(9000)),
    )]));

    let redacted = redact_event(event, DEFAULT_STRING_MAX_CHARS, 1024);
    let attrs = redacted.attrs.expect("attrs should be replaced");

    assert_eq!(attrs["truncated"], true);
    assert!(attrs["originalSizeBytes"].as_u64().unwrap() > 1024);
}

#[test]
fn queue_prefers_high_priority_events_when_full() {
    let queue = TelemetryQueue::new(2);
    let mut debug = telemetry_event(TS, TelemetrySource::Runtime);
    debug.level = Some(TelemetryLevel::Debug);
    debug.name = Some("debug".to_string());
    let mut info = telemetry_event(TS, TelemetrySource::Runtime);
    info.level = Some(TelemetryLevel::Info);
    info.message = Some("info".to_string());
    let mut warn = telemetry_event(TS, TelemetrySource::Runtime);
    warn.level = Some(TelemetryLevel::Warn);
    warn.message = Some("warn".to_string());

    assert!(queue.enqueue(debug));
    assert!(queue.enqueue(info));
    assert!(queue.enqueue(warn));

    let drained = queue.drain(10);
    assert_eq!(drained.len(), 2);
    assert!(drained
        .iter()
        .any(|event| event.message.as_deref() == Some("warn")));
    assert_eq!(queue.drop_counters().dropped, 1);
}

#[test]
fn queue_drops_low_priority_event_when_full() {
    let queue = TelemetryQueue::new(1);
    let mut warn = telemetry_event(TS, TelemetrySource::Runtime);
    warn.level = Some(TelemetryLevel::Warn);
    let mut debug = telemetry_event(TS, TelemetrySource::Runtime);
    debug.level = Some(TelemetryLevel::Debug);
    debug.name = Some("debug".to_string());

    assert!(queue.enqueue(warn));
    assert!(!queue.enqueue(debug));

    assert_eq!(queue.len(), 1);
    assert_eq!(queue.drop_counters().dropped, 1);
}

#[test]
fn batch_builder_splits_by_event_count() {
    let events = (0..3)
        .map(|index| {
            let mut event = telemetry_event(TS, TelemetrySource::Runtime);
            event.name = Some(format!("trace-{index}"));
            event
        })
        .collect();
    let mut seq = 1;

    let batches = build_batches("producer-1", &mut seq, events, 2, 262_144);

    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].seq, 1);
    assert_eq!(batches[0].events.len(), 2);
    assert_eq!(batches[1].seq, 2);
    assert_eq!(batches[1].events.len(), 1);
    assert_eq!(seq, 3);
}

#[test]
fn batch_builder_splits_by_bytes() {
    let events = (0..3)
        .map(|index| {
            let mut event = telemetry_event(TS, TelemetrySource::Runtime);
            event.level = Some(TelemetryLevel::Info);
            event.message = Some(format!("{index}-{}", "x".repeat(200)));
            event
        })
        .collect();
    let mut seq = 10;

    let batches = build_batches("producer-1", &mut seq, events, 200, 500);

    assert!(batches.len() > 1);
    assert!(batches
        .iter()
        .all(|batch| serde_json::to_vec(batch).unwrap().len() <= 500));
}

#[test]
fn producer_serializes_register_and_batch_envelopes() {
    let mut config = TelemetryConfig::for_test("producer-1");
    config.queue_max_events = 10;
    config.batch_max_events = 10;
    let producer = TelemetryProducer::new(config);
    let register = serde_json::to_value(producer.register_envelope()).unwrap();

    assert_eq!(register["type"], TELEMETRY_REGISTER_TYPE);
    assert_eq!(register["protocol"], "skiff-telemetry-v1");
    assert_eq!(register["source"], "test");

    let mut event = telemetry_event(TS, TelemetrySource::Test);
    event.level = Some(TelemetryLevel::Info);
    event.message = Some("hello".to_string());
    assert!(producer.emit(event));

    let batches = producer.drain_batches();
    let value = serde_json::to_value(&batches[0]).unwrap();
    assert_eq!(value["type"], TELEMETRY_BATCH_TYPE);
    assert_eq!(value["producerId"], "producer-1");
    assert_eq!(value["seq"], 1);
    assert_eq!(value["events"][0]["message"], "hello");
}

#[test]
fn producer_emits_queue_drop_event_with_drop_counters() {
    let mut config = TelemetryConfig::for_test("producer-1");
    config.queue_max_events = 1;
    config.batch_max_events = 10;
    let producer = TelemetryProducer::new(config);

    let mut first = telemetry_event(TS, TelemetrySource::Test);
    first.name = Some("first".to_string());
    let mut dropped = telemetry_event(TS, TelemetrySource::Test);
    dropped.name = Some("dropped".to_string());

    assert!(producer.emit(first));
    assert!(!producer.emit(dropped));
    assert_eq!(producer.drop_counters().dropped, 1);

    let events: Vec<_> = producer
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect();
    let queue_event = events
        .iter()
        .find(|event| event.name.as_deref() == Some("telemetry.queue"))
        .expect("drop counters should be emitted as a telemetry.queue event");

    assert_eq!(queue_event.source, TelemetrySource::Test);
    assert_eq!(queue_event.runtime_id.as_deref(), Some("runtime-test-1"));
    assert_eq!(queue_event.name.as_deref(), Some("telemetry.queue"));
    assert_eq!(
        queue_event
            .dropped
            .as_ref()
            .and_then(|dropped| dropped.get("dropped"))
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        queue_event
            .dropped
            .as_ref()
            .and_then(|dropped| dropped.get("queueLock"))
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(producer.drop_counters().dropped, 0);
}

#[test]
fn producer_does_not_inject_queue_drop_event_without_drops() {
    let mut config = TelemetryConfig::for_test("producer-1");
    config.queue_max_events = 10;
    config.batch_max_events = 10;
    let producer = TelemetryProducer::new(config);

    let mut event = telemetry_event(TS, TelemetrySource::Test);
    event.name = Some("kept".to_string());
    assert!(producer.emit(event));

    let events: Vec<_> = producer
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name.as_deref(), Some("kept"));
}

#[test]
fn exporter_skeleton_drains_to_test_sink() {
    let producer = TelemetryProducer::new(TelemetryConfig::for_test("producer-1"));
    let exporter = TelemetryExporter::new("ws://127.0.0.1:4002/telemetry", producer.clone());
    let sink = TelemetryTestSink::new();
    let event = telemetry_event(TS, TelemetrySource::Test);

    producer.emit(event);
    exporter.drain_once_to_sink(&sink);

    assert_eq!(exporter.endpoint(), "ws://127.0.0.1:4002/telemetry");
    assert_eq!(sink.events().len(), 1);
}

#[tokio::test]
async fn websocket_exporter_registers_and_sends_batch() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock telemetry should bind");
    let endpoint = format!(
        "ws://{}",
        listener.local_addr().expect("listener should have address")
    );
    let (messages_tx, mut messages_rx) = mpsc::unbounded_channel::<serde_json::Value>();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("telemetry should accept");
        let mut ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("telemetry websocket should accept");
        while let Some(message) = ws.next().await {
            let message = message.expect("telemetry message should decode");
            if !message.is_text() {
                continue;
            }
            let value = serde_json::from_str(message.to_text().unwrap())
                .expect("telemetry envelope should be json");
            let _ = messages_tx.send(value);
        }
    });

    let mut config = TelemetryConfig::for_test("producer-1");
    config.batch_max_events = 1;
    config.flush_interval_ms = 20;
    let producer = TelemetryProducer::new(config);
    let handle = TelemetryExporter::new(endpoint, producer.clone()).start();
    let mut event = telemetry_event(TS, TelemetrySource::Test);
    event.name = Some("request.end".to_string());
    producer.emit(event);

    let register = timeout(Duration::from_secs(2), messages_rx.recv())
        .await
        .expect("register should arrive")
        .expect("register should be present");
    let batch = timeout(Duration::from_secs(2), messages_rx.recv())
        .await
        .expect("batch should arrive")
        .expect("batch should be present");

    assert_eq!(register["type"], TELEMETRY_REGISTER_TYPE);
    assert_eq!(register["producerId"], "producer-1");
    assert_eq!(batch["type"], TELEMETRY_BATCH_TYPE);
    assert_eq!(batch["events"][0]["name"], "request.end");

    handle.shutdown(Duration::from_millis(100)).await;
}

struct FileSinkTempDir {
    path: std::path::PathBuf,
}

impl FileSinkTempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telemetry-file-sink-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for FileSinkTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn read_jsonl_lines(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("telemetry file should exist")
        .lines()
        .map(|line| serde_json::from_str(line).expect("line should be valid json"))
        .collect()
}

fn assert_file_header(header: &Value) {
    assert_eq!(header["type"], TELEMETRY_FILE_HEADER_TYPE);
    assert_eq!(header["protocol"], TELEMETRY_FILE_PROTOCOL);
    assert_eq!(header["producerId"], "producer-1");
    assert_eq!(header["source"], "test");
    assert!(header["createdAt"].as_str().is_some());
}

#[test]
fn file_sink_writes_header_then_one_jsonl_line_per_event() {
    let dir = FileSinkTempDir::new();
    let mut config = TelemetryConfig::for_test("producer-1");
    config.file_path = Some(dir.path().join("custom.jsonl"));
    config.batch_max_events = 10;
    let producer = TelemetryProducer::new(config);
    let sink = TelemetryFileSink::new(producer.clone());

    for index in 0..3 {
        let mut event = telemetry_event(TS, TelemetrySource::Test);
        event.level = Some(TelemetryLevel::Info);
        event.message = Some(format!("msg-{index}"));
        assert!(producer.emit(event));
    }
    sink.drain_once_to_file().expect("file sink flush should succeed");

    let lines = read_jsonl_lines(&dir.path().join("custom.jsonl"));
    assert_eq!(lines.len(), 4, "header plus 3 events, no batch envelope");
    assert_file_header(&lines[0]);
    for (index, line) in lines.iter().enumerate().skip(1) {
        assert_eq!(line["message"], format!("msg-{}", index - 1));
    }
}

#[test]
fn file_sink_defaults_to_producer_id_under_file_root() {
    let dir = FileSinkTempDir::new();
    let mut config = TelemetryConfig::for_test("producer-1");
    config.file_root = dir.path().to_path_buf();
    config.file_path = None;
    let producer = TelemetryProducer::new(config);
    let sink = TelemetryFileSink::new(producer.clone());

    let mut event = telemetry_event(TS, TelemetrySource::Test);
    event.name = Some("default.path".to_string());
    assert!(producer.emit(event));
    sink.drain_once_to_file().expect("file sink flush should succeed");

    let lines = read_jsonl_lines(&dir.path().join("producer-1.jsonl"));
    assert_eq!(lines.len(), 2);
    assert_file_header(&lines[0]);
    assert_eq!(lines[1]["name"], "default.path");
}

#[test]
fn file_sink_resolves_relative_file_path_against_file_root() {
    let dir = FileSinkTempDir::new();
    let mut config = TelemetryConfig::for_test("producer-1");
    config.file_root = dir.path().to_path_buf();
    config.file_path = Some(std::path::PathBuf::from("nested/override.jsonl"));
    let producer = TelemetryProducer::new(config);
    let sink = TelemetryFileSink::new(producer.clone());

    let mut event = telemetry_event(TS, TelemetrySource::Test);
    event.name = Some("relative.override".to_string());
    assert!(producer.emit(event));
    sink.drain_once_to_file().expect("file sink flush should succeed");

    let lines = read_jsonl_lines(&dir.path().join("nested/override.jsonl"));
    assert_eq!(lines.len(), 2);
    assert_file_header(&lines[0]);
    assert_eq!(lines[1]["name"], "relative.override");
}

#[test]
fn file_sink_rotates_on_size_and_retains_max_files() {
    let dir = FileSinkTempDir::new();
    let mut config = TelemetryConfig::for_test("producer-1");
    config.file_path = Some(dir.path().join("telemetry.jsonl"));
    config.file_max_bytes = 1;
    config.file_max_files = 3;
    config.batch_max_events = 100;
    let producer = TelemetryProducer::new(config);
    let sink = TelemetryFileSink::new(producer.clone());

    for index in 0..5 {
        let mut event = telemetry_event(TS, TelemetrySource::Test);
        event.name = Some(format!("evt-{index}"));
        assert!(producer.emit(event));
    }
    sink.drain_once_to_file().expect("file sink flush should succeed");

    let current = read_jsonl_lines(&dir.path().join("telemetry.jsonl"));
    assert_eq!(current.len(), 2, "current file holds header plus last event");
    assert_file_header(&current[0]);
    assert_eq!(current[1]["name"], "evt-4");

    let mut seen = Vec::new();
    for index in 1..=3 {
        let rotated = dir.path().join(format!("telemetry.jsonl.{index}"));
        let lines = read_jsonl_lines(&rotated);
        assert_eq!(lines.len(), 2, "rotated {index} should hold header plus one event");
        assert_file_header(&lines[0]);
        seen.push(lines[1]["name"].as_str().unwrap().to_string());
    }
    assert!(
        !dir.path().join("telemetry.jsonl.4").exists(),
        "oldest rotated file beyond fileMaxFiles must be evicted"
    );
    assert_eq!(seen, vec!["evt-3".to_string(), "evt-2".to_string(), "evt-1".to_string()]);
    assert!(
        !seen.iter().any(|name| name == "evt-0"),
        "evt-0 must be evicted by rotation"
    );
}
