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
    PlatformEvent, TelemetryBatchEnvelope, TelemetryEvent, TelemetryProtocol,
    TelemetryRegisterEnvelope, TelemetrySource,
};

use crate::config::RouterConfig;

pub const TELEMETRY_REGISTER_TYPE: &str = "telemetry.register";
pub const TELEMETRY_BATCH_TYPE: &str = "telemetry.batch";
pub const TELEMETRY_FILE_HEADER_TYPE: &str = "fileHeader";
pub const TELEMETRY_FILE_PROTOCOL: &str = "skiff-telemetry-v1";
const DEFAULT_QUEUE_MAX_EVENTS: usize = 10_000;
const DEFAULT_BATCH_MAX_EVENTS: usize = 200;
const DEFAULT_BATCH_MAX_BYTES: usize = 262_144;
const DEFAULT_STRING_MAX_CHARS: usize = 2048;
const DEFAULT_EVENT_MAX_BYTES: usize = 16 * 1024;
const PROFILE_EVENT_MAX_BYTES: usize = 512 * 1024;
const DEFAULT_FLUSH_INTERVAL_MS: u64 = 1000;
const DEFAULT_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_FILE_MAX_FILES: usize = 8;
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
    queue_max_events: usize,
    batch_max_events: usize,
    batch_max_bytes: usize,
    flush_interval_ms: u64,
    string_max_chars: usize,
    event_max_bytes: usize,
    file_root: std::path::PathBuf,
    file_path: Option<std::path::PathBuf>,
    file_max_bytes: u64,
    file_max_files: usize,
}

impl RouterTelemetryConfig {
    fn from_router(
        config: &RouterConfig,
        telemetry: Option<&crate::config::TelemetryConfig>,
    ) -> Self {
        let profile = config.profile.as_str();
        let file_root = config
            .artifacts_path
            .parent()
            .map(|parent| parent.join("logs").join("telemetry"))
            .unwrap_or_else(|| std::path::PathBuf::from("logs").join("telemetry"));
        Self {
            producer_id: format!("router:{profile}"),
            source: TelemetrySource::Router,
            protocol: TelemetryProtocol::SkiffTelemetryV1,
            queue_max_events: usize::try_from(
                telemetry
                    .map(|t| t.queue_max_events)
                    .unwrap_or(DEFAULT_QUEUE_MAX_EVENTS as u64),
            )
            .unwrap_or(DEFAULT_QUEUE_MAX_EVENTS),
            batch_max_events: usize::try_from(
                telemetry
                    .map(|t| t.batch_max_events)
                    .unwrap_or(DEFAULT_BATCH_MAX_EVENTS as u64),
            )
            .unwrap_or(DEFAULT_BATCH_MAX_EVENTS),
            batch_max_bytes: usize::try_from(
                telemetry
                    .map(|t| t.batch_max_bytes)
                    .unwrap_or(DEFAULT_BATCH_MAX_BYTES as u64),
            )
            .unwrap_or(DEFAULT_BATCH_MAX_BYTES),
            flush_interval_ms: telemetry
                .map(|t| t.flush_interval_ms)
                .unwrap_or(DEFAULT_FLUSH_INTERVAL_MS),
            string_max_chars: DEFAULT_STRING_MAX_CHARS,
            event_max_bytes: DEFAULT_EVENT_MAX_BYTES,
            file_root,
            file_path: telemetry.and_then(|t| t.file_path.clone()),
            file_max_bytes: telemetry
                .and_then(|t| t.file_max_bytes)
                .unwrap_or(DEFAULT_FILE_MAX_BYTES),
            file_max_files: usize::try_from(
                telemetry
                    .and_then(|t| t.file_max_files)
                    .unwrap_or(DEFAULT_FILE_MAX_FILES as u64),
            )
            .unwrap_or(DEFAULT_FILE_MAX_FILES),
        }
    }
}

#[derive(Debug, Default)]
struct TelemetryDropCounters {
    dropped: AtomicU64,
    queue_lock: AtomicU64,
}

