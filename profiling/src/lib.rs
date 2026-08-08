//! Rust profile 采样端（`rust.profile` 契约第 1 节）。
//!
//! 跨仓库契约见 `skiff-telemetry` 仓库 `doc/rust-profile.md`。本 crate 只实现
//! 采样端：后台线程按壁钟分钟边界对齐的窗口循环，用 pprof-rs 的
//! `ITIMER_PROF`（SIGPROF，天然只采运行态）做采样，每窗口产出折叠栈
//! （根→叶子用 `;` 连接）、按线程归因的样本与进程 CPU 时间，供 runtime /
//! router 进程内的后台任务消费（`take_window` 取窗口 → 发 PlatformEvent）。
//!
//! 依赖仅 `pprof` / `libc` / `anyhow`，不引入 frame-pointer 编译要求（pprof
//! 默认用 unwind backtrace）。CPU 时间读取按平台分支：Linux 解析
//! `/proc/self/stat` 的 utime+stime（clock ticks），macOS 用
//! `getrusage(RUSAGE_SELF)`；其余平台返回 0。
//!
//! 注意：pprof-rs 的 profiler 是进程级单例（全局 SIGPROF handler），同一进程
//! 内同一时间只允许一个采样窗口；本 crate 的窗口串行执行，天然满足。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

/// 窗口最短时长：60s，导出间隔必须是它的整数倍（对齐壁钟分钟边界）。
const MINUTE_MS: u64 = 60_000;

/// 采样配置。
pub struct ProfileConfig {
    /// 是否启用采样。`false` 时 [`start`] 返回的空转句柄不采样。
    pub enabled: bool,
    /// 采样频率（Hz），默认 1000（1ms）。
    pub sampling_hz: u64,
    /// 导出窗口时长（ms），必须为 60_000 的整数倍，默认 60_000。
    pub export_interval_ms: u64,
    /// 每个窗口保留的栈条数上限（按 samples 降序截断），默认 2048。
    pub max_stacks: usize,
    /// 单条折叠栈字符串的字符数上限，超长截断（截到该长度即可，无标记），默认 1024。
    pub max_folded_chars: usize,
    /// 窗口内全部栈的估算字节总预算（每条按 `len(folded) + 32` 计，32 为 JSON
    /// 开销余量），超出部分从低 samples 侧截断，默认 196_608（192KB）。
    pub max_stacks_bytes: usize,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sampling_hz: 1000,
            export_interval_ms: MINUTE_MS,
            max_stacks: 2048,
            max_folded_chars: 1024,
            max_stacks_bytes: 196_608,
        }
    }
}

/// 单个线程的 CPU 时间样本。
pub struct ThreadSample {
    /// 线程名。
    pub name: String,
    /// 线程 CPU 时间（ms）。
    pub cpu_ms: u64,
}

/// 单个折叠栈的样本计数。
pub struct StackSample {
    /// 折叠栈（根到叶子用 `;` 连接）。
    pub folded: String,
    /// 该栈命中的样本数。
    pub samples: u64,
}

/// 一个完整采样窗口的产物。
pub struct ProfileWindow {
    /// 对齐到壁钟分钟边界的窗口起点（unix 毫秒）。
    pub interval_start_ms: i64,
    /// 窗口时长（ms），60_000。
    pub interval_ms: u64,
    /// 窗口实际墙钟时长（ms），用 `Instant` 实测。
    pub wall_ms: u64,
    /// 进程 CPU 时间（ms，utime+stime），本窗口内的增量。
    pub cpu_ms: u64,
    /// 按线程归因的 CPU 时间；无法归因时为空数组。
    pub threads: Vec<ThreadSample>,
    /// 折叠栈，按 samples 降序，截断到 `max_stacks` 条数与 `max_stacks_bytes`
    /// 字节预算（取更严格者）。
    pub stacks: Vec<StackSample>,
}

/// 采样句柄：由 [`start`] 返回，持有后台采样线程与已完成窗口的队列。
pub struct ProfileHandle {
    /// 已完成窗口的队列（按完成时间顺序 push），与后台线程共享。
    queue: Arc<Mutex<VecDeque<ProfileWindow>>>,
    /// 停止标志，后台线程在睡眠分片间检查。
    stop: Arc<AtomicBool>,
    /// 后台采样线程；`None` 表示未启用（空转句柄）。
    thread: Option<thread::JoinHandle<()>>,
}

impl ProfileHandle {
    /// 未启用时返回的空转句柄：不采样，`take_window` 恒为 `None`。
    fn idle() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            stop: Arc::new(AtomicBool::new(true)),
            thread: None,
        }
    }

    /// 取回最近完成的窗口（按完成时间顺序，最早完成的先返回）。
    pub fn take_window(&mut self) -> Option<ProfileWindow> {
        let mut queue = self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.pop_front()
    }

    /// 停止采样并等待后台线程退出。
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread {
            // 后台线程睡眠分片检查停止标志，最长一个分片后退出；
            // 若正处于窗口采样中，最多等当前窗口睡完。
            let _ = thread.join();
        }
    }
}

