//! `rust.profile` 采样接入：driver 在启动时把 `skiff-profiling` 句柄挂进 tokio
//! 运行时，后台任务轮询 `take_window()`，把每个完成的窗口经 host telemetry
//! producer 发成 PlatformEvent（契约见 skiff-telemetry doc/rust-profile.md §1/§2）。

use std::time::Duration;

use serde_json::{json, Map, Value};
use skiff_runtime_host::host::telemetry::TelemetryProducer;
use skiff_runtime_host::telemetry::telemetry_timestamp_now;
use skiff_runtime_transport::protocol::{PlatformEvent, TelemetrySource};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::config::RuntimeProfileConfig;

/// `take_window` 轮询间隔。默认导出窗口是 60s（分钟对齐），1s 轮询把送达
/// 延迟压到窗口量级以下，同时轮询成本可忽略。
const TAKE_WINDOW_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 契约 §1 的 `max_stacks` 默认值；runtime.yml 暂不暴露该字段。
const DEFAULT_MAX_STACKS: usize = 2048;

/// 事件名（契约 §2）。
const RUST_PROFILE_EVENT_NAME: &str = "rust.profile";

/// 启动采样并把窗口发射任务挂进当前 tokio 运行时。
///
/// 任务在 `shutdown` 时经 watch 收到通知，调用 `ProfileHandle::stop()` 后退出。
pub fn start_profile_emitter(
    profile: &RuntimeProfileConfig,
    producer: TelemetryProducer,
) -> anyhow::Result<ProfileEmitterHandle> {
    let mut handle = skiff_profiling::start(skiff_profiling::ProfileConfig {
        enabled: true,
        sampling_hz: profile.sampling_hz,
        export_interval_ms: profile.export_interval_ms,
        max_stacks: DEFAULT_MAX_STACKS,
        ..Default::default()
    })?;
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TAKE_WINDOW_POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    handle.stop();
                    break;
                }
                _ = interval.tick() => {
                    while let Some(window) = handle.take_window() {
                        let attrs = rust_profile_attrs(&window);
                        let event = PlatformEvent::new(RUST_PROFILE_EVENT_NAME)
                            .with_attrs(Some(attrs))
                            .into_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
                        let _ = producer.emit(event);
                    }
                }
            }
        }
    });
    Ok(ProfileEmitterHandle { shutdown_tx, task })
}

/// 契约 §2 的事件 attrs；数字字段一律用 JSON number（非字符串）。
fn rust_profile_attrs(window: &skiff_profiling::ProfileWindow) -> Map<String, Value> {
    Map::from_iter([
        ("producer".to_string(), json!("runtime")),
        (
            "intervalStartMs".to_string(),
            json!(window.interval_start_ms),
        ),
        ("intervalMs".to_string(), json!(window.interval_ms)),
        ("wallMs".to_string(), json!(window.wall_ms)),
        ("cpuMs".to_string(), json!(window.cpu_ms)),
        (
            "threads".to_string(),
            json!(window
                .threads
                .iter()
                .map(|thread| json!({ "name": thread.name, "cpuMs": thread.cpu_ms }))
                .collect::<Vec<_>>()),
        ),
        (
            "stacks".to_string(),
            json!(window
                .stacks
                .iter()
                .map(|stack| json!({ "folded": stack.folded, "samples": stack.samples }))
                .collect::<Vec<_>>()),
        ),
        (
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
        ),
    ])
}

/// 采样发射后台任务句柄；`shutdown` 通知任务 `stop()` 采样句柄并等待退出。
pub struct ProfileEmitterHandle {
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ProfileEmitterHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.task.await;
    }
}