impl TelemetryDropCounters {
    fn record_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshots and resets; returns an empty map when nothing was dropped so
    /// the `telemetry.dropped` event is only injected when it carries data.
    fn take(&self) -> Map<String, Value> {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        let queue_lock = self.queue_lock.swap(0, Ordering::Relaxed);
        if dropped == 0 && queue_lock == 0 {
            return Map::new();
        }
        Map::from_iter([
            ("dropped".to_string(), json!(dropped)),
            ("queueLock".to_string(), json!(queue_lock)),
        ])
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
        if let Some(telemetry) = config.telemetry.as_ref() {
            if !telemetry.enabled {
                return None;
            }
        }
        Some(Self {
            config: Arc::new(RouterTelemetryConfig::from_router(config, config.telemetry.as_ref())),
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
        }
    }

    pub fn emit(&self, event: TelemetryEvent) -> bool {
        let config = &self.config;
        let event = redact_event(event, config.string_max_chars, config.event_max_bytes);
        let enqueued = {
            let mut events = self.events.lock().expect("telemetry queue lock");
            if events.len() >= config.queue_max_events {
                if let Some(dropped) = events.pop_front() {
                    let _ = dropped;
                    self.dropped.record_dropped();
                }
            }
            if events.len() < config.queue_max_events {
                events.push_back(event);
                true
            } else {
                self.dropped.record_dropped();
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
                PlatformEvent::new("telemetry.dropped")
                    .with_attrs(Some(dropped))
                    .into_event(telemetry_timestamp_now(), TelemetrySource::Router),
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
        self.events.lock().map(|events| events.len()).unwrap_or(0)
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
        let _ = timeout(EXPORTER_SHUTDOWN_FLUSH_TIMEOUT, self.task).await;
    }
}

/// File sink (default when `telemetry.endpoint` is absent/empty): writes one
/// `TelemetryEvent` per JSONL line (no batch wrapper), with a file header
/// rewritten on every new file (including post-rotation). Semantics mirror the
/// runtime host file sink; write failures warn on stderr and are skipped.
#[derive(Debug, Clone)]
pub struct RouterTelemetryFileSink {
    producer: RouterTelemetryProducer,
}

impl RouterTelemetryFileSink {
    pub fn new(producer: RouterTelemetryProducer) -> Self {
        Self { producer }
    }

    pub fn producer(&self) -> &RouterTelemetryProducer {
        &self.producer
    }

    /// Synchronously flushes the pending queue to the JSONL file. Exposed for
    /// tests and the shutdown flush; the background loop uses the same core.
    pub fn drain_once_to_file(&self) -> Result<(), String> {
        flush_pending_to_file(&self.producer)
    }

    pub fn start(self) -> RouterTelemetryFileSinkHandle {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(file_sink_loop(self.producer, shutdown_rx));
        RouterTelemetryFileSinkHandle { shutdown_tx, task }
    }
}

#[derive(Debug)]
pub struct RouterTelemetryFileSinkHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl RouterTelemetryFileSinkHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = timeout(EXPORTER_SHUTDOWN_FLUSH_TIMEOUT, self.task).await;
    }
}

async fn file_sink_loop(
    producer: RouterTelemetryProducer,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(
        producer.config_snapshot().flush_interval_ms.max(1),
    ));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = flush_pending_to_file(&producer);
                break;
            }
            _ = interval.tick() => {
                if let Err(error) = flush_pending_to_file(&producer) {
                    eprintln!("[router-telemetry] file sink flush failed: {error}");
                }
            }
            _ = producer.notified() => {
                if producer.queue_len() >= producer.config_snapshot().batch_max_events {
                    if let Err(error) = flush_pending_to_file(&producer) {
                        eprintln!("[router-telemetry] file sink flush failed: {error}");
                    }
                }
            }
        }
    }
}

fn flush_pending_to_file(producer: &RouterTelemetryProducer) -> Result<(), String> {
    let events: Vec<TelemetryEvent> = producer
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect();
    for event in events {
        write_event_to_file(producer, &event)?;
    }
    Ok(())
}

fn write_event_to_file(
    producer: &RouterTelemetryProducer,
    event: &TelemetryEvent,
) -> Result<(), String> {
    let config = producer.config_snapshot();
    let path = file_sink_path(config);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create telemetry dir {}: {error}", parent.display()))?;
    }
    if file_needs_rotation(&path, config.file_max_bytes) {
        rotate_files(&path, config.file_max_files)?;
        write_file_header(&path, config)?;
    } else if !path.exists() {
        write_file_header(&path, config)?;
    }
    let line = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open telemetry file {}: {error}", path.display()))?;
    std::io::Write::write_all(&mut file, line.as_bytes())
        .and_then(|_| std::io::Write::write_all(&mut file, b"\n"))
        .map_err(|error| format!("write telemetry file {}: {error}", path.display()))
}