/// 启动后台采样线程。
///
/// - `export_interval_ms` 必须为 60_000 的整数倍，否则返回错误；
/// - `sampling_hz` 必须落在 `1..=i32::MAX`（pprof 以 `c_int` 表达频率）；
/// - `enabled: false` 时返回空转句柄（契约默认关闭采样）。
pub fn start(config: ProfileConfig) -> Result<ProfileHandle> {
    if !config.enabled {
        return Ok(ProfileHandle::idle());
    }
    if config.export_interval_ms == 0 || config.export_interval_ms % MINUTE_MS != 0 {
        bail!(
            "export_interval_ms must be a positive multiple of {MINUTE_MS}, got {}",
            config.export_interval_ms
        );
    }
    if config.sampling_hz == 0 || config.sampling_hz > i32::MAX as u64 {
        bail!(
            "sampling_hz must be within 1..={}, got {}",
            i32::MAX,
            config.sampling_hz
        );
    }

    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_queue = Arc::clone(&queue);
    let thread_stop = Arc::clone(&stop);

    let thread = thread::Builder::new()
        .name("skiff-profile".to_owned())
        .spawn(move || sampling_loop(config, thread_stop, thread_queue))
        .map_err(|err| anyhow::anyhow!("failed to spawn profiling thread: {err}"))?;

    Ok(ProfileHandle {
        queue,
        stop,
        thread: Some(thread),
    })
}

/// 后台采样主循环：每轮「睡到下一个对齐窗口起点 → 起 ProfilerGuard 采样 →
/// 睡满窗口 → 停止、算 cpu/wall、构建窗口入队」。
fn sampling_loop(config: ProfileConfig, stop: Arc<AtomicBool>, queue: Arc<Mutex<VecDeque<ProfileWindow>>>) {
    let interval_ms = config.export_interval_ms;

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // 睡到下一个对齐的窗口起点（壁钟分钟边界）。
        let now_ms = unix_now_ms();
        let next_start_ms = (now_ms / interval_ms + 1) * interval_ms;
        let wait_ms = next_start_ms.saturating_sub(now_ms);
        if !sleep_until_stop(wait_ms, &stop) {
            break;
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // 记录窗口起点 CPU 时间，然后开始采样。
        let cpu_start = process_cpu_ms();
        let window_instant = Instant::now();
        let guard = match pprof::ProfilerGuardBuilder::default()
            .frequency(config.sampling_hz as i32)
            .build()
        {
            Ok(guard) => guard,
            Err(err) => {
                // 起 guard 失败（如同一进程已有 profiler 在跑）：跳过本窗口。
                eprintln!("skiff-profiling: failed to start sampler: {err}");
                if !sleep_until_stop(MINUTE_MS, &stop) {
                    break;
                }
                continue;
            }
        };

        // 睡满整个窗口。中途收到停止信号则丢弃这个不完整窗口。
        if !sleep_until_stop(interval_ms, &stop) {
            drop(guard);
            break;
        }
        let wall_ms = window_instant.elapsed().as_millis() as u64;
        let cpu_ms = process_cpu_ms().saturating_sub(cpu_start);

        // 停止采样并产出窗口。report 需要在 guard 存活期间构建。
        let (threads, stacks) = match guard.report().build() {
            Ok(report) => collect(
                &report,
                cpu_ms,
                config.max_stacks,
                config.max_folded_chars,
                config.max_stacks_bytes,
            ),
            Err(err) => {
                eprintln!("skiff-profiling: failed to build report: {err}");
                (Vec::new(), Vec::new())
            }
        };
        drop(guard);

        queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(ProfileWindow {
                interval_start_ms: next_start_ms as i64,
                interval_ms,
                wall_ms,
                cpu_ms,
                threads,
                stacks,
            });
    }
}

