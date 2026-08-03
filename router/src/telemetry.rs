//! Router task-dispatch telemetry (skiff-telemetry-v1 reuse).
//!
//! The router historically only relayed telemetry configuration to runtimes;
//! this module adds the control-plane producer for task dispatch
//! observability using the exact same `TelemetryEvent` /
//! `TelemetryBatchEnvelope` protocol as `runtime/host/src/host/telemetry.rs`
//! (register + bounded batch over the `/telemetry` WebSocket). No second
//! protocol is introduced, and telemetry never blocks or mutates business
//! state: the default sink is a no-op and the producer queue is bounded.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime};

use futures_util::{Sink, SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use tokio::{
    sync::{watch, Notify},
    task::JoinHandle,
    time::{sleep, timeout, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use skiff_runtime_transport::protocol::{
    TelemetryBatchEnvelope, TelemetryEvent, TelemetryLevel, TelemetryProtocol,
    TelemetryRegisterEnvelope, TelemetrySource, TelemetryTopic, TelemetryVisibility,
};

use crate::config::RouterConfig;

pub const TELEMETRY_REGISTER_TYPE: &str = "telemetry.register";
pub const TELEMETRY_BATCH_TYPE: &str = "telemetry.batch";
const DEFAULT_QUEUE_MAX_EVENTS: usize = 10_000;
const DEFAULT_BATCH_MAX_EVENTS: usize = 200;
const DEFAULT_BATCH_MAX_BYTES: usize = 262_144;
const DEFAULT_STRING_MAX_CHARS: usize = 2048;
const DEFAULT_EVENT_MAX_BYTES: usize = 16 * 1024;
const EXPORTER_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const EXPORTER_SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);

/// Read-only telemetry sink consumed by the task control plane / scheduler
/// observation. Emitters never await or block on telemetry.
pub trait TaskTelemetrySink: Send + Sync + fmt::Debug {
    fn emit(&self, event: TelemetryEvent) -> bool;
}

/// Default disabled sink (no `telemetry.endpoint` / disabled config).
#[derive(Debug, Default)]
pub struct NoopTaskTelemetrySink;

impl TaskTelemetrySink for NoopTaskTelemetrySink {
    fn emit(&self, _event: TelemetryEvent) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
struct RouterTelemetryConfig {
    producer_id: String,
    source: TelemetrySource,
    protocol: TelemetryProtocol,
    topics: Vec<TelemetryTopic>,
    queue_max_events: usize,
    batch_max_events: usize,
    batch_max_bytes: usize,
    flush_interval_ms: u64,
    string_max_chars: usize,
    event_max_bytes: usize,
}

impl RouterTelemetryConfig {
    fn from_router(config: &RouterConfig, telemetry: &crate::config::TelemetryConfig) -> Self {
        let environment = config.environment.as_deref().unwrap_or("router");
        let topics = parse_topics(&telemetry.topics);
        Self {
            producer_id: format!("router:{environment}"),
            source: TelemetrySource::Router,
            protocol: TelemetryProtocol::SkiffTelemetryV1,
            topics,
            queue_max_events: usize::try_from(telemetry.queue_max_events)
                .unwrap_or(DEFAULT_QUEUE_MAX_EVENTS),
            batch_max_events: usize::try_from(telemetry.batch_max_events)
                .unwrap_or(DEFAULT_BATCH_MAX_EVENTS),
            batch_max_bytes: usize::try_from(telemetry.batch_max_bytes)
                .unwrap_or(DEFAULT_BATCH_MAX_BYTES),
            flush_interval_ms: telemetry.flush_interval_ms.max(1),
            string_max_chars: DEFAULT_STRING_MAX_CHARS,
            event_max_bytes: DEFAULT_EVENT_MAX_BYTES,
        }
    }
}

fn parse_topics(raw: &[String]) -> Vec<TelemetryTopic> {
    let mut topics = Vec::new();
    for topic in raw {
        let parsed = match topic.as_str() {
            "log" => TelemetryTopic::Log,
            "trace" => TelemetryTopic::Trace,
            "metric" => TelemetryTopic::Metric,
            "health" => TelemetryTopic::Health,
            "debug" => TelemetryTopic::Debug,
            _ => continue,
        };
        if !topics.contains(&parsed) {
            topics.push(parsed);
        }
    }
    topics
}

#[derive(Debug, Default)]
struct TelemetryDropCounters {
    counters: Mutex<Map<String, Value>>,
}

impl TelemetryDropCounters {
    fn record(&self, topic: &TelemetryTopic) {
        let key = match topic {
            TelemetryTopic::Log => "log",
            TelemetryTopic::Trace => "trace",
            TelemetryTopic::Metric => "metric",
            TelemetryTopic::Health => "health",
            TelemetryTopic::Debug => "debug",
        };
        let mut counters = self.counters.lock().expect("drop counters lock");
        let current = counters
            .get(key)
            .and_then(Value::as_u64)
            .unwrap_or_default();
        counters.insert(key.to_string(), json!(current + 1));
    }

    fn take(&self) -> Map<String, Value> {
        std::mem::take(&mut *self.counters.lock().expect("drop counters lock"))
    }
}

/// Bounded producer for router task-dispatch events. Cloneable; all state is
/// shared. Drop behavior mirrors the runtime producer: bounded queue, drops
/// recorded and surfaced as a `telemetry.dropped` event on the next drain.
#[derive(Debug, Clone)]
pub struct RouterTelemetryProducer {
    config: Arc<RouterTelemetryConfig>,
    events: Arc<Mutex<VecDeque<TelemetryEvent>>>,
    next_seq: Arc<AtomicU64>,
    notify: Arc<Notify>,
    dropped: Arc<TelemetryDropCounters>,
}

impl RouterTelemetryProducer {
    pub fn new(config: &RouterConfig) -> Option<Self> {
        let telemetry = config.telemetry.as_ref()?;
        if !telemetry.enabled || telemetry.endpoint.trim().is_empty() {
            return None;
        }
        Some(Self {
            config: Arc::new(RouterTelemetryConfig::from_router(config, telemetry)),
            events: Arc::new(Mutex::new(VecDeque::new())),
            next_seq: Arc::new(AtomicU64::new(1)),
            notify: Arc::new(Notify::new()),
            dropped: Arc::new(TelemetryDropCounters::default()),
        })
    }

    pub(crate) fn config_snapshot(&self) -> &RouterTelemetryConfig {
        &self.config
    }

    pub fn register_envelope(&self) -> TelemetryRegisterEnvelope {
        let config = &self.config;
        TelemetryRegisterEnvelope {
            envelope_type: TELEMETRY_REGISTER_TYPE.to_string(),
            protocol: config.protocol.clone(),
            producer_id: config.producer_id.clone(),
            source: config.source.clone(),
            runtime_id: None,
            topics: config.topics.clone(),
        }
    }

    pub fn emit(&self, event: TelemetryEvent) -> bool {
        let config = &self.config;
        if !config.topics.contains(&event.topic) {
            self.dropped.record(&event.topic);
            return false;
        }
        let event = redact_event(event, config.string_max_chars, config.event_max_bytes);
        let enqueued = {
            let mut events = self.events.lock().expect("telemetry queue lock");
            if events.len() >= config.queue_max_events {
                if let Some(dropped) = events.pop_front() {
                    self.dropped.record(&dropped.topic);
                }
            }
            if events.len() < config.queue_max_events {
                events.push_back(event);
                true
            } else {
                self.dropped.record(&event.topic);
                false
            }
        };
        if enqueued {
            self.notify.notify_one();
        }
        enqueued
    }

    pub fn drain_batches(&self) -> Vec<TelemetryBatchEnvelope> {
        let config = &self.config;
        let mut events = {
            let mut queue = self.events.lock().expect("telemetry queue lock");
            let count = config.batch_max_events.min(queue.len());
            queue.drain(..count).collect::<Vec<_>>()
        };
        let dropped = self.dropped.take();
        if !dropped.is_empty() {
            events.insert(
                0,
                router_telemetry_event(TelemetryTopic::Log, TelemetryLevel::Warn, |event| {
                    event.name = Some("telemetry.dropped".to_string());
                    event.attrs = Some(dropped);
                }),
            );
        }
        build_batches(
            &config.producer_id,
            &self.next_seq,
            events,
            config.batch_max_events,
            config.batch_max_bytes,
        )
    }

    pub async fn notified(&self) {
        self.notify.notified().await;
    }

    pub fn queue_len(&self) -> usize {
        self.events
            .lock()
            .map(|events| events.len())
            .unwrap_or(0)
    }
}

impl TaskTelemetrySink for RouterTelemetryProducer {
    fn emit(&self, event: TelemetryEvent) -> bool {
        Self::emit(self, event)
    }
}

/// WebSocket exporter reusing the runtime producer protocol (register once,
/// then flush bounded batches on interval / queue pressure).
#[derive(Debug, Clone)]
pub struct RouterTelemetryExporter {
    endpoint: String,
    producer: RouterTelemetryProducer,
}

impl RouterTelemetryExporter {
    pub fn new(endpoint: impl Into<String>, producer: RouterTelemetryProducer) -> Self {
        Self {
            endpoint: endpoint.into(),
            producer,
        }
    }

    pub fn start(self) -> RouterTelemetryExporterHandle {
        let endpoint = self.endpoint.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(exporter_loop(endpoint, self.producer, shutdown_rx));
        RouterTelemetryExporterHandle { shutdown_tx, task }
    }
}

pub struct RouterTelemetryExporterHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl RouterTelemetryExporterHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = timeout(
            EXPORTER_SHUTDOWN_FLUSH_TIMEOUT,
            self.task,
        )
        .await;
    }
}

async fn exporter_loop(
    endpoint: String,
    producer: RouterTelemetryProducer,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_millis(250);
    loop {
        if *shutdown.borrow() {
            break;
        }
        match timeout(EXPORTER_CONNECT_TIMEOUT, connect_async(&endpoint)).await {
            Ok(Ok((ws, _))) => {
                backoff = Duration::from_millis(250);
                let (mut writer, mut reader) = ws.split();
                match send_json(&mut writer, &producer.register_envelope()).await {
                    Ok(()) => {
                        run_connected_exporter(&mut writer, &mut reader, &producer, &mut shutdown)
                            .await;
                    }
                    Err(error) => eprintln!(
                        "[router-telemetry] register send failed for {endpoint}: {error}"
                    ),
                }
            }
            Ok(Err(error)) => {
                if std::env::var("SKIFF_ROUTER_TASK_DEBUG").is_ok() {
                    eprintln!("[router-telemetry] connect failed for {endpoint}: {error}");
                }
            }
            Err(_) => {}
        }
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = shutdown.changed() => {}
            _ = sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

async fn run_connected_exporter<W, R>(
    writer: &mut W,
    reader: &mut R,
    producer: &RouterTelemetryProducer,
    shutdown: &mut watch::Receiver<bool>,
) where
    W: Sink<Message> + Unpin,
    <W as Sink<Message>>::Error: std::fmt::Display,
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut interval = tokio::time::interval(Duration::from_millis(
        producer.config_snapshot().flush_interval_ms.max(1),
    ));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = timeout(
                    EXPORTER_SHUTDOWN_FLUSH_TIMEOUT,
                    flush_pending_batches(writer, producer),
                ).await;
                break;
            }
            _ = interval.tick() => {
                if flush_pending_batches(writer, producer).await.is_err() {
                    break;
                }
            }
            _ = producer.notified() => {
                if producer.queue_len() >= producer.config_snapshot().batch_max_events
                    && flush_pending_batches(writer, producer).await.is_err()
                {
                    break;
                }
            }
            message = reader.next() => {
                match message {
                    Some(Ok(message)) if message.is_close() => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }
}

async fn flush_pending_batches<W>(
    writer: &mut W,
    producer: &RouterTelemetryProducer,
) -> Result<(), String>
where
    W: Sink<Message> + Unpin,
    <W as Sink<Message>>::Error: std::fmt::Display,
{
    for batch in producer.drain_batches() {
        send_json(writer, &batch).await?;
    }
    Ok(())
}

async fn send_json<W, T>(writer: &mut W, envelope: &T) -> Result<(), String>
where
    W: Sink<Message> + Unpin,
    <W as Sink<Message>>::Error: std::fmt::Display,
    T: serde::Serialize,
{
    let text = serde_json::to_string(envelope).map_err(|error| error.to_string())?;
    writer
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
}

/// Base router task event with Log topic and Router source.
pub fn router_telemetry_event(
    topic: TelemetryTopic,
    level: TelemetryLevel,
    configure: impl FnOnce(&mut TelemetryEvent),
) -> TelemetryEvent {
    let mut event = TelemetryEvent {
        topic,
        ts: telemetry_timestamp_now(),
        source: TelemetrySource::Router,
        visibility: TelemetryVisibility::Operational,
        service_id: None,
        revision_id: None,
        build_id: None,
        activation_identity: None,
        runtime_id: None,
        provider_id: None,
        provider_revision: None,
        provider_capability: None,
        provider_target: None,
        request_id: None,
        client_request_id: None,
        trace_id: None,
        error_id: None,
        span_id: None,
        parent_span_id: None,
        target: None,
        level: Some(level),
        name: None,
        message: None,
        attrs: None,
        error: None,
        duration_ms: None,
        dropped: None,
    };
    configure(&mut event);
    event
}

pub fn task_event(
    name: &str,
    level: TelemetryLevel,
    task_id: Option<&str>,
    attrs: Map<String, Value>,
) -> TelemetryEvent {
    let mut event = router_telemetry_event(TelemetryTopic::Log, level, |event| {
        event.name = Some(name.to_string());
    });
    let mut attrs = attrs;
    if let Some(task_id) = task_id {
        attrs.insert("taskId".to_string(), Value::String(task_id.to_string()));
    }
    event.attrs = Some(attrs);
    event
}

/// Backlog gauge event (authoritative design "Observability And Retention":
/// backlog depth, oldest eligible age and terminal age). Ages are derived
/// against the store-authority `observedAt`, never a local wall clock.
pub fn backlog_metric_event(
    observation: &skiff_task_control::store::BacklogObservation,
) -> TelemetryEvent {
    let mut attrs = Map::new();
    attrs.insert("scheduled".to_string(), json!(observation.scheduled));
    attrs.insert("ready".to_string(), json!(observation.ready));
    attrs.insert("leased".to_string(), json!(observation.leased));
    attrs.insert("terminalCount".to_string(), json!(observation.terminal_count));
    if let Some(oldest_due_at) = observation.oldest_due_at {
        attrs.insert("oldestDueAtMs".to_string(), json!(oldest_due_at.millis()));
        if let Some(observed_at) = observation.observed_at {
            attrs.insert(
                "oldestEligibleAgeMs".to_string(),
                json!((observed_at.millis() - oldest_due_at.millis()).max(0)),
            );
        }
    }
    if let Some(oldest_terminal_at) = observation.oldest_terminal_at {
        attrs.insert(
            "oldestTerminalAtMs".to_string(),
            json!(oldest_terminal_at.millis()),
        );
        if let Some(observed_at) = observation.observed_at {
            attrs.insert(
                "terminalAgeMs".to_string(),
                json!((observed_at.millis() - oldest_terminal_at.millis()).max(0)),
            );
        }
    }
    if let Some(observed_at) = observation.observed_at {
        attrs.insert("observedAtMs".to_string(), json!(observed_at.millis()));
    }
    let mut event = router_telemetry_event(TelemetryTopic::Metric, TelemetryLevel::Info, |event| {
        event.name = Some("task.backlog".to_string());
        event.attrs = Some(attrs);
    });
    event
}

pub fn telemetry_timestamp_now() -> String {
    crate::health::time::format_iso_millis(SystemTime::now())
}

fn build_batches(
    producer_id: &str,
    next_seq: &AtomicU64,
    events: Vec<TelemetryEvent>,
    max_events: usize,
    max_bytes: usize,
) -> Vec<TelemetryBatchEnvelope> {
    if events.is_empty() || max_events == 0 {
        return Vec::new();
    }
    let mut batches = Vec::new();
    let mut current = Vec::new();
    for event in events {
        let mut candidate = current.clone();
        candidate.push(event.clone());
        if !current.is_empty()
            && (candidate.len() > max_events
                || serialized_batch_size(producer_id, next_seq.load(Ordering::Relaxed), &candidate)
                    > max_bytes)
        {
            batches.push(make_batch(producer_id, next_seq, std::mem::take(&mut current)));
        }
        current.push(event);
    }
    if !current.is_empty() {
        batches.push(make_batch(producer_id, next_seq, current));
    }
    batches
}

fn make_batch(
    producer_id: &str,
    next_seq: &AtomicU64,
    events: Vec<TelemetryEvent>,
) -> TelemetryBatchEnvelope {
    let seq = next_seq.fetch_add(1, Ordering::Relaxed);
    TelemetryBatchEnvelope {
        envelope_type: TELEMETRY_BATCH_TYPE.to_string(),
        producer_id: producer_id.to_string(),
        seq,
        events,
    }
}

fn serialized_batch_size(
    producer_id: &str,
    seq: u64,
    events: &[TelemetryEvent],
) -> usize {
    serde_json::to_vec(&TelemetryBatchEnvelope {
        envelope_type: TELEMETRY_BATCH_TYPE.to_string(),
        producer_id: producer_id.to_string(),
        seq,
        events: events.to_vec(),
    })
    .map(|bytes| bytes.len())
    .unwrap_or(usize::MAX)
}

fn redact_event(
    mut event: TelemetryEvent,
    string_max_chars: usize,
    event_max_bytes: usize,
) -> TelemetryEvent {
    event.service_id = truncate_option(event.service_id, string_max_chars);
    event.revision_id = truncate_option(event.revision_id, string_max_chars);
    event.build_id = truncate_option(event.build_id, string_max_chars);
    event.activation_identity = truncate_option(event.activation_identity, string_max_chars);
    event.runtime_id = truncate_option(event.runtime_id, string_max_chars);
    event.provider_id = truncate_option(event.provider_id, string_max_chars);
    event.provider_revision = truncate_option(event.provider_revision, string_max_chars);
    event.provider_capability = truncate_option(event.provider_capability, string_max_chars);
    event.provider_target = truncate_option(event.provider_target, string_max_chars);
    event.request_id = truncate_option(event.request_id, string_max_chars);
    event.client_request_id = truncate_option(event.client_request_id, string_max_chars);
    event.trace_id = truncate_option(event.trace_id, string_max_chars);
    event.error_id = truncate_option(event.error_id, string_max_chars);
    event.span_id = truncate_option(event.span_id, string_max_chars);
    event.parent_span_id = truncate_option(event.parent_span_id, string_max_chars);
    event.target = truncate_option(event.target, string_max_chars);
    event.name = truncate_option(event.name, string_max_chars);
    event.message = truncate_option(event.message, string_max_chars);
    event.attrs = event
        .attrs
        .map(|attrs| redact_map(attrs, string_max_chars));
    event.error = event
        .error
        .map(|error| redact_map(error, string_max_chars));
    if serde_json::to_vec(&event).map(|bytes| bytes.len()).unwrap_or(usize::MAX)
        > event_max_bytes
    {
        event.attrs = Some(Map::from_iter([
            ("truncated".to_string(), Value::Bool(true)),
            (
                "originalSizeBytes".to_string(),
                json!(serde_json::to_vec(&event).map(|bytes| bytes.len()).unwrap_or(0)),
            ),
        ]));
        event.message = None;
    }
    event
}

fn truncate_option(value: Option<String>, max_chars: usize) -> Option<String> {
    value.map(|value| {
        if value.chars().count() <= max_chars {
            value
        } else {
            value.chars().take(max_chars).collect()
        }
    })
}

fn redact_map(map: Map<String, Value>, string_max_chars: usize) -> Map<String, Value> {
    map.into_iter()
        .map(|(key, value)| {
            if is_secret_key(&key) {
                (key, Value::String("[redacted]".to_string()))
            } else {
                (key, redact_value(value, string_max_chars))
            }
        })
        .collect()
}

fn redact_value(value: Value, string_max_chars: usize) -> Value {
    match value {
        Value::String(value) => Value::String(
            truncate_option(Some(value), string_max_chars).unwrap_or_default(),
        ),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(|value| redact_value(value, string_max_chars)).collect())
        }
        Value::Object(object) => Value::Object(redact_map(object, string_max_chars)),
        other => other,
    }
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ["secret", "token", "password", "authorization", "mongo", "endpoint"]
        .iter()
        .any(|needle| lower.contains(needle))
}