fn file_sink_path(config: &RouterTelemetryConfig) -> std::path::PathBuf {
    match &config.file_path {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => config.file_root.join(path),
        None => config.file_root.join(format!("{}.jsonl", config.producer_id)),
    }
}

fn file_needs_rotation(path: &std::path::Path, max_bytes: u64) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.len() >= max_bytes)
        .unwrap_or(false)
}

/// Rotates `<name>.jsonl` -> `<name>.jsonl.1` and shifts older files up
/// (`.1` -> `.2`, ...), overwriting the oldest rotated file so at most
/// `max_files` rotated files are retained.
fn rotate_files(path: &std::path::Path, max_files: usize) -> Result<(), String> {
    for index in (1..max_files).rev() {
        let source = rotated_path(path, index);
        if source.exists() {
            std::fs::rename(&source, rotated_path(path, index + 1))
                .map_err(|error| format!("rotate telemetry file {}: {error}", source.display()))?;
        }
    }
    std::fs::rename(path, rotated_path(path, 1))
        .map_err(|error| format!("rotate telemetry file {}: {error}", path.display()))
}

fn rotated_path(path: &std::path::Path, index: usize) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "telemetry.jsonl".to_string());
    path.with_file_name(format!("{file_name}.{index}"))
}

fn write_file_header(path: &std::path::Path, config: &RouterTelemetryConfig) -> Result<(), String> {
    let header = json!({
        "type": TELEMETRY_FILE_HEADER_TYPE,
        "protocol": TELEMETRY_FILE_PROTOCOL,
        "producerId": config.producer_id,
        "source": config.source,
        "createdAt": telemetry_timestamp_now(),
    });
    let line = serde_json::to_string(&header).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open telemetry file {}: {error}", path.display()))?;
    std::io::Write::write_all(&mut file, line.as_bytes())
        .and_then(|_| std::io::Write::write_all(&mut file, b"\n"))
        .map_err(|error| format!("write telemetry file {}: {error}", path.display()))
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
                    Err(error) => {
                        eprintln!("[router-telemetry] register send failed for {endpoint}: {error}")
                    }
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

pub fn task_event(
    name: &str,
    task_id: Option<&str>,
    attrs: Map<String, Value>,
) -> TelemetryEvent {
    let mut attrs = attrs;
    if let Some(task_id) = task_id {
        attrs.insert("taskId".to_string(), Value::String(task_id.to_string()));
    }
    PlatformEvent::new(name)
        .with_attrs(Some(attrs))
        .into_event(telemetry_timestamp_now(), TelemetrySource::Router)
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
    attrs.insert(
        "terminalCount".to_string(),
        json!(observation.terminal_count),
    );
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
    PlatformEvent::new("task.backlog")
        .with_attrs(Some(attrs))
        .into_event(telemetry_timestamp_now(), TelemetrySource::Router)
}

pub fn telemetry_timestamp_now() -> String {
    crate::health::time::format_iso_millis(SystemTime::now())
}

/// rust.profile sampling producer wiring (rust.profile contract §2): the
/// `skiff-profiling` sampler runs on a background thread; this loop polls
/// `take_window` on the Tokio runtime and emits one PlatformEvent per
/// completed window through the task telemetry sink (no-op when telemetry is
/// disabled). Shutdown signals the loop, which drains the remaining windows
/// and calls `handle.stop()`.
pub struct ProfileSamplingHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ProfileSamplingHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        // The loop drains the pending windows and joins the sampling thread
        // (bounded: the sampler checks its stop flag at least every 200ms).
        let _ = timeout(Duration::from_secs(1), self.task).await;
    }
}