/// 把 pprof report 收敛成线程归因与折叠栈（按 samples 降序）。
///
/// 线程 CPU 时间按样本比例归因进程 CPU：`cpu_ms * 线程样本数 / 总样本数`。
/// 无样本时 threads 为空（契约允许）。折叠串按根→叶子用 `;` 连接，
/// 顺序与 pprof flamegraph 实现一致（frames 与内联符号均逆序）。
///
/// 体积控制顺序：折叠串先按 `max_folded_chars` 截断（截到该长度即可，无标记），
/// 再按 samples 降序排列，最后按 `max_stacks_bytes` 预算从高 samples 侧累加截断
/// （每条估算 `len(folded) + 32`，32 为 JSON 开销余量），并同时保留 `max_stacks`
/// 条数上限（取更严格者）。
fn collect(
    report: &pprof::Report,
    cpu_ms: u64,
    max_stacks: usize,
    max_folded_chars: usize,
    max_stacks_bytes: usize,
) -> (Vec<ThreadSample>, Vec<StackSample>) {
    let mut thread_samples: HashMap<String, u64> = HashMap::new();
    let mut stack_samples: HashMap<String, u64> = HashMap::new();
    let mut total_samples = 0u64;

    for (frames, count) in &report.data {
        let count = u64::try_from(*count).unwrap_or(0);
        if count == 0 {
            continue;
        }
        total_samples += count;
        *thread_samples.entry(frames.thread_name_or_id()).or_insert(0) += count;

        let mut folded = fold(frames);
        if folded.chars().count() > max_folded_chars {
            // 折叠栈字符串超长截断（截到 max_folded_chars 即可，无标记）。
            folded = folded.chars().take(max_folded_chars).collect();
        }
        if !folded.is_empty() {
            *stack_samples.entry(folded).or_insert(0) += count;
        }
    }

    let mut threads: Vec<ThreadSample> = thread_samples
        .into_iter()
        .map(|(name, samples)| ThreadSample {
            name,
            cpu_ms: if total_samples > 0 {
                cpu_ms * samples / total_samples
            } else {
                0
            },
        })
        .collect();
    threads.sort_by(|a, b| b.cpu_ms.cmp(&a.cpu_ms).then_with(|| a.name.cmp(&b.name)));

    let mut stacks: Vec<StackSample> = stack_samples
        .into_iter()
        .map(|(folded, samples)| StackSample { folded, samples })
        .collect();
    stacks.sort_by(|a, b| b.samples.cmp(&a.samples).then_with(|| a.folded.cmp(&b.folded)));

    // 字节预算从高 samples 侧逐条累加（`len(folded) + 32`），保留预算内的栈；
    // retain 保持降序，随后仍按 max_stacks 条数上限截断（两者取更严格者）。
    let mut used_bytes = 0usize;
    stacks.retain(|stack| {
        let cost = stack.folded.len() + 32;
        if used_bytes.saturating_add(cost) > max_stacks_bytes {
            return false;
        }
        used_bytes += cost;
        true
    });
    stacks.truncate(max_stacks);

    (threads, stacks)
}

/// 把一条样本的栈帧折叠成「根;…;叶子」串；空栈（无符号）返回空串。
fn fold(frames: &pprof::Frames) -> String {
    let mut parts = Vec::new();
    for frame in frames.frames.iter().rev() {
        for symbol in frame.iter().rev() {
            parts.push(symbol.name());
        }
    }
    parts.join(";")
}

/// 分片睡眠：每片检查停止标志；返回 `false` 表示期间收到停止请求。
fn sleep_until_stop(ms: u64, stop: &AtomicBool) -> bool {
    const CHUNK_MS: u64 = 200;
    let mut remaining = ms;
    while remaining > 0 {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let chunk = remaining.min(CHUNK_MS);
        thread::sleep(Duration::from_millis(chunk));
        remaining -= chunk;
    }
    !stop.load(Ordering::Relaxed)
}

/// 当前 unix 毫秒。
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 进程 CPU 时间（ms，utime+stime）。
#[cfg(target_os = "linux")]
fn process_cpu_ms() -> u64 {
    let stat = match std::fs::read_to_string("/proc/self/stat") {
        Ok(stat) => stat,
        Err(_) => return 0,
    };
    // comm（第 2 字段）可能含空格/括号，从最后一个 ')' 之后开始数：
    // 第 3 字段是 state，utime 是第 14 字段、stime 是第 15 字段。
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return 0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let (Some(utime), Some(stime)) = (fields.get(11), fields.get(12)) else {
        return 0;
    };
    let (Ok(utime), Ok(stime)) = (utime.parse::<u64>(), stime.parse::<u64>()) else {
        return 0;
    };

    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if clk_tck <= 0 {
        return 0;
    }
    utime.saturating_add(stime).saturating_mul(1000) / clk_tck as u64
}

/// 进程 CPU 时间（ms，utime+stime），来自 `getrusage(RUSAGE_SELF)`。
#[cfg(target_os = "macos")]
fn process_cpu_ms() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return 0;
    }
    // macOS 的 time_t 是 i64，suseconds_t 是 i32。
    let utime_sec = usage.ru_utime.tv_sec;
    let utime_usec = i64::from(usage.ru_utime.tv_usec);
    let stime_sec = usage.ru_stime.tv_sec;
    let stime_usec = i64::from(usage.ru_stime.tv_usec);
    let total_ms = utime_sec
        .saturating_add(stime_sec)
        .saturating_mul(1000)
        .saturating_add(utime_usec.saturating_add(stime_usec) / 1000);
    total_ms.max(0) as u64
}

/// 其他平台暂无 CPU 时间读取实现，返回 0。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cpu_ms() -> u64 {
    0
}
