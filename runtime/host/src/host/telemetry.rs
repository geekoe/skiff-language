#![allow(dead_code)]

use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};

use futures_util::{Sink, SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use tokio::{
    sync::{watch, Notify},
    task::JoinHandle,
    time::{sleep, timeout, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, warn};

use skiff_runtime_transport::protocol::{
    TelemetryBatchEnvelope, TelemetryControlConfig, TelemetryEvent, TelemetryLevel,
    TelemetryProtocol, TelemetryRegisterEnvelope, TelemetrySource,
};

use crate::telemetry::{telemetry_event, telemetry_timestamp_now, TelemetryEmitter};

pub const TELEMETRY_REGISTER_TYPE: &str = "telemetry.register";
pub const TELEMETRY_BATCH_TYPE: &str = "telemetry.batch";
pub const DEFAULT_QUEUE_MAX_EVENTS: usize = 10_000;
pub const DEFAULT_BATCH_MAX_EVENTS: usize = 200;
pub const DEFAULT_BATCH_MAX_BYTES: usize = 262_144;
pub const DEFAULT_FLUSH_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_STRING_MAX_CHARS: usize = 2048;
pub const DEFAULT_EVENT_MAX_BYTES: usize = 16 * 1024;
pub const PROFILE_EVENT_MAX_BYTES: usize = 512 * 1024;
pub const DEFAULT_FILE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_FILE_MAX_FILES: usize = 8;
pub const TELEMETRY_FILE_HEADER_TYPE: &str = "fileHeader";
pub const TELEMETRY_FILE_PROTOCOL: &str = "skiff-telemetry-v1";
const EXPORTER_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const EXPORTER_SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub producer_id: String,
    pub source: TelemetrySource,
    pub runtime_id: Option<String>,
    pub protocol: TelemetryProtocol,
    pub queue_max_events: usize,
    pub batch_max_events: usize,
    pub batch_max_bytes: usize,
    pub flush_interval_ms: u64,
    pub string_max_chars: usize,
    pub event_max_bytes: usize,
    pub file_root: PathBuf,
    pub file_path: Option<PathBuf>,
    pub file_max_bytes: u64,
    pub file_max_files: usize,
}

impl TelemetryConfig {
    pub fn from_control(
        producer_id: impl Into<String>,
        source: TelemetrySource,
        runtime_id: Option<String>,
        control: &TelemetryControlConfig,
    ) -> Self {
        Self {
            producer_id: producer_id.into(),
            source,
            runtime_id,
            protocol: control.protocol.clone(),
            queue_max_events: control.queue_max_events as usize,
            batch_max_events: control.batch_max_events as usize,
            batch_max_bytes: control.batch_max_bytes as usize,
            flush_interval_ms: control.flush_interval_ms as u64,
            string_max_chars: DEFAULT_STRING_MAX_CHARS,
            event_max_bytes: DEFAULT_EVENT_MAX_BYTES,
            file_root: PathBuf::from("."),
            file_path: None,
            file_max_bytes: DEFAULT_FILE_MAX_BYTES,
            file_max_files: DEFAULT_FILE_MAX_FILES,
        }
    }

    /// `file_root` is the default JSONL directory (`<runtime_home.parent()>/logs/telemetry`);
    /// `file_path` overrides the sink file: absolute paths are used directly, relative paths
    /// resolve against `file_root`. `None` selects `<file_root>/<producer_id>.jsonl`.
    pub fn for_runtime(
        producer_id: impl Into<String>,
        runtime_id: impl Into<String>,
        file_root: PathBuf,
        file_path: Option<PathBuf>,
        file_max_bytes: Option<u64>,
        file_max_files: Option<usize>,
    ) -> Self {
        Self {
            producer_id: producer_id.into(),
            source: TelemetrySource::Runtime,
            runtime_id: Some(runtime_id.into()),
            protocol: TelemetryProtocol::SkiffTelemetryV1,
            queue_max_events: DEFAULT_QUEUE_MAX_EVENTS,
            batch_max_events: DEFAULT_BATCH_MAX_EVENTS,
            batch_max_bytes: DEFAULT_BATCH_MAX_BYTES,
            flush_interval_ms: DEFAULT_FLUSH_INTERVAL_MS,
            string_max_chars: DEFAULT_STRING_MAX_CHARS,
            event_max_bytes: DEFAULT_EVENT_MAX_BYTES,
            file_root,
            file_path,
            file_max_bytes: file_max_bytes.unwrap_or(DEFAULT_FILE_MAX_BYTES),
            file_max_files: file_max_files.unwrap_or(DEFAULT_FILE_MAX_FILES),
        }
    }

    pub fn for_test(producer_id: impl Into<String>) -> Self {
        Self {
            producer_id: producer_id.into(),
            source: TelemetrySource::Test,
            runtime_id: Some("runtime-test-1".to_string()),
            protocol: TelemetryProtocol::SkiffTelemetryV1,
            queue_max_events: 100,
            batch_max_events: 200,
            batch_max_bytes: 262_144,
            flush_interval_ms: DEFAULT_FLUSH_INTERVAL_MS,
            string_max_chars: DEFAULT_STRING_MAX_CHARS,
            event_max_bytes: DEFAULT_EVENT_MAX_BYTES,
            file_root: PathBuf::from("."),
            file_path: None,
            file_max_bytes: DEFAULT_FILE_MAX_BYTES,
            file_max_files: DEFAULT_FILE_MAX_FILES,
        }
    }
}

/// Runtime telemetry producer configuration (driver-parsed, host-consumed).
///
/// `None` (or `enabled: false` in the driver config) disables telemetry
/// entirely (no producer sink). `endpoint: Some(non-empty)` selects the WS
/// exporter; `endpoint: None` (missing/empty) selects the default JSONL file
/// sink. `file_path` overrides the sink file: absolute paths are used
/// directly, relative paths resolve against the default
/// `<runtime_home.parent()>/logs/telemetry` root in the host.
#[derive(Debug, Clone)]
pub struct RuntimeTelemetryConfig {
    pub endpoint: Option<String>,
    pub file_path: Option<PathBuf>,
    pub file_max_bytes: Option<u64>,
    pub file_max_files: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TelemetryDropCounters {
    pub dropped: u64,
    pub queue_lock: u64,
}

impl TelemetryDropCounters {
    pub fn has_any(&self) -> bool {
        self.dropped > 0 || self.queue_lock > 0
    }

    pub fn to_dropped_map(&self) -> Map<String, Value> {
        Map::from_iter([
            ("dropped".to_string(), json!(self.dropped)),
            ("queueLock".to_string(), json!(self.queue_lock)),
        ])
    }
}

#[derive(Debug)]
struct TelemetryDropCounterAtomics {
    dropped: AtomicU64,
    queue_lock: AtomicU64,
}

impl TelemetryDropCounterAtomics {
    fn new() -> Self {
        Self {
            dropped: AtomicU64::new(0),
            queue_lock: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> TelemetryDropCounters {
        TelemetryDropCounters {
            dropped: self.dropped.load(Ordering::Relaxed),
            queue_lock: self.queue_lock.load(Ordering::Relaxed),
        }
    }

    fn take_snapshot(&self) -> TelemetryDropCounters {
        TelemetryDropCounters {
            dropped: self.dropped.swap(0, Ordering::Relaxed),
            queue_lock: self.queue_lock.swap(0, Ordering::Relaxed),
        }
    }

    fn increment_dropped(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_queue_lock(&self) {
        self.queue_lock.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryQueue {
    max_events: Arc<AtomicUsize>,
    events: Arc<Mutex<VecDeque<TelemetryEvent>>>,
    counters: Arc<TelemetryDropCounterAtomics>,
}

impl TelemetryQueue {
    pub fn new(max_events: usize) -> Self {
        Self {
            max_events: Arc::new(AtomicUsize::new(max_events)),
            events: Arc::new(Mutex::new(VecDeque::new())),
            counters: Arc::new(TelemetryDropCounterAtomics::new()),
        }
    }

    pub fn update_max_events(&self, max_events: usize) {
        self.max_events.store(max_events, Ordering::Relaxed);
    }

    pub fn enqueue(&self, event: TelemetryEvent) -> bool {
        let Ok(mut events) = self.events.try_lock() else {
            self.counters.increment_dropped();
            self.counters.increment_queue_lock();
            return false;
        };

        let max_events = self.max_events.load(Ordering::Relaxed);
        if max_events == 0 {
            self.counters.increment_dropped();
            return false;
        }

        while events.len() > max_events {
            if events.pop_front().is_some() {
                self.counters.increment_dropped();
            }
        }

        if events.len() < max_events {
            events.push_back(event);
            return true;
        }

        let incoming_priority = event_priority(&event);
        if let Some((index, _)) = events
            .iter()
            .enumerate()
            .min_by_key(|(_, queued)| event_priority(queued))
            .filter(|(_, queued)| event_priority(queued) < incoming_priority)
        {
            events.remove(index).expect("indexed event exists");
            self.counters.increment_dropped();
            events.push_back(event);
            true
        } else {
            self.counters.increment_dropped();
            false
        }
    }

    pub fn drain(&self, max_events: usize) -> Vec<TelemetryEvent> {
        let Ok(mut events) = self.events.try_lock() else {
            return Vec::new();
        };
        let count = max_events.min(events.len());
        events.drain(..count).collect()
    }

    pub fn len(&self) -> usize {
        self.events.lock().map(|events| events.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.events
            .lock()
            .map(|events| events.is_empty())
            .unwrap_or(true)
    }

    pub fn drop_counters(&self) -> TelemetryDropCounters {
        self.counters.snapshot()
    }

    pub fn take_drop_counters(&self) -> TelemetryDropCounters {
        self.counters.take_snapshot()
    }

    pub fn record_drop(&self) {
        self.counters.increment_dropped();
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryProducer {
    config: Arc<RwLock<TelemetryConfig>>,
    queue: TelemetryQueue,
    next_seq: Arc<Mutex<u64>>,
    notify: Arc<Notify>,
}

impl TelemetryProducer {
    pub fn new(config: TelemetryConfig) -> Self {
        let queue = TelemetryQueue::new(config.queue_max_events);
        Self {
            config: Arc::new(RwLock::new(config)),
            queue,
            next_seq: Arc::new(Mutex::new(1)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn update_from_control(&self, control: &TelemetryControlConfig) {
        let Ok(mut config) = self.config.write() else {
            return;
        };
        config.protocol = control.protocol.clone();
        config.queue_max_events = control.queue_max_events as usize;
        config.batch_max_events = control.batch_max_events as usize;
        config.batch_max_bytes = control.batch_max_bytes as usize;
        config.flush_interval_ms = control.flush_interval_ms as u64;
        self.queue.update_max_events(config.queue_max_events);
        self.notify.notify_waiters();
    }

    pub fn config_snapshot(&self) -> TelemetryConfig {
        self.config
            .read()
            .expect("telemetry config lock poisoned")
            .clone()
    }

    pub fn register_envelope(&self) -> TelemetryRegisterEnvelope {
        let config = self.config_snapshot();
        TelemetryRegisterEnvelope {
            envelope_type: TELEMETRY_REGISTER_TYPE.to_string(),
            protocol: config.protocol,
            producer_id: config.producer_id,
            source: config.source,
            runtime_id: config.runtime_id,
        }
    }

    pub fn emit(&self, event: TelemetryEvent) -> bool {
        let config = self.config_snapshot();
        let event = redact_event(event, config.string_max_chars, config.event_max_bytes);
        let enqueued = self.queue.enqueue(event);
        if enqueued {
            self.notify.notify_one();
        }
        enqueued
    }

    /// Best-effort producer path for synchronous observation sinks.
    ///
    /// Configuration and queue contention both drop the event immediately;
    /// this path never waits for either lock and starts no background work.
    pub fn try_emit(&self, event: TelemetryEvent) -> bool {
        let Ok(config) = self.config.try_read() else {
            return false;
        };
        let event = redact_event(event, config.string_max_chars, config.event_max_bytes);
        drop(config);
        let enqueued = self.queue.enqueue(event);
        if enqueued {
            self.notify.notify_one();
        }
        enqueued
    }

    pub fn drain_batches(&self) -> Vec<TelemetryBatchEnvelope> {
        let config = self.config_snapshot();
        let mut events = self.queue.drain(config.batch_max_events);
        let dropped = self.queue.take_drop_counters();
        if dropped.has_any() {
            let mut event = telemetry_event(telemetry_timestamp_now(), config.source.clone());
            event.runtime_id = config.runtime_id.clone();
            event.name = Some("telemetry.queue".to_string());
            event.dropped = Some(dropped.to_dropped_map());
            events.push(event);
        }
        let mut next_seq = self.next_seq.lock().expect("telemetry seq lock poisoned");
        build_batches(
            &config.producer_id,
            &mut next_seq,
            events,
            config.batch_max_events,
            config.batch_max_bytes,
        )
    }

    pub async fn notified(&self) {
        self.notify.notified().await;
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn drop_counters(&self) -> TelemetryDropCounters {
        self.queue.drop_counters()
    }
}

impl TelemetryEmitter for TelemetryProducer {
    fn emit(&self, event: TelemetryEvent) -> bool {
        Self::emit(self, event)
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryExporter {
    endpoint: String,
    producer: TelemetryProducer,
}

impl TelemetryExporter {
    pub fn new(endpoint: impl Into<String>, producer: TelemetryProducer) -> Self {
        Self {
            endpoint: endpoint.into(),
            producer,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn register_envelope(&self) -> TelemetryRegisterEnvelope {
        self.producer.register_envelope()
    }

    pub fn drain_once_to_sink(&self, sink: &TelemetryTestSink) {
        sink.record_batches(self.producer.drain_batches());
    }

    pub fn start(self) -> TelemetryExporterHandle {
        let endpoint = self.endpoint.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(exporter_loop(self.endpoint, self.producer, shutdown_rx));
        TelemetryExporterHandle {
            endpoint,
            shutdown_tx,
            task,
        }
    }
}

#[derive(Debug)]
pub struct TelemetryExporterHandle {
    endpoint: String,
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl TelemetryExporterHandle {
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn shutdown(self, wait: Duration) {
        let _ = self.shutdown_tx.send(true);
        let mut task = self.task;
        tokio::select! {
            _ = &mut task => {}
            _ = sleep(wait) => {
                task.abort();
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TelemetryFileSink {
    producer: TelemetryProducer,
}

impl TelemetryFileSink {
    pub fn new(producer: TelemetryProducer) -> Self {
        Self { producer }
    }

    /// Synchronously flushes the pending queue to the JSONL file. Exposed for
    /// tests and the shutdown flush; the background loop uses the same core.
    pub fn drain_once_to_file(&self) -> Result<(), String> {
        flush_pending_to_file(&self.producer)
    }

    pub fn start(self) -> TelemetryFileSinkHandle {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(file_sink_loop(self.producer, shutdown_rx));
        TelemetryFileSinkHandle { shutdown_tx, task }
    }
}

#[derive(Debug)]
pub struct TelemetryFileSinkHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl TelemetryFileSinkHandle {
    pub async fn shutdown(self, wait: Duration) {
        let _ = self.shutdown_tx.send(true);
        let mut task = self.task;
        tokio::select! {
            _ = &mut task => {}
            _ = sleep(wait) => {
                task.abort();
            }
        }
    }
}

async fn file_sink_loop(producer: TelemetryProducer, mut shutdown: watch::Receiver<bool>) {
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
                    eprintln!("telemetry file sink flush failed: {error}");
                }
            }
            _ = producer.notified() => {
                if producer.queue_len() >= producer.config_snapshot().batch_max_events {
                    if let Err(error) = flush_pending_to_file(&producer) {
                        eprintln!("telemetry file sink flush failed: {error}");
                    }
                }
            }
        }
    }
}

fn flush_pending_to_file(producer: &TelemetryProducer) -> Result<(), String> {
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

fn write_event_to_file(producer: &TelemetryProducer, event: &TelemetryEvent) -> Result<(), String> {
    let config = producer.config_snapshot();
    let path = file_sink_path(&config);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create telemetry dir {}: {error}", parent.display()))?;
    }
    if file_needs_rotation(&path, config.file_max_bytes) {
        rotate_files(&path, config.file_max_files)?;
        write_file_header(&path, &config)?;
    } else if !path.exists() {
        write_file_header(&path, &config)?;
    }
    let line = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open telemetry file {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("write telemetry file {}: {error}", path.display()))
}

fn file_sink_path(config: &TelemetryConfig) -> PathBuf {
    match &config.file_path {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => config.file_root.join(path),
        None => config.file_root.join(format!("{}.jsonl", config.producer_id)),
    }
}

fn file_needs_rotation(path: &Path, max_bytes: u64) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.len() >= max_bytes)
        .unwrap_or(false)
}

/// Rotates `<name>.jsonl` -> `<name>.jsonl.1` and shifts older files up
/// (`.1` -> `.2`, ...), overwriting the oldest rotated file so at most
/// `max_files` rotated files are retained.
fn rotate_files(path: &Path, max_files: usize) -> Result<(), String> {
    for index in (1..max_files).rev() {
        let source = rotated_path(path, index);
        if source.exists() {
            fs::rename(&source, rotated_path(path, index + 1))
                .map_err(|error| format!("rotate telemetry file {}: {error}", source.display()))?;
        }
    }
    fs::rename(path, rotated_path(path, 1))
        .map_err(|error| format!("rotate telemetry file {}: {error}", path.display()))
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "telemetry.jsonl".to_string());
    path.with_file_name(format!("{file_name}.{index}"))
}

fn write_file_header(path: &Path, config: &TelemetryConfig) -> Result<(), String> {
    let header = json!({
        "type": TELEMETRY_FILE_HEADER_TYPE,
        "protocol": TELEMETRY_FILE_PROTOCOL,
        "producerId": config.producer_id,
        "source": config.source,
        "createdAt": telemetry_timestamp_now(),
    });
    let line = serde_json::to_string(&header).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open telemetry file {}: {error}", path.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("write telemetry file {}: {error}", path.display()))
}

async fn exporter_loop(
    endpoint: String,
    producer: TelemetryProducer,
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
                        warn!(
                            event = "telemetry.register_send_failed",
                            endpoint = %endpoint,
                            error = %error
                        );
                    }
                }
            }
            Ok(Err(error)) => {
                debug!(
                    event = "telemetry.connect_failed",
                    endpoint = %endpoint,
                    error = %error
                );
            }
            Err(_) => {
                debug!(
                    event = "telemetry.connect_timeout",
                    endpoint = %endpoint
                );
            }
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
    producer: &TelemetryProducer,
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
                    flush_pending_batches(writer, producer)
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
    producer: &TelemetryProducer,
) -> Result<(), String>
where
    W: Sink<Message> + Unpin,
    <W as Sink<Message>>::Error: std::fmt::Display,
{
    for batch in producer.drain_batches() {
        // A send must never block the exporter forever: a stalled peer (TCP
        // backpressure from a consumer that stopped reading) would otherwise
        // freeze the whole telemetry pipeline (queue backlog, profile windows
        // minutes late). On timeout the batch is dropped and the caller breaks
        // out of the connected exporter loop, which reconnects and resumes.
        match timeout(EXPORTER_SEND_TIMEOUT, send_json(writer, &batch)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                warn!(
                    event = "telemetry.send_failed",
                    error = %error
                );
                return Err(error);
            }
            Err(_) => {
                warn!(
                    event = "telemetry.send_timed_out",
                    batch_events = batch.events.len(),
                    timeoutMs = EXPORTER_SEND_TIMEOUT.as_millis(),
                );
                return Err("telemetry send timed out; dropping batch and reconnecting".to_string());
            }
        }
    }
    Ok(())
}

/// Max wall time for a single batch send over the telemetry WebSocket. A send
/// that exceeds it is treated as a stalled connection: the batch is dropped
/// and the exporter reconnects.
const EXPORTER_SEND_TIMEOUT: Duration = Duration::from_secs(5);

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

#[derive(Debug, Default)]
pub struct TelemetryTestSink {
    batches: Mutex<Vec<TelemetryBatchEnvelope>>,
}

impl TelemetryTestSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_batches(&self, batches: Vec<TelemetryBatchEnvelope>) {
        if batches.is_empty() {
            return;
        }
        self.batches
            .lock()
            .expect("telemetry test sink lock poisoned")
            .extend(batches);
    }

    pub fn batches(&self) -> Vec<TelemetryBatchEnvelope> {
        self.batches
            .lock()
            .expect("telemetry test sink lock poisoned")
            .clone()
    }

    pub fn events(&self) -> Vec<TelemetryEvent> {
        self.batches()
            .into_iter()
            .flat_map(|batch| batch.events)
            .collect()
    }
}

pub fn build_batches(
    producer_id: &str,
    next_seq: &mut u64,
    events: Vec<TelemetryEvent>,
    max_events: usize,
    max_bytes: usize,
) -> Vec<TelemetryBatchEnvelope> {
    if events.is_empty() || max_events == 0 {
        return Vec::new();
    }

    // Incremental byte accounting: each event is serialized once instead of
    // re-serializing the whole growing candidate batch per event (the old
    // quadratic behavior made flushes with large events (rust.profile
    // windows) pathologically slow and starved the queue drain).
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_size = 0usize;
    for event in events {
        let event_size = serialized_event_size(&event).saturating_add(BATCH_EVENT_JSON_OVERHEAD);
        if !current.is_empty()
            && (current.len() >= max_events
                || current_size.saturating_add(event_size) > max_bytes)
        {
            batches.push(make_batch(
                producer_id,
                next_seq,
                std::mem::take(&mut current),
            ));
            current_size = 0;
        }
        current_size = current_size.saturating_add(event_size);
        current.push(event);
    }

    if !current.is_empty() {
        batches.push(make_batch(producer_id, next_seq, current));
    }

    batches
}

/// Per-event JSON envelope overhead allowance used by [`build_batches`] byte
/// budgeting (envelope keys, separators, per-event field names). The budget is
/// a heuristic, so an approximate constant is sufficient.
const BATCH_EVENT_JSON_OVERHEAD: usize = 256;

pub fn redact_event(
    mut event: TelemetryEvent,
    string_max_chars: usize,
    event_max_bytes: usize,
) -> TelemetryEvent {
    event.service_id = truncate_option_string(event.service_id, string_max_chars);
    event.revision_id = truncate_option_string(event.revision_id, string_max_chars);
    event.build_id = truncate_option_string(event.build_id, string_max_chars);
    event.activation_identity = truncate_option_string(event.activation_identity, string_max_chars);
    event.runtime_id = truncate_option_string(event.runtime_id, string_max_chars);
    event.provider_id = truncate_option_string(event.provider_id, string_max_chars);
    event.provider_revision = truncate_option_string(event.provider_revision, string_max_chars);
    event.provider_capability = truncate_option_string(event.provider_capability, string_max_chars);
    event.provider_target = truncate_option_string(event.provider_target, string_max_chars);
    event.request_id = truncate_option_string(event.request_id, string_max_chars);
    event.client_request_id = truncate_option_string(event.client_request_id, string_max_chars);
    event.trace_id = truncate_option_string(event.trace_id, string_max_chars);
    event.error_id = truncate_option_string(event.error_id, string_max_chars);
    event.span_id = truncate_option_string(event.span_id, string_max_chars);
    event.parent_span_id = truncate_option_string(event.parent_span_id, string_max_chars);
    event.target = truncate_option_string(event.target, string_max_chars);
    event.name = truncate_option_string(event.name, string_max_chars);
    event.message = truncate_option_string(event.message, string_max_chars);
    event.attrs = event.attrs.map(|attrs| redact_map(attrs, string_max_chars));
    event.error = event.error.map(|error| redact_map(error, string_max_chars));
    event.dropped = event
        .dropped
        .map(|dropped| redact_map(dropped, string_max_chars));

    let original_size = serialized_event_size(&event);
    let effective_max = if event.name.as_deref() == Some("rust.profile") {
        event_max_bytes.max(PROFILE_EVENT_MAX_BYTES)
    } else {
        event_max_bytes
    };
    if original_size > effective_max {
        let truncation = Map::from_iter([
            ("truncated".to_string(), Value::Bool(true)),
            ("originalSizeBytes".to_string(), json!(original_size)),
        ]);
        event.attrs = Some(truncation.clone());
        if event.error.is_some() {
            event.error = Some(truncation);
        }
        event.message = None;
    }

    event
}

fn event_priority(event: &TelemetryEvent) -> u8 {
    match event.level {
        Some(TelemetryLevel::Debug) => 0,
        Some(TelemetryLevel::Info) => 1,
        Some(TelemetryLevel::Warn | TelemetryLevel::Error) => 3,
        None => 1,
    }
}

fn make_batch(
    producer_id: &str,
    next_seq: &mut u64,
    events: Vec<TelemetryEvent>,
) -> TelemetryBatchEnvelope {
    let batch = TelemetryBatchEnvelope {
        envelope_type: TELEMETRY_BATCH_TYPE.to_string(),
        producer_id: producer_id.to_string(),
        seq: *next_seq,
        events,
    };
    *next_seq += 1;
    batch
}

fn serialized_event_size(event: &TelemetryEvent) -> usize {
    serde_json::to_vec(event)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn truncate_option_string(value: Option<String>, max_chars: usize) -> Option<String> {
    value.map(|value| truncate_string(&value, max_chars))
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
        Value::String(value) => Value::String(truncate_string(&value, string_max_chars)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_value(value, string_max_chars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(redact_map(map, string_max_chars)),
        other => other,
    }
}

fn truncate_string(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|ch| *ch != '-' && *ch != '_')
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "setcookie"
            | "token"
            | "apikey"
            | "secret"
            | "password"
            | "mongourl"
            | "connectionstring"
    )
}

#[cfg(test)]
mod tests;