/// Starts the rust.profile sampler when the `profileSampling` config block is
/// enabled. Fail-soft: a sampler start error (e.g. bad frequency) only logs on
/// stderr and the router keeps running.
pub fn start_profile_sampling(
    config: &RouterConfig,
    sink: Arc<dyn TaskTelemetrySink>,
) -> Option<ProfileSamplingHandle> {
    let sampling = config.profile_sampling.as_ref()?;
    if !sampling.enabled {
        return None;
    }
    let profiling_config = skiff_profiling::ProfileConfig {
        enabled: true,
        sampling_hz: sampling.sampling_hz,
        export_interval_ms: sampling.export_interval_ms,
        ..skiff_profiling::ProfileConfig::default()
    };
    let handle = match skiff_profiling::start(profiling_config) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("[router-profile] failed to start sampling: {error}");
            return None;
        }
    };
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(profile_window_loop(handle, sink, shutdown_rx));
    Some(ProfileSamplingHandle { shutdown_tx, task })
}

/// Polls `take_window` on a 1s cadence and emits a PlatformEvent per window.
/// Windows land at most once per export interval; a while-loop drain keeps up
/// after process suspension or a long sampling stall.
async fn profile_window_loop(
    mut handle: skiff_profiling::ProfileHandle,
    sink: Arc<dyn TaskTelemetrySink>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = interval.tick() => {
                while let Some(window) = handle.take_window() {
                    sink.emit(profile_window_event(&window));
                }
            }
        }
    }
    // Best-effort final drain before stopping the sampler; a partial window
    // that never completed a full export interval is dropped by skiff-profiling.
    while let Some(window) = handle.take_window() {
        sink.emit(profile_window_event(&window));
    }
    handle.stop();
}

/// `rust.profile` PlatformEvent for one completed sampling window (contract
/// §2): producer is explicitly `"router"`, numeric fields are JSON numbers.
pub fn profile_window_event(window: &skiff_profiling::ProfileWindow) -> TelemetryEvent {
    let mut attrs = Map::new();
    attrs.insert("producer".to_string(), json!("router"));
    attrs.insert("intervalStartMs".to_string(), json!(window.interval_start_ms));
    attrs.insert("intervalMs".to_string(), json!(window.interval_ms));
    attrs.insert("wallMs".to_string(), json!(window.wall_ms));
    attrs.insert("cpuMs".to_string(), json!(window.cpu_ms));
    attrs.insert(
        "threads".to_string(),
        json!(window
            .threads
            .iter()
            .map(|thread| json!({ "name": thread.name, "cpuMs": thread.cpu_ms }))
            .collect::<Vec<_>>()),
    );
    attrs.insert(
        "stacks".to_string(),
        json!(window
            .stacks
            .iter()
            .map(|stack| json!({ "folded": stack.folded, "samples": stack.samples }))
            .collect::<Vec<_>>()),
    );
    attrs.insert(
        "functions".to_string(),
        json!(window
            .functions
            .iter()
            .map(|function| json!({
                "name": function.name,
                "units": function.units,
                "cpuMs": function.cpu_ms,
            }))
            .collect::<Vec<_>>()),
    );
    PlatformEvent::new("rust.profile")
        .with_attrs(Some(attrs))
        .into_event(telemetry_timestamp_now(), TelemetrySource::Router)
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
            batches.push(make_batch(
                producer_id,
                next_seq,
                std::mem::take(&mut current),
            ));
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

fn serialized_batch_size(producer_id: &str, seq: u64, events: &[TelemetryEvent]) -> usize {
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
    event.attrs = event.attrs.map(|attrs| redact_map(attrs, string_max_chars));
    event.error = event.error.map(|error| redact_map(error, string_max_chars));
    let effective_max = if event.name.as_deref() == Some("rust.profile") {
        event_max_bytes.max(PROFILE_EVENT_MAX_BYTES)
    } else {
        event_max_bytes
    };
    if serde_json::to_vec(&event)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
        > effective_max
    {
        event.attrs = Some(Map::from_iter([
            ("truncated".to_string(), Value::Bool(true)),
            (
                "originalSizeBytes".to_string(),
                json!(serde_json::to_vec(&event)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0)),
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
        Value::String(value) => {
            Value::String(truncate_option(Some(value), string_max_chars).unwrap_or_default())
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_value(value, string_max_chars))
                .collect(),
        ),
        Value::Object(object) => Value::Object(redact_map(object, string_max_chars)),
        other => other,
    }
}

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "authorization",
        "mongo",
        "endpoint",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
